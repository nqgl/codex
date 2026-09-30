//! Removes local prompt overrides without suppressing managed configuration or requirements.

use codex_config::ConfigLayerSource;
use codex_config::ConfigLayerStack;
use codex_config::config_toml::ConfigToml;
use codex_features::FeatureToml;
use codex_features::MultiAgentV2ConfigToml;
use codex_protocol::config_types::PromptMode;

pub(super) fn apply(cfg: &mut ConfigToml, stack: &ConfigLayerStack) -> std::io::Result<PromptMode> {
    let mode = cfg.prompt_mode.unwrap_or_default();
    if mode == PromptMode::Custom {
        return Ok(mode);
    }
    let layers = stack
        .layers_low_to_high()
        .filter(|layer| match &layer.name {
            ConfigLayerSource::Mdm { .. }
            | ConfigLayerSource::System { .. }
            | ConfigLayerSource::EnterpriseManaged { .. }
            | ConfigLayerSource::LegacyManagedConfigTomlFromFile { .. }
            | ConfigLayerSource::LegacyManagedConfigTomlFromMdm => true,
            ConfigLayerSource::PackagedDefaults { .. }
            | ConfigLayerSource::User { .. }
            | ConfigLayerSource::Project { .. }
            | ConfigLayerSource::SessionFlags => false,
        })
        .cloned()
        .collect();
    // Requirements are applied by the main loader after this projection. In particular,
    // a required catalog must not be cleared along with a user's local catalog override.
    let mut managed =
        ConfigLayerStack::new(layers, Default::default(), Default::default())?.effective_config();
    if let Some(table) = managed.as_table_mut() {
        table.retain(|key, _| {
            matches!(
                key,
                "instructions"
                    | "model_instructions_file"
                    | "developer_instructions"
                    | "compact_prompt"
                    | "experimental_compact_prompt_file"
                    | "model_catalog_json"
                    | "features"
            )
        });
        if let Some(features) = table
            .get_mut("features")
            .and_then(toml::Value::as_table_mut)
        {
            features.retain(|key, _| key == "multi_agent_v2");
            if let Some(hints) = features
                .get_mut("multi_agent_v2")
                .and_then(toml::Value::as_table_mut)
            {
                hints.retain(|key, _| {
                    matches!(
                        key,
                        "usage_hint_text"
                            | "root_agent_usage_hint_text"
                            | "subagent_usage_hint_text"
                            | "subagent_developer_instructions"
                            | "multi_agent_mode_hint_text"
                    )
                });
            }
        }
    }
    let managed: ConfigToml = managed.try_into().map_err(std::io::Error::other)?;
    let managed_hints = super::multi_agent_v2_toml_config(managed.features.as_ref()).cloned();
    cfg.instructions = managed.instructions;
    cfg.model_instructions_file = managed.model_instructions_file;
    cfg.developer_instructions = managed.developer_instructions;
    cfg.compact_prompt = managed.compact_prompt;
    cfg.experimental_compact_prompt_file = managed.experimental_compact_prompt_file;
    cfg.model_catalog_json = managed.model_catalog_json;
    if let Some(feature) = cfg
        .features
        .as_mut()
        .and_then(|features| features.multi_agent_v2.as_mut())
    {
        if let FeatureToml::Enabled(enabled) = feature {
            *feature = FeatureToml::Config(MultiAgentV2ConfigToml {
                enabled: Some(*enabled),
                ..Default::default()
            });
        }
        if let FeatureToml::Config(hints) = feature {
            let managed_hints = managed_hints.unwrap_or_default();
            hints.usage_hint_text = managed_hints.usage_hint_text;
            hints.root_agent_usage_hint_text = managed_hints.root_agent_usage_hint_text;
            hints.subagent_usage_hint_text = managed_hints.subagent_usage_hint_text;
            hints.subagent_developer_instructions = managed_hints.subagent_developer_instructions;
            hints.multi_agent_mode_hint_text = managed_hints.multi_agent_mode_hint_text;
        }
    }
    Ok(mode)
}
