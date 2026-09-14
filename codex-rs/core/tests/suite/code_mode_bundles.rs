#![allow(clippy::unwrap_used)]

use anyhow::Result;
use codex_features::Feature;
use core_test_support::responses;
use core_test_support::responses::ResponsesRequest;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_custom_tool_call;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::sse;
use core_test_support::skip_if_no_network;
use core_test_support::test_codex::test_codex;
use pretty_assertions::assert_eq;
use serde_json::Value;

fn custom_tool_output_items(req: &ResponsesRequest, call_id: &str) -> Vec<Value> {
    match req.custom_tool_call_output(call_id).get("output") {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::String(text)) => {
            vec![serde_json::json!({ "type": "input_text", "text": text })]
        }
        _ => panic!("custom tool output should be serialized as text or content items"),
    }
}

fn text_item(items: &[Value], index: usize) -> &str {
    items[index]
        .get("text")
        .and_then(Value::as_str)
        .expect("content item should be input_text")
}

fn tool_names(body: &Value) -> Vec<String> {
    body.get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    tool.get("name")
                        .or_else(|| tool.get("type"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn code_mode_truncated_output_can_be_recovered_from_automatic_bundle() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("resp-1"),
                ev_custom_tool_call(
                    "call-1",
                    "exec",
                    r#"// @exec: {"max_output_tokens": 5}
text("0123456789012345678901234567890123456789");
"#,
                ),
                ev_completed("resp-1"),
            ]),
            sse(vec![
                ev_response_created("resp-2"),
                ev_custom_tool_call(
                    "call-2",
                    "exec",
                    r#"
const trunc = bundles.open("bundle_1").select("text-1").omitted();
store("trunc", trunc);
const recovered = await load("trunc").read({
  startChar: 10,
  charCount: 20,
});
text(JSON.stringify({id: trunc.id, recovered}));
"#,
                ),
                ev_completed("resp-2"),
            ]),
            sse(vec![
                ev_assistant_message("msg-1", "done"),
                ev_completed("resp-3"),
            ]),
        ],
    )
    .await;
    let builder = test_codex().with_model("test-gpt-5.1-codex");
    let test = builder
        .with_config(|config| {
            config
                .features
                .enable(Feature::CodeMode)
                .expect("code mode should be enabled");
        })
        .build(&server)
        .await?;

    test.submit_turn("recover truncated evidence").await?;

    let requests = response_mock.requests();
    let first_items = custom_tool_output_items(&requests[1], "call-1");
    assert_eq!(first_items.len(), 3);
    assert_eq!(
        text_item(&first_items, /*index*/ 2),
        concat!(
            "\n[Truncated output saved as queryable bundle `bundle_1`: ",
            "40 chars total, 20 chars omitted; ",
            "text-1=40 chars (20 omitted at C10-C30). ",
            "Use `const trunc = bundles.open(\"bundle_1\").omitted();` ",
            "to inspect what was cut.]"
        )
    );

    let second_items = custom_tool_output_items(&requests[2], "call-2");
    let recovered: Value = serde_json::from_str(text_item(&second_items, /*index*/ 1))?;
    assert_eq!(
        recovered,
        serde_json::json!({
            "id": "bundle_1",
            "recovered": {
                "item": "text-1",
                "view": "omitted",
                "coordinateUnit": "Unicode scalar values",
                "addressing": {
                    "mode": "characters",
                    "startChar": 10,
                    "charCount": 20,
                },
                "requestedRange": {"startChar": 10, "endChar": 30},
                "returnedRange": {"startChar": 10, "endChar": 30},
                "capped": false,
                "continuation": null,
                "originalChars": 40,
                "blocks": [{
                    "startChar": 10,
                    "endChar": 30,
                    "startLine": 1,
                    "endLine": 1,
                    "text": "01234567890123456789",
                }],
            },
        })
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn code_mode_bundle_ask_uses_read_only_terra_medium_query() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("resp-1"),
                ev_custom_tool_call(
                    "call-1",
                    "exec",
                    r#"
const bundle = await bundles.create({report: "alpha is the decisive evidence"});
const result = await bundle.ask("What is the decisive evidence?");
text(JSON.stringify(result));
"#,
                ),
                ev_completed("resp-1"),
            ]),
            sse(vec![
                ev_response_created("resp-query"),
                ev_assistant_message("msg-query", "Alpha is decisive [report:L1-L1]."),
                responses::ev_completed_with_tokens("resp-query", 42),
            ]),
            sse(vec![
                ev_assistant_message("msg-final", "done"),
                ev_completed("resp-2"),
            ]),
        ],
    )
    .await;
    let builder = test_codex().with_model("test-gpt-5.1-codex");
    let test = builder
        .with_config(|config| {
            config
                .features
                .enable(Feature::CodeMode)
                .expect("code mode should be enabled");
        })
        .build(&server)
        .await?;

    test.submit_turn("ask the evidence bundle").await?;

    let requests = response_mock.requests();
    let query_request = &requests[1];
    assert_eq!(query_request.body_json()["model"], "gpt-5.6-terra");
    assert_eq!(query_request.body_json()["reasoning"]["effort"], "medium");
    assert_eq!(tool_names(&query_request.body_json()), Vec::<String>::new());
    assert!(
        query_request
            .instructions_text()
            .contains("read-only evidence analyst")
    );
    assert!(
        query_request
            .instructions_text()
            .contains("complete selected bundle contents")
    );
    assert_eq!(
        query_request.message_input_texts("user").len(),
        1,
        "bundle query should receive one bounded capsule, not parent history"
    );
    let turn_metadata: Value = serde_json::from_str(
        query_request.body_json()["client_metadata"]["x-codex-turn-metadata"]
            .as_str()
            .expect("query request should include turn metadata"),
    )?;
    assert_eq!(turn_metadata["request_kind"], "bundle_query");
    assert!(
        query_request.body_contains_text("alpha is the decisive evidence"),
        "small bundle queries should receive all selected evidence inline"
    );

    let result_items = custom_tool_output_items(&requests[2], "call-1");
    let mut result: Value = serde_json::from_str(text_item(&result_items, /*index*/ 1))?;
    assert!(
        result["freshness"]["ageSeconds"].is_u64(),
        "bundle query should report snapshot age"
    );
    result["freshness"]
        .as_object_mut()
        .expect("freshness should be an object")
        .remove("ageSeconds");
    assert_eq!(
        result,
        serde_json::json!({
            "answer": "Alpha is decisive [report:L1-L1].",
            "model": "gpt-5.6-terra",
            "reasoningEffort": "medium",
            "usage": {
                "input_tokens": 42,
                "cached_input_tokens": 0,
                "cache_write_input_tokens": 0,
                "output_tokens": 0,
                "reasoning_output_tokens": 0,
                "total_tokens": 42,
            },
            "sourceBundle": {
                "id": "bundle_1",
                "selected": [],
                "view": "full",
            },
            "freshness": {
                "status": "snapshot",
                "sourceStatus": "unknown",
                "warning": "This is an immutable snapshot. Source freshness is not guaranteed; rerun the source tool when current state matters.",
            },
        })
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn code_mode_bundle_query_can_retrieve_evidence_over_multiple_rounds() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("resp-1"),
                ev_custom_tool_call(
                    "call-1",
                    "exec",
                    r#"
const bundle = await bundles.create({
  report: "x".repeat(32_000) + " the decisive needle is here"
});
const result = await bundle.ask("Locate the needle.");
text(result.answer);
"#,
                ),
                ev_completed("resp-1"),
            ]),
            sse(vec![
                ev_response_created("resp-query-1"),
                responses::ev_function_call(
                    "query-call",
                    "bundle_search",
                    r#"{"query":"needle","limit":5}"#,
                ),
                ev_completed("resp-query-1"),
            ]),
            sse(vec![
                ev_response_created("resp-query-2"),
                ev_assistant_message("msg-query", "Found it [report:C13-C19]."),
                ev_completed("resp-query-2"),
            ]),
            sse(vec![
                ev_assistant_message("msg-final", "done"),
                ev_completed("resp-2"),
            ]),
        ],
    )
    .await;
    let builder = test_codex().with_model("test-gpt-5.1-codex");
    let test = builder
        .with_config(|config| {
            config
                .features
                .enable(Feature::CodeMode)
                .expect("code mode should be enabled");
        })
        .build(&server)
        .await?;

    test.submit_turn("query a bundle through retrieval").await?;

    let requests = response_mock.requests();
    assert_eq!(requests[1].body_json()["model"], "gpt-5.6-terra");
    assert_eq!(requests[2].body_json()["model"], "gpt-5.6-terra");
    assert_eq!(
        tool_names(&requests[1].body_json()),
        vec![
            "bundle_info".to_string(),
            "bundle_read".to_string(),
            "bundle_search".to_string(),
        ]
    );
    assert!(
        requests[2]
            .function_call_output("query-call")
            .to_string()
            .contains("decisive needle"),
        "second query round should receive local bundle-search evidence"
    );
    assert_eq!(
        text_item(
            &custom_tool_output_items(&requests[3], "call-1"),
            /*index*/ 1
        ),
        "Found it [report:C13-C19]."
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn code_mode_bundle_query_reserves_final_round_for_synthesis() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let mut sequence = vec![sse(vec![
        ev_response_created("resp-1"),
        ev_custom_tool_call(
            "call-1",
            "exec",
            r#"
const bundle = await bundles.create({
  report: "x".repeat(32_000) + " alpha is still decisive"
});
const result = await bundle.ask("What is decisive?");
text(result.answer);
"#,
        ),
        ev_completed("resp-1"),
    ])];
    for round in 1..=5 {
        sequence.push(sse(vec![
            ev_response_created(&format!("resp-query-{round}")),
            responses::ev_function_call(&format!("query-call-{round}"), "bundle_info", "{}"),
            ev_completed(&format!("resp-query-{round}")),
        ]));
    }
    sequence.extend([
        sse(vec![
            ev_response_created("resp-query-final"),
            ev_assistant_message("msg-query-final", "Alpha is decisive [report:C0-C24]."),
            responses::ev_completed_with_tokens("resp-query-final", 60),
        ]),
        sse(vec![
            ev_assistant_message("msg-final", "done"),
            ev_completed("resp-2"),
        ]),
    ]);
    let response_mock = responses::mount_sse_sequence(&server, sequence).await;
    let test = test_codex()
        .with_model("test-gpt-5.1-codex")
        .with_config(|config| {
            config
                .features
                .enable(Feature::CodeMode)
                .expect("code mode should be enabled");
        })
        .build(&server)
        .await?;

    test.submit_turn("query until the synthesis round").await?;

    let requests = response_mock.requests();
    assert_eq!(tool_names(&requests[6].body_json()), Vec::<String>::new());
    assert!(
        requests[6]
            .instructions_text()
            .contains("final synthesis round")
    );
    assert_eq!(
        text_item(
            &custom_tool_output_items(&requests[7], "call-1"),
            /*index*/ 1
        ),
        "Alpha is decisive [report:C0-C24]."
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_mode_bundle_each_query_returns_another_bundle() -> Result<()> {
    skip_if_no_network!(Ok(()));

    let server = responses::start_mock_server().await;
    let response_mock = responses::mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("resp-1"),
                ev_custom_tool_call(
                    "call-1",
                    "exec",
                    r#"
const source = await bundles.create({one: "alpha", two: "beta"});
const result = await source.each().summarize("one sentence");
const info = await result.info();
text(JSON.stringify({
  id: result.id,
  origin: info.origin,
  names: info.items.map((item) => item.name),
  operation: info.operation.operation,
  sourceId: info.operation.sourceBundle.id,
  inputTokens: info.operation.usage.input_tokens,
  failedItems: info.operation.failedItems,
}));
"#,
                ),
                ev_completed("resp-1"),
            ]),
            sse(vec![
                ev_response_created("resp-query-1"),
                ev_assistant_message("msg-query-1", "first summary"),
                responses::ev_completed_with_tokens("resp-query-1", 10),
            ]),
            sse(vec![
                ev_response_created("resp-query-2"),
                ev_assistant_message("msg-query-2", "second summary"),
                responses::ev_completed_with_tokens("resp-query-2", 20),
            ]),
            sse(vec![
                ev_assistant_message("msg-final", "done"),
                ev_completed("resp-2"),
            ]),
        ],
    )
    .await;
    let builder = test_codex().with_model("test-gpt-5.1-codex");
    let test = builder
        .with_config(|config| {
            config
                .features
                .enable(Feature::CodeMode)
                .expect("code mode should be enabled");
        })
        .build(&server)
        .await?;

    test.submit_turn("summarize each bundle item").await?;

    let requests = response_mock.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.body_json()["model"] == "gpt-5.6-terra")
            .count(),
        2
    );
    let result_items = custom_tool_output_items(&requests[3], "call-1");
    let result: Value = serde_json::from_str(text_item(&result_items, /*index*/ 1))?;
    assert_eq!(
        result,
        serde_json::json!({
            "id": "bundle_2",
            "origin": "query_results",
            "names": ["one", "two"],
            "operation": "summarize",
            "sourceId": "bundle_1",
            "inputTokens": 30,
            "failedItems": [],
        })
    );

    Ok(())
}
