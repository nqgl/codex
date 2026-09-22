use chrono::Utc;
use codex_protocol::protocol::SessionSource;
use pretty_assertions::assert_eq;

use super::super::test_support::make_test_app;
use super::*;
use crate::monitor::MonitorStatus;
use crate::monitor::MonitorTrust;

#[tokio::test]
async fn restore_thread_monitors_restarts_persisted_running_monitors() {
    let temp = tempfile::tempdir().expect("temporary Codex home");
    let state_db = codex_state::StateRuntime::init(
        codex_state::SqliteConfig::new_for_testing(
            codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(temp.path())
                .expect("absolute temporary path"),
        ),
        "test-provider".to_string(),
    )
    .await
    .expect("initialize state database");
    let thread_id = ThreadId::new();
    let mut metadata = codex_state::ThreadMetadataBuilder::new(
        thread_id,
        temp.path().join("rollout.jsonl"),
        Utc::now(),
        SessionSource::Cli,
    );
    metadata.cwd = temp.path().to_path_buf();
    state_db
        .insert_thread_if_absent(&metadata.build("test-provider"))
        .await
        .expect("insert thread metadata");
    state_db
        .upsert_thread_monitor(
            thread_id,
            &codex_state::ThreadMonitor {
                name: "agent-comms".to_string(),
                command: "echo restored-monitor".to_string(),
                cwd: temp.path().to_path_buf(),
                trusted: true,
                running: true,
            },
        )
        .await
        .expect("persist monitor");

    let mut app = make_test_app().await;
    app.state_db = Some(state_db.clone());
    app.primary_thread_id = Some(thread_id);
    app.restore_thread_monitors(thread_id).await;

    assert_eq!(
        app.monitors
            .iter()
            .map(MonitorHandle::status)
            .collect::<Vec<_>>(),
        vec![MonitorStatus {
            name: "agent-comms".to_string(),
            command: "echo restored-monitor".to_string(),
            cwd: temp.path().to_path_buf(),
            state: MonitorState::Running,
            trust: MonitorTrust::Trusted,
        }]
    );

    app.pause_monitors_for_fork();
    app.persist_all_monitors().await;
    assert_eq!(
        state_db
            .list_thread_monitors(thread_id)
            .await
            .expect("list paused monitors"),
        vec![codex_state::ThreadMonitor {
            name: "agent-comms".to_string(),
            command: "echo restored-monitor".to_string(),
            cwd: temp.path().to_path_buf(),
            trusted: true,
            running: false,
        }]
    );
    app.monitors.clear();
    app.restore_thread_monitors(thread_id).await;
    let statuses = app
        .monitors
        .iter()
        .map(MonitorHandle::status)
        .collect::<Vec<_>>();
    assert_eq!(
        statuses,
        vec![MonitorStatus {
            name: "agent-comms".to_string(),
            command: "echo restored-monitor".to_string(),
            cwd: temp.path().to_path_buf(),
            state: MonitorState::Paused,
            trust: MonitorTrust::Trusted,
        }]
    );
    let (message, _) = status_message(&statuses);
    insta::assert_snapshot!(
        "paused_monitor_stays_paused_after_restore",
        message.replace(&temp.path().display().to_string(), "<TEST_WORKSPACE>")
    );
}

#[tokio::test]
async fn fork_inherits_monitor_definitions_paused_without_changing_source_state() {
    let temp = tempfile::tempdir().expect("temporary Codex home");
    let state_db = codex_state::StateRuntime::init(
        codex_state::SqliteConfig::new_for_testing(
            codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(temp.path())
                .expect("absolute temporary path"),
        ),
        "test-provider".to_string(),
    )
    .await
    .expect("initialize state database");
    let source_thread_id = ThreadId::new();
    let fork_thread_id = ThreadId::new();
    for thread_id in [source_thread_id, fork_thread_id] {
        let mut metadata = codex_state::ThreadMetadataBuilder::new(
            thread_id,
            temp.path().join(format!("rollout-{thread_id}.jsonl")),
            Utc::now(),
            SessionSource::Cli,
        );
        metadata.cwd = temp.path().to_path_buf();
        state_db
            .insert_thread_if_absent(&metadata.build("test-provider"))
            .await
            .expect("insert thread metadata");
    }
    let source_monitor = codex_state::ThreadMonitor {
        name: "agent-comms".to_string(),
        command: "echo should-not-run".to_string(),
        cwd: temp.path().to_path_buf(),
        trusted: true,
        running: true,
    };
    state_db
        .upsert_thread_monitor(source_thread_id, &source_monitor)
        .await
        .expect("persist source monitor");

    let mut app = make_test_app().await;
    app.state_db = Some(state_db.clone());
    app.inherit_thread_monitors_paused(source_thread_id, fork_thread_id)
        .await;

    assert_eq!(
        app.monitors
            .iter()
            .map(MonitorHandle::status)
            .collect::<Vec<_>>(),
        vec![MonitorStatus {
            name: source_monitor.name.clone(),
            command: source_monitor.command.clone(),
            cwd: source_monitor.cwd.clone(),
            state: MonitorState::Paused,
            trust: MonitorTrust::Trusted,
        }]
    );
    assert_eq!(
        state_db
            .list_thread_monitors(source_thread_id)
            .await
            .expect("list source monitors"),
        vec![source_monitor.clone()]
    );
    assert_eq!(
        state_db
            .list_thread_monitors(fork_thread_id)
            .await
            .expect("list fork monitors"),
        vec![codex_state::ThreadMonitor {
            running: false,
            ..source_monitor
        }]
    );
}
