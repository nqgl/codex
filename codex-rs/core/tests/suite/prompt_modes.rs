use codex_features::Feature;
use codex_protocol::config_types::PromptMode;
use codex_protocol::protocol::MultiAgentVersion;
use core_test_support::responses;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;
use std::sync::Arc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upstream_prompt_mode_preserves_managed_instructions_and_required_catalog()
-> anyhow::Result<()> {
    let home = Arc::new(tempfile::tempdir()?);
    std::fs::write(
        home.path().join("config.toml"),
        "prompt_mode = 'upstream'\ndeveloper_instructions = 'Local override.'\nmodel_catalog_json = 'missing-local-catalog.json'\n",
    )?;
    let mut catalog = codex_models_manager::bundled_models_response()?;
    let mut model = catalog
        .models
        .into_iter()
        .find(|model| model.slug == "gpt-5.5")
        .expect("catalog model");
    model
        .model_messages
        .get_or_insert_default()
        .instructions_template = Some("Administrative catalog base.".to_owned());
    catalog.models = vec![model];
    let path = home.path().join("managed-models.json");
    std::fs::write(&path, serde_json::to_vec(&catalog)?)?;
    let path_literal = toml::Value::String(path.to_string_lossy().into_owned());
    let cloud = codex_config::test_support::CloudConfigBundleFixture::enterprise_config("developer_instructions = 'Managed task instructions.'")
        .add_enterprise_requirement(format!("model_catalog_json = {path_literal}\nadditional_developer_instructions = 'Company policy is still in force.'\n"))
        .into_loader();
    let server = responses::start_mock_server().await;
    let mock = responses::mount_sse_once(
        &server,
        responses::sse(vec![responses::ev_completed("managed")]),
    )
    .await;
    let test = test_codex()
        .with_home(home)
        .with_cloud_config_bundle(cloud)
        .build_with_auto_env(&server)
        .await?;
    test.submit_turn("Inspect the policy boundary.").await?;
    let request = mock.single_request();
    assert_eq!(
        (
            request.body_json()["instructions"].clone(),
            request.body_contains_text("Managed task instructions."),
            request.body_contains_text("Company policy is still in force."),
            request.body_contains_text("Local override.")
        ),
        (
            serde_json::json!("Administrative catalog base."),
            true,
            true,
            false
        )
    );
    test.codex.shutdown_and_wait().await?;
    Ok(())
}

#[test_case::test_case(PromptMode::Custom, "prompt-model-a"; "custom_a")]
#[test_case::test_case(PromptMode::Custom, "prompt-model-b"; "custom_b")]
#[test_case::test_case(PromptMode::Upstream, "prompt-model-a"; "upstream_a")]
#[test_case::test_case(PromptMode::Upstream, "prompt-model-b"; "upstream_b")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_mode_selects_base_and_delegation_without_disabling_tools(
    mode: PromptMode,
    model: &'static str,
) -> anyhow::Result<()> {
    let home = Arc::new(tempfile::tempdir()?);
    let mode_name = mode.select("custom", "upstream");
    let base_file = home.path().join("custom-base.md");
    std::fs::write(&base_file, "Our file base.")?;
    let base_literal = toml::Value::String(base_file.to_string_lossy().into_owned());
    let catalog_override = mode.select("", "model_catalog_json = 'missing-local-catalog.json'\n");
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            "prompt_mode = '{mode_name}'\nmodel_instructions_file = {base_literal}\n{catalog_override}instructions = 'Our custom base.'\ndeveloper_instructions = 'Our developer override.'\n[features.multi_agent_v2]\nenabled = true\nmode = 'proactive'\n"
        ),
    )?;
    let server = responses::start_mock_server().await;
    let mock = responses::mount_sse_once(
        &server,
        responses::sse(vec![responses::ev_completed("done")]),
    )
    .await;
    let mut builder = test_codex()
        .with_home(home)
        .with_model(model)
        .with_config(|config| {
            config
                .features
                .enable(Feature::MultiAgentV2)
                .expect("multi-agent tools");
        });
    for model_name in ["prompt-model-a", "prompt-model-b"] {
        builder = builder.with_model_info_override(model_name, move |info| {
            info.multi_agent_version = Some(MultiAgentVersion::V2);
            let messages = info.model_messages.get_or_insert_default();
            messages.instructions_template = Some(format!("Native base for {model_name}."));
            messages.multi_agent = None;
            messages.tools = None;
        });
    }
    let test = builder
        .with_model(model)
        .build_with_auto_env(&server)
        .await?;
    test.submit_turn("Inspect the project.").await?;
    let request = mock.single_request();
    let expected_base = mode.select(
        "Our file base.".to_owned(),
        format!("Native base for {model}."),
    );
    assert_eq!(
        request.body_json()["instructions"],
        serde_json::json!(expected_base)
    );
    assert_eq!(
        (
            request.body_contains_text("Our developer override."),
            request.body_contains_text("Actively consider sub-agents for separable work"),
            request.body_contains_text("If at any point you can parallelize work"),
            request.body_contains_text("Only call this tool for a concrete, bounded subtask"),
        ),
        mode.select((true, true, false, true), (false, false, true, false))
    );
    assert!(request.body_contains_text("spawn_agent"));
    assert!(request.body_contains_text("<permissions instructions>"));
    let next_model = if model == "prompt-model-a" {
        "prompt-model-b"
    } else {
        "prompt-model-a"
    };
    let next = responses::mount_sse_once(
        &server,
        responses::sse(vec![responses::ev_completed("next")]),
    )
    .await;
    test.codex
        .update_thread_settings(codex_protocol::protocol::ThreadSettingsOverrides {
            model: Some(next_model.to_owned()),
            ..Default::default()
        })
        .await?;
    test.submit_text_turn("Continue with the other model.")
        .await?;
    let next_request = next.single_request();
    assert_eq!(
        next_request.body_json()["model"],
        serde_json::json!(next_model)
    );
    assert_eq!(
        next_request.body_contains_text(&format!("Native base for {next_model}.")),
        mode == PromptMode::Upstream
    );
    assert_eq!(
        next_request.body_contains_text("If at any point you can parallelize work"),
        mode == PromptMode::Upstream
    );
    test.codex.shutdown_and_wait().await?;
    Ok(())
}
