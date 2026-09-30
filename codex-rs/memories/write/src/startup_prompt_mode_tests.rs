use super::*;
use codex_protocol::config_types::PromptMode;
use pretty_assertions::assert_eq;

#[tokio::test]
async fn upstream_mode_reaches_the_background_consolidation_request() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    std::fs::write(
        home.path().join("config.toml"),
        "prompt_mode = 'upstream'\n",
    )?;
    let db = init_state_db(&home).await?;
    seed_stage1_output(
        db.as_ref(),
        home.path(),
        chrono::Utc::now() - chrono::Duration::hours(1),
        "A reusable decision.",
        "Evidence for the decision.",
        "decision",
    )
    .await?;
    let root = home.path().join("memories");
    seed_required_memory_artifacts(&root).await?;
    let mock = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("stock-memory"),
            ev_assistant_message("stock-memory-message", "Done."),
            ev_completed("stock-memory"),
        ]),
    )
    .await;
    let test = build_test_codex(&server, home).await?;
    trigger_memories_startup(&test).await;
    let request = wait_for_single_request(&mock).await;
    assert_eq!(
        phase2_prompt_text(&request),
        crate::prompts::build_consolidation_prompt_for_version(
            &root,
            codex_protocol::MemoryVersion::V1,
            PromptMode::Upstream
        )
    );
    wait_for_phase2_workspace_reset(db.memories(), &root).await?;
    shutdown_test_codex(&test).await?;
    Ok(())
}
