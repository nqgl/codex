//! Group tools and prompt are available only after the user binds a root thread.

use anyhow::Result;
use app_test_support::MockResponsesConfig;
use app_test_support::TestAppServer;
use codex_app_server_protocol::ThreadResumeParams;
use codex_app_server_protocol::ThreadResumeResponse;
use codex_app_server_protocol::ThreadStartParams;
use codex_app_server_protocol::ThreadStartResponse;
use codex_app_server_protocol::TurnCompletedNotification;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::UserInput;
use codex_group_mail_extension::GroupMailStore;
use codex_group_mail_extension::Priority;
use codex_protocol::ThreadId;
use core_test_support::responses;
use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[tokio::test]
async fn registered_root_receives_group_prompt_and_native_tools() -> Result<()> {
    let server = responses::start_mock_server().await;
    let home = TempDir::new()?;
    MockResponsesConfig::new(&server.uri()).write(home.path())?;
    let response_mock = responses::mount_sse_once(
        &server,
        responses::sse(vec![
            responses::ev_response_created("response"),
            responses::ev_assistant_message("message", "Done"),
            responses::ev_completed("response"),
        ]),
    )
    .await;
    let mut app = TestAppServer::builder()
        .with_codex_home(home.path())
        .build_initialized()
        .await?;
    let start = app
        .send_thread_start_request_with_auto_env(ThreadStartParams::default())
        .await?;
    let started: ThreadStartResponse = app.read_response(start).await?;
    let thread_id = ThreadId::from_string(&started.thread.id).map_err(anyhow::Error::msg)?;
    let store = GroupMailStore::open(home.path()).await?;
    store.join(thread_id, "research", "alice").await?;
    app.send_turn_start_request(TurnStartParams {
        thread_id: started.thread.id,
        input: vec![UserInput::Text {
            text: "Hello".to_string(),
            text_elements: Vec::new(),
        }],
        ..Default::default()
    })
    .await?;
    let _: TurnCompletedNotification = app.read_notification("turn/completed").await?;

    let request = response_mock.single_request();
    assert!(request.body_contains_text("You are alice in research. Use send_to or broadcast"));
    let tools = request.body_json()["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let names = tools
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names
            .iter()
            .filter(|name| matches!(**name, "send_to" | "broadcast"))
            .count(),
        2
    );
    Ok(())
}

#[tokio::test]
async fn resume_delivers_older_low_mail_with_later_high_mail() -> Result<()> {
    let server = responses::start_mock_server().await;
    let home = TempDir::new()?;
    MockResponsesConfig::new(&server.uri()).write(home.path())?;
    let completed_response = |id: &str| {
        responses::sse(vec![
            responses::ev_response_created(id),
            responses::ev_assistant_message(&format!("{id}-message"), "Done"),
            responses::ev_completed(id),
        ])
    };
    let mock = responses::mount_sse_sequence(
        &server,
        vec![completed_response("initial"), completed_response("mail")],
    )
    .await;
    let mut app = TestAppServer::builder()
        .with_codex_home(home.path())
        .build_initialized()
        .await?;
    let alice_request = app
        .send_thread_start_request_with_auto_env(ThreadStartParams::default())
        .await?;
    let alice: ThreadStartResponse = app.read_response(alice_request).await?;
    let bob_request = app
        .send_thread_start_request_with_auto_env(ThreadStartParams::default())
        .await?;
    let bob: ThreadStartResponse = app.read_response(bob_request).await?;
    let alice_id = ThreadId::from_string(&alice.thread.id).map_err(anyhow::Error::msg)?;
    let bob_id = ThreadId::from_string(&bob.thread.id).map_err(anyhow::Error::msg)?;
    let store = GroupMailStore::open(home.path()).await?;
    store.join(alice_id, "research", "alice").await?;
    store.join(bob_id, "research", "bob").await?;

    app.send_turn_start_request(TurnStartParams {
        thread_id: bob.thread.id.clone(),
        input: vec![UserInput::Text {
            text: "Ready".to_string(),
            text_elements: Vec::new(),
        }],
        ..Default::default()
    })
    .await?;
    let _: TurnCompletedNotification = app.read_notification("turn/completed").await?;
    app.shutdown_gracefully().await?;
    assert!(
        store
            .members(alice_id)
            .await?
            .iter()
            .any(|peer| peer.name == "bob" && !peer.online)
    );

    store
        .send(alice_id, Some(&["bob".into()]), "first", Priority::Low)
        .await?;
    store
        .send(alice_id, Some(&["bob".into()]), "second", Priority::High)
        .await?;
    let mut resumed_app = TestAppServer::builder()
        .with_codex_home(home.path())
        .build_initialized()
        .await?;
    let resume = resumed_app
        .send_thread_resume_request(ThreadResumeParams {
            thread_id: bob.thread.id,
            ..Default::default()
        })
        .await?;
    let _: ThreadResumeResponse = resumed_app.read_response(resume).await?;
    assert!(
        store
            .members(alice_id)
            .await?
            .iter()
            .any(|peer| peer.name == "bob" && peer.online)
    );
    let _: TurnCompletedNotification = resumed_app.read_notification("turn/completed").await?;
    assert!(
        store
            .members(alice_id)
            .await?
            .iter()
            .any(|peer| peer.name == "bob" && peer.online)
    );
    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    let body = requests[1].body_json().to_string();
    let first = body
        .find("From alice to bob:\\nfirst")
        .expect("first message");
    let second = body
        .find("From alice to bob:\\nsecond")
        .expect("second message");
    assert!(first < second);
    assert!(!body.contains("Priority:"));
    Ok(())
}
