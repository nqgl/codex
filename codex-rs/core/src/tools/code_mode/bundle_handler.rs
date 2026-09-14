use codex_code_mode::BUNDLE_TOOL_NAME;
use codex_code_mode::BundleReference;
use codex_code_mode::bundle_items_from_json;
use serde_json::Value as JsonValue;
use tokio_util::sync::CancellationToken;

use super::ExecContext;
use crate::function_tool::FunctionCallError;

pub(super) fn is_bundle_tool_name(tool_name: &codex_tools::ToolName) -> bool {
    tool_name.namespace.is_none() && tool_name.name == BUNDLE_TOOL_NAME
}

pub(super) async fn handle_bundle_call(
    exec: &ExecContext,
    input: Option<JsonValue>,
    cancellation_token: CancellationToken,
) -> Result<JsonValue, FunctionCallError> {
    if cancellation_token.is_cancelled() {
        return Err(FunctionCallError::RespondToModel(
            "bundle operation cancelled".to_string(),
        ));
    }
    let input = input
        .and_then(|input| input.as_object().cloned())
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "bundle operation expects an object argument".to_string(),
            )
        })?;
    let operation = input.get("op").and_then(JsonValue::as_str).ok_or_else(|| {
        FunctionCallError::RespondToModel("bundle operation requires an `op` string".to_string())
    })?;
    let bundle_id = input
        .get("bundle")
        .and_then(JsonValue::as_object)
        .and_then(|bundle| bundle.get("id"))
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    let each = input
        .get("each")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    tracing::info!(
        target: "codex_core::queryable_bundles",
        event = "bundle_operation",
        operation,
        bundle_id,
        each,
    );

    match operation {
        "create" => {
            let value = input.get("value").cloned().ok_or_else(|| {
                FunctionCallError::RespondToModel(
                    "bundles.create requires a serializable value".to_string(),
                )
            })?;
            let items = bundle_items_from_json(value).map_err(FunctionCallError::RespondToModel)?;
            let reference = exec
                .session
                .services
                .code_mode_service
                .insert_bundle(
                    codex_code_mode::BundleOrigin::Manual,
                    items,
                    /*context*/ None,
                    /*operation_metadata*/ None,
                )
                .await
                .map_err(FunctionCallError::RespondToModel)?;
            Ok(reference.to_marker_json())
        }
        "info" => {
            let snapshot = snapshot(exec, &input).await?;
            Ok(snapshot.info_json())
        }
        "read" => {
            let snapshot = snapshot(exec, &input).await?;
            snapshot
                .read_json(&JsonValue::Object(input))
                .map_err(FunctionCallError::RespondToModel)
        }
        "search" => {
            let snapshot = snapshot(exec, &input).await?;
            let query = input
                .get("query")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "bundle.search requires a query string".to_string(),
                    )
                })?;
            let limit = input
                .get("limit")
                .and_then(JsonValue::as_u64)
                .and_then(|limit| usize::try_from(limit).ok())
                .unwrap_or(20);
            snapshot
                .search_json(query, limit)
                .map_err(FunctionCallError::RespondToModel)
        }
        "ask" | "summarize" => {
            super::bundle_query::run(exec, operation, &input, cancellation_token).await
        }
        operation => Err(FunctionCallError::RespondToModel(format!(
            "unknown bundle operation `{operation}`"
        ))),
    }
}

async fn snapshot(
    exec: &ExecContext,
    input: &serde_json::Map<String, JsonValue>,
) -> Result<codex_code_mode::BundleSnapshot, FunctionCallError> {
    let reference = input
        .get("bundle")
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "bundle operation requires a bundle reference".to_string(),
            )
        })
        .and_then(|reference| {
            BundleReference::from_json(reference).map_err(FunctionCallError::RespondToModel)
        })?;
    exec.session
        .services
        .code_mode_service
        .bundle_snapshot(&reference)
        .await
        .map_err(FunctionCallError::RespondToModel)
}
