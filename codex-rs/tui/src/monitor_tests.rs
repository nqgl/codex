use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use pretty_assertions::assert_eq;
use tokio::sync::mpsc;

use super::*;

#[test]
fn parses_monitor_lifecycle_commands() {
    let cwd = Path::new("/workspace");
    assert_eq!(
        parse_monitor_command(
            "add project-watch --cwd messages --trust -- ./watch-for-changes --json",
            cwd
        ),
        Ok(MonitorCommand::Add(MonitorRequest {
            name: "project-watch".to_string(),
            command: "./watch-for-changes --json".to_string(),
            cwd: cwd.join("messages"),
            trust: MonitorTrust::Trusted,
        }))
    );
    assert_eq!(
        parse_monitor_command("pause PROJECT-WATCH", cwd),
        Ok(MonitorCommand::Pause("project-watch".to_string()))
    );
    assert_eq!(
        parse_monitor_command("resume project-watch", cwd),
        Ok(MonitorCommand::Resume("project-watch".to_string()))
    );
    assert_eq!(
        parse_monitor_command("remove project-watch", cwd),
        Ok(MonitorCommand::Remove("project-watch".to_string()))
    );
    assert_eq!(parse_monitor_command("", cwd), Ok(MonitorCommand::List));
}

#[test]
fn command_delimiter_ignores_quoted_double_dash() {
    assert_eq!(
        parse_monitor_command(
            "add quoted -- printf '%s -- %s' left right",
            Path::new("/workspace")
        ),
        Ok(MonitorCommand::Add(MonitorRequest {
            name: "quoted".to_string(),
            command: "printf '%s -- %s' left right".to_string(),
            cwd: PathBuf::from("/workspace"),
            trust: MonitorTrust::Untrusted,
        }))
    );
}

#[test]
fn monitor_output_is_bounded_and_reports_omissions() {
    let mut batch = OutputBatch::default();
    batch.push("x".repeat(MAX_OUTPUT_BYTES + 50).as_bytes());
    assert_eq!(batch.retained.len(), MAX_OUTPUT_BYTES);
    assert_eq!(batch.omitted_bytes, 50);

    let message = MonitorNotification {
        monitor_id: 1,
        name: "large".to_string(),
        output: batch.text(),
        omitted_bytes: batch.omitted_bytes,
        trust: MonitorTrust::Untrusted,
    }
    .user_message();
    assert!(message.contains("50 additional output bytes"));
    assert!(message.contains("untrusted external input"));
}

#[test]
fn monitor_status_snapshot() {
    let statuses = vec![
        MonitorStatus {
            name: "project-watch".to_string(),
            command: "./watch-for-changes".to_string(),
            cwd: PathBuf::from("/workspace/project"),
            state: MonitorState::Running,
            trust: MonitorTrust::Trusted,
        },
        MonitorStatus {
            name: "ci".to_string(),
            command: "gh pr checks --watch".to_string(),
            cwd: PathBuf::from("/workspace/project"),
            state: MonitorState::Paused,
            trust: MonitorTrust::Untrusted,
        },
    ];
    let (message, hint) = status_message(&statuses);
    insta::assert_snapshot!(
        "monitor_status",
        format!("{message}\n\n{}", hint.expect("status hint"))
    );
}

#[test]
fn persisted_monitor_becomes_a_runtime_request() {
    assert_eq!(
        MonitorRequest::from(codex_state::ThreadMonitor {
            name: "project-watch".to_string(),
            command: "./watch-for-changes".to_string(),
            cwd: PathBuf::from("/workspace/messages"),
            trusted: true,
            running: true,
        }),
        MonitorRequest {
            name: "project-watch".to_string(),
            command: "./watch-for-changes".to_string(),
            cwd: PathBuf::from("/workspace/messages"),
            trust: MonitorTrust::Trusted,
        }
    );
}

#[tokio::test]
async fn shell_output_becomes_a_monitor_event() {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let sender = AppEventSender::new(tx);
    let temp = tempfile::tempdir().expect("temp dir");
    let monitor = MonitorHandle::start(
        MonitorRequest {
            name: "echo".to_string(),
            command: "echo monitor-output".to_string(),
            cwd: temp.path().to_path_buf(),
            trust: MonitorTrust::Trusted,
        },
        sender,
    )
    .expect("start monitor");

    let notification = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let AppEvent::MonitorOutput(notification) = rx.recv().await.expect("monitor event") {
                break notification;
            }
        }
    })
    .await
    .expect("monitor output timeout");

    assert_eq!(notification.monitor_id, monitor.id());
    assert_eq!(notification.name, "echo");
    assert_eq!(notification.output, "monitor-output");
    assert_eq!(notification.omitted_bytes, 0);
}
