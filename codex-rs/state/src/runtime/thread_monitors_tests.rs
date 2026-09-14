use std::path::PathBuf;

use codex_protocol::ThreadId;
use pretty_assertions::assert_eq;

use super::StateRuntime;
use super::test_support::test_thread_metadata;
use super::test_support::unique_temp_dir;
use crate::ThreadMonitor;
use codex_utils_absolute_path::test_support::PathExt;

#[tokio::test]
async fn running_thread_monitors_round_trip_and_update() {
    let home = unique_temp_dir();
    let runtime = StateRuntime::init(
        crate::SqliteConfig::new_for_testing(home.as_path().abs()),
        "test-provider".to_string(),
    )
    .await
    .expect("initialize state runtime");
    let thread_id = ThreadId::new();
    runtime
        .insert_thread_if_absent(&test_thread_metadata(
            &home,
            thread_id,
            PathBuf::from("/workspace"),
        ))
        .await
        .expect("insert thread");

    let first = ThreadMonitor {
        name: "agent-comms".to_string(),
        command: "./watch codex".to_string(),
        cwd: PathBuf::from("/workspace/messages"),
        trusted: true,
        running: true,
    };
    let second = ThreadMonitor {
        name: "ci".to_string(),
        command: "gh pr checks --watch".to_string(),
        cwd: PathBuf::from("/workspace"),
        trusted: false,
        running: false,
    };
    runtime
        .upsert_thread_monitor(thread_id, &second)
        .await
        .expect("insert second monitor");
    runtime
        .upsert_thread_monitor(thread_id, &first)
        .await
        .expect("insert first monitor");
    assert_eq!(
        runtime
            .list_thread_monitors(thread_id)
            .await
            .expect("list monitors"),
        vec![first.clone(), second.clone()]
    );

    let updated = ThreadMonitor {
        command: "./watch codex --json".to_string(),
        running: false,
        ..first
    };
    runtime
        .upsert_thread_monitor(thread_id, &updated)
        .await
        .expect("update monitor");
    assert!(
        runtime
            .delete_thread_monitor(thread_id, &second.name)
            .await
            .expect("delete monitor")
    );
    assert_eq!(
        runtime
            .list_thread_monitors(thread_id)
            .await
            .expect("list updated monitors"),
        vec![updated]
    );
}
