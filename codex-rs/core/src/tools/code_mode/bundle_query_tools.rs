use std::collections::BTreeMap;

use codex_code_mode::BundleSnapshot;
use codex_tools::AdditionalProperties;
use codex_tools::JsonSchema;
use codex_tools::ResponsesApiTool;
use codex_tools::ToolSpec;
use serde_json::Value as JsonValue;

pub(super) fn execute_bundle_tool(
    snapshot: &BundleSnapshot,
    name: &str,
    arguments: &str,
) -> String {
    let arguments = match serde_json::from_str::<JsonValue>(arguments) {
        Ok(arguments) => arguments,
        Err(error) => return format!("Invalid bundle tool arguments: {error}"),
    };
    let result = match name {
        "bundle_info" => Ok(snapshot.info_json()),
        "bundle_read" => snapshot.read_json(&arguments),
        "bundle_search" => {
            let query = arguments
                .get("query")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "bundle_search requires a query string".to_string());
            query.and_then(|query| {
                let limit = arguments
                    .get("limit")
                    .and_then(JsonValue::as_u64)
                    .and_then(|limit| usize::try_from(limit).ok())
                    .unwrap_or(20);
                snapshot.search_json(query, limit)
            })
        }
        _ => Err(format!("Unknown bundle tool `{name}`")),
    };
    match result {
        Ok(result) => serde_json::to_string(&result)
            .unwrap_or_else(|error| format!("Failed to serialize bundle result: {error}")),
        Err(error) => error,
    }
}

pub(super) fn bundle_query_tools() -> Vec<ToolSpec> {
    let integer = |description: &str| JsonSchema::integer(Some(description.to_string()));
    let read_properties = BTreeMap::from([
        (
            "item".to_string(),
            JsonSchema::string(Some("Exact bundle item name.".to_string())),
        ),
        (
            "startLine".to_string(),
            integer("One-based original line number."),
        ),
        ("lineCount".to_string(), integer("Number of lines to read.")),
        (
            "startChar".to_string(),
            integer("Zero-based original character offset."),
        ),
        (
            "charCount".to_string(),
            integer("Number of characters to read."),
        ),
        (
            "chunk".to_string(),
            integer("One-based 4096-character virtual chunk."),
        ),
        (
            "chunkCount".to_string(),
            integer("Number of virtual chunks to read."),
        ),
    ]);
    vec![
        function_tool(
            "bundle_info",
            "Return item names, exact sizes, omitted ranges, and freshness metadata.",
            JsonSchema::object(BTreeMap::new(), Some(Vec::new()), Some(false.into())),
        ),
        function_tool(
            "bundle_read",
            "Read one item using original line, character, or virtual-chunk coordinates. Use only one addressing mode.",
            JsonSchema::object(
                read_properties,
                Some(vec!["item".to_string()]),
                Some(false.into()),
            ),
        ),
        function_tool(
            "bundle_search",
            "Search selected bundle material and return matches with original coordinates.",
            JsonSchema::object(
                BTreeMap::from([
                    (
                        "query".to_string(),
                        JsonSchema::string(Some("Literal search text.".to_string())),
                    ),
                    (
                        "limit".to_string(),
                        JsonSchema::integer(Some("Maximum matches.".to_string())),
                    ),
                ]),
                Some(vec!["query".to_string()]),
                Some(AdditionalProperties::Boolean(false)),
            ),
        ),
    ]
}

fn function_tool(name: &str, description: &str, parameters: JsonSchema) -> ToolSpec {
    ToolSpec::Function(ResponsesApiTool {
        name: name.to_string(),
        description: description.to_string(),
        strict: false,
        defer_loading: None,
        parameters,
        output_schema: None,
    })
}
