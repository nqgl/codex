use pretty_assertions::assert_eq;
use serde_json::json;

use super::*;

#[test]
fn omitted_view_preserves_original_coordinates() {
    let text = "seen0\nhidden-one\nseen1\nhidden-two\nseen2";
    let mut store = QueryableBundleStore::default();
    let reference = store
        .insert(
            BundleOrigin::TruncatedExecOutput,
            vec![
                BundleItemInput::new("document", text).with_omitted_ranges(vec![
                    BundleRange::new(/*start_char*/ 6, /*end_char*/ 16),
                    BundleRange::new(/*start_char*/ 23, /*end_char*/ 33),
                ]),
                BundleItemInput::new("fully-visible", "not omitted"),
            ],
            /*context*/ None,
            /*operation_metadata*/ None,
        )
        .expect("bundle should be inserted");
    let snapshot = store
        .snapshot(&BundleReference {
            view: BundleView::Omitted,
            ..reference
        })
        .expect("omitted view should be available");

    assert_eq!(snapshot.item_names(), vec!["document".to_string()]);
    assert_eq!(
        snapshot
            .read_json(&json!({"item": "document", "startLine": 1, "lineCount": 10}))
            .expect("omitted ranges should be readable"),
        json!({
            "item": "document",
            "view": "omitted",
            "coordinateUnit": "Unicode scalar values",
            "addressing": {"mode": "lines", "startLine": 1, "lineCount": 10},
            "requestedRange": {"startChar": 0, "endChar": 39},
            "returnedRange": {"startChar": 0, "endChar": 39},
            "capped": false,
            "continuation": null,
            "originalChars": 39,
            "blocks": [
                {
                    "startChar": 6,
                    "endChar": 16,
                    "startLine": 2,
                    "endLine": 2,
                    "text": "hidden-one",
                },
                {
                    "startChar": 23,
                    "endChar": 33,
                    "startLine": 4,
                    "endLine": 4,
                    "text": "hidden-two",
                },
            ],
        })
    );
    assert_eq!(
        snapshot
            .search_json("hidden", /*limit*/ 20)
            .expect("omitted ranges should be searchable"),
        json!({
            "query": "hidden",
            "matches": [
                {
                    "item": "document",
                    "startChar": 6,
                    "endChar": 12,
                    "line": 2,
                    "preview": "hidden-one",
                },
                {
                    "item": "document",
                    "startChar": 23,
                    "endChar": 29,
                    "line": 4,
                    "preview": "hidden-two",
                },
            ],
            "limited": false,
        })
    );
    let query_contents = snapshot.contents_for_query();
    assert!(query_contents.contains(
        "Bundle item \"document\", original range C6-C16 / L2-L2:\n\
         <bundle_item>\nhidden-one\n</bundle_item>"
    ));
    assert!(query_contents.contains(
        "Bundle item \"document\", original range C23-C33 / L4-L4:\n\
         <bundle_item>\nhidden-two\n</bundle_item>"
    ));
    assert!(!query_contents.contains("seen0"));
    assert!(!query_contents.contains("fully-visible"));
}

#[test]
fn line_reads_cap_single_long_lines_and_offer_character_continuation() {
    let text = "x".repeat(MAX_READ_CHARS + 100);
    let mut store = QueryableBundleStore::default();
    let reference = store
        .insert(
            BundleOrigin::Manual,
            vec![BundleItemInput::new("long-line", &text)],
            /*context*/ None,
            /*operation_metadata*/ None,
        )
        .expect("bundle should be inserted");
    let snapshot = store
        .snapshot(&reference)
        .expect("bundle should be available");

    assert_eq!(
        snapshot
            .read_json(&json!({"item": "long-line", "startLine": 1, "lineCount": 1}))
            .expect("long line should be bounded"),
        json!({
            "item": "long-line",
            "view": "full",
            "coordinateUnit": "Unicode scalar values",
            "addressing": {"mode": "lines", "startLine": 1, "lineCount": 1},
            "requestedRange": {"startChar": 0, "endChar": MAX_READ_CHARS + 100},
            "returnedRange": {"startChar": 0, "endChar": MAX_READ_CHARS},
            "capped": true,
            "continuation": {
                "startChar": MAX_READ_CHARS,
                "remainingChars": 100,
                "hint": "Continue with character or virtual-chunk addressing.",
            },
            "originalChars": MAX_READ_CHARS + 100,
            "blocks": [{
                "startChar": 0,
                "endChar": MAX_READ_CHARS,
                "startLine": 1,
                "endLine": 1,
                "text": "x".repeat(MAX_READ_CHARS),
            }],
        })
    );

    let character_read = snapshot
        .read_json(&json!({
            "item": "long-line",
            "startChar": 0,
            "charCount": MAX_READ_CHARS + 100,
        }))
        .expect("character reads should report capped requested ranges");
    assert_eq!(
        character_read["requestedRange"],
        json!({"startChar": 0, "endChar": MAX_READ_CHARS + 100})
    );
    assert_eq!(
        character_read["returnedRange"],
        json!({"startChar": 0, "endChar": MAX_READ_CHARS})
    );
    assert_eq!(character_read["capped"], true);
    assert_eq!(character_read["continuation"]["remainingChars"], json!(100));

    assert_eq!(
        snapshot
            .read_json(&json!({"item": "long-line", "chunk": 5}))
            .expect("virtual chunks should reach beyond the line-read cap"),
        json!({
            "item": "long-line",
            "view": "full",
            "coordinateUnit": "Unicode scalar values",
            "addressing": {"mode": "virtual_chunks", "chunk": 5, "chunkCount": 1},
            "requestedRange": {
                "startChar": VIRTUAL_CHUNK_CHARS * 4,
                "endChar": MAX_READ_CHARS + 100,
            },
            "returnedRange": {
                "startChar": VIRTUAL_CHUNK_CHARS * 4,
                "endChar": MAX_READ_CHARS + 100,
            },
            "capped": false,
            "continuation": null,
            "originalChars": MAX_READ_CHARS + 100,
            "blocks": [{
                "startChar": VIRTUAL_CHUNK_CHARS * 4,
                "endChar": MAX_READ_CHARS + 100,
                "startLine": 1,
                "endLine": 1,
                "text": "x".repeat(100),
            }],
        })
    );
}

#[test]
fn selections_validate_names_without_copying_document_text() {
    let mut store = QueryableBundleStore::default();
    let reference = store
        .insert(
            BundleOrigin::Manual,
            vec![
                BundleItemInput::new("one", "alpha"),
                BundleItemInput::new("two", "beta"),
            ],
            /*context*/ None,
            /*operation_metadata*/ None,
        )
        .expect("bundle should be inserted");

    assert_eq!(
        store
            .snapshot(&BundleReference {
                selected: vec!["two".to_string()],
                ..reference.clone()
            })
            .expect("selection should be valid")
            .item_names(),
        vec!["two".to_string()]
    );
    assert_eq!(
        store
            .snapshot(&BundleReference {
                selected: vec!["missing".to_string()],
                ..reference
            })
            .expect_err("unknown selection should fail"),
        "bundle item `missing` does not exist; available items: one, two"
    );
}
