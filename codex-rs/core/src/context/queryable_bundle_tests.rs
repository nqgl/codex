use codex_utils_output_truncation::approx_token_count;

use super::*;

#[test]
fn queryable_bundle_context_has_a_hard_token_bound() {
    let context = QueryableBundleContext::new(
        &"🧭".repeat(10_000),
        QueryableBundleMaterial::Contents(&"🪨".repeat(100_000)),
    );

    assert!(
        approx_token_count(&context.render()) < 9_000,
        "bundle query context must remain below the 10K-token item limit"
    );
}

#[test]
fn queryable_bundle_context_labels_complete_contents() {
    let context = QueryableBundleContext::new(
        "find alpha",
        QueryableBundleMaterial::Contents("alpha is here"),
    );

    assert_eq!(
        context.render(),
        "<queryable_bundle>\nTask:\nfind alpha\n\nComplete selected bundle contents:\nalpha is here\n</queryable_bundle>"
    );
}
