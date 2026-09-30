use super::*;
use codex_protocol::config_types::PromptMode;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn prompt_mode_is_forwarded_for_embedded_and_daemon_thread_lifecycles()
-> color_eyre::Result<()> {
    let home = tempfile::tempdir()?;
    let mut config = crate::legacy_core::config::ConfigBuilder::default()
        .codex_home(home.path().to_path_buf())
        .loader_overrides(codex_config::LoaderOverrides::without_managed_config_for_tests())
        .build()
        .await?;
    for mode in [PromptMode::Custom, PromptMode::Upstream] {
        config.prompt_mode = mode;
        for connection in [ThreadParamsMode::Embedded, ThreadParamsMode::Remote] {
            let thread = ThreadId::new();
            let start = thread_start_params_from_config(
                &config, connection, /*remote_cwd_override*/ None,
                /*session_start_source*/ None,
            );
            let resume = thread_resume_params_from_config(
                config.clone(),
                thread,
                connection,
                /*remote_cwd_override*/ None,
                ResumeModelSettings::OverrideFromCurrentConfig,
            );
            let fork = thread_fork_params_from_config(
                config.clone(),
                thread,
                connection,
                /*remote_cwd_override*/ None,
            );
            let choices = [start.config, resume.config, fork.config].map(|overrides| {
                overrides
                    .expect("per-session overrides")
                    .get("prompt_mode")
                    .cloned()
            });
            assert_eq!(
                choices,
                [
                    Some(serde_json::json!(mode)),
                    Some(serde_json::json!(mode)),
                    Some(serde_json::json!(mode))
                ]
            );
        }
    }
    Ok(())
}
