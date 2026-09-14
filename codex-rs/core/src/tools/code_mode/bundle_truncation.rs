use codex_code_mode::BundleItemInput;
use codex_code_mode::BundleOrigin;
use codex_code_mode::BundleRange;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_utils_audio::estimate_audio_token_count;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::formatted_truncate_text_content_items_with_policy;
use codex_utils_output_truncation::truncate_function_output_items_with_policy;

use super::ExecContext;
use crate::unified_exec::resolve_max_tokens;

pub(super) fn truncate_code_mode_result(
    items: &[FunctionCallOutputContentItem],
    max_output_tokens: Option<usize>,
) -> Vec<FunctionCallOutputContentItem> {
    let max_output_tokens = resolve_max_tokens(max_output_tokens);
    let policy = TruncationPolicy::Tokens(max_output_tokens);
    if items
        .iter()
        .all(|item| matches!(item, FunctionCallOutputContentItem::InputText { .. }))
    {
        let (truncated_items, _) = formatted_truncate_text_content_items_with_policy(items, policy);
        return truncated_items;
    }

    truncate_function_output_items_with_policy(items, policy, estimate_audio_token_count)
}

pub(super) struct BundleAwareTruncation {
    pub(super) items: Vec<FunctionCallOutputContentItem>,
    pub(super) bundle_items: Vec<BundleItemInput>,
    pub(super) omitted_chars: usize,
}

pub(super) async fn truncate_code_mode_result_with_bundle(
    exec: &ExecContext,
    items: Vec<FunctionCallOutputContentItem>,
    max_output_tokens: Option<usize>,
) -> Vec<FunctionCallOutputContentItem> {
    let result = bundle_aware_truncation(items, max_output_tokens);
    if result.bundle_items.is_empty() {
        return result.items;
    }
    let context = result
        .items
        .iter()
        .filter_map(|item| match item {
            FunctionCallOutputContentItem::InputText { text } => Some(text.as_str()),
            FunctionCallOutputContentItem::InputImage { .. }
            | FunctionCallOutputContentItem::InputAudio { .. }
            | FunctionCallOutputContentItem::EncryptedContent { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let truncated_item_summaries = result
        .bundle_items
        .iter()
        .filter(|item| !item.omitted_ranges.is_empty())
        .map(|item| {
            let omitted = item
                .omitted_ranges
                .iter()
                .map(|range| range.end_char.saturating_sub(range.start_char))
                .sum::<usize>();
            let ranges = item
                .omitted_ranges
                .iter()
                .map(|range| format!("C{}-C{}", range.start_char, range.end_char))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{}={} chars ({omitted} omitted at {ranges})",
                item.name,
                item.text.chars().count()
            )
        })
        .collect::<Vec<_>>();
    let item_sizes = truncated_item_summaries
        .iter()
        .take(8)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let additional_items = truncated_item_summaries.len().saturating_sub(8);
    let additional_items = if additional_items == 0 {
        String::new()
    } else {
        format!(", plus {additional_items} more items")
    };
    let total_chars = result
        .bundle_items
        .iter()
        .map(|item| item.text.chars().count())
        .sum::<usize>();
    let item_count = result.bundle_items.len();
    let reference = match exec
        .session
        .services
        .code_mode_service
        .insert_bundle(
            BundleOrigin::TruncatedExecOutput,
            result.bundle_items,
            Some(context),
            /*operation_metadata*/ None,
        )
        .await
    {
        Ok(reference) => reference,
        Err(error) => {
            tracing::warn!(
                target: "codex_core::queryable_bundles",
                event = "automatic_bundle_failed",
                error,
            );
            return result.items;
        }
    };
    tracing::info!(
        target: "codex_core::queryable_bundles",
        event = "automatic_bundle_created",
        bundle_id = reference.id,
        item_count,
        truncated_item_count = truncated_item_summaries.len(),
        total_chars,
        omitted_chars = result.omitted_chars,
    );
    let mut output = result.items;
    output.push(FunctionCallOutputContentItem::InputText {
        text: format!(
            "\n[Truncated output saved as queryable bundle `{}`: {total_chars} chars total, {} chars omitted; {item_sizes}{additional_items}. Use `const trunc = bundles.open(\"{}\").omitted();` to inspect what was cut.]",
            reference.id, result.omitted_chars, reference.id,
        ),
    });
    output
}

pub(super) fn bundle_aware_truncation(
    items: Vec<FunctionCallOutputContentItem>,
    max_output_tokens: Option<usize>,
) -> BundleAwareTruncation {
    let truncated = truncate_code_mode_result(&items, max_output_tokens);
    if truncated == items {
        return BundleAwareTruncation {
            items,
            bundle_items: Vec::new(),
            omitted_chars: 0,
        };
    }
    let max_output_tokens = resolve_max_tokens(max_output_tokens);
    let policy = TruncationPolicy::Tokens(max_output_tokens);
    let all_text = items
        .iter()
        .all(|item| matches!(item, FunctionCallOutputContentItem::InputText { .. }));
    let bundle_items = if all_text {
        combined_text_bundle_items(&items, &truncated)
    } else {
        itemwise_bundle_items(&items, policy)
    };
    let omitted_chars = bundle_items
        .iter()
        .flat_map(|item| &item.omitted_ranges)
        .map(|range| range.end_char.saturating_sub(range.start_char))
        .sum();
    if omitted_chars == 0 {
        return BundleAwareTruncation {
            items: truncated,
            bundle_items: Vec::new(),
            omitted_chars,
        };
    }
    BundleAwareTruncation {
        items: truncated,
        bundle_items,
        omitted_chars,
    }
}

fn combined_text_bundle_items(
    items: &[FunctionCallOutputContentItem],
    truncated: &[FunctionCallOutputContentItem],
) -> Vec<BundleItemInput> {
    let texts = items
        .iter()
        .filter_map(|item| match item {
            FunctionCallOutputContentItem::InputText { text } => Some(text),
            FunctionCallOutputContentItem::InputImage { .. }
            | FunctionCallOutputContentItem::InputAudio { .. }
            | FunctionCallOutputContentItem::EncryptedContent { .. } => None,
        })
        .collect::<Vec<_>>();
    let mut combined = String::new();
    let mut item_ranges = Vec::with_capacity(texts.len());
    for text in &texts {
        if !combined.is_empty() {
            combined.push('\n');
        }
        let start = combined.chars().count();
        combined.push_str(text);
        item_ranges.push((start, combined.chars().count()));
    }
    let snippet = truncated
        .first()
        .and_then(|item| match item {
            FunctionCallOutputContentItem::InputText { text } => text.split_once("\n\n"),
            FunctionCallOutputContentItem::InputImage { .. }
            | FunctionCallOutputContentItem::InputAudio { .. }
            | FunctionCallOutputContentItem::EncryptedContent { .. } => None,
        })
        .map(|(_, snippet)| snippet)
        .unwrap_or_default();
    let omitted = omitted_range_for_snippet(&combined, snippet);
    texts
        .into_iter()
        .zip(item_ranges)
        .enumerate()
        .map(|(index, (text, (start, end)))| {
            let omitted_ranges = intersect_global_range(&omitted, start, end);
            BundleItemInput::new(format!("text-{}", index + 1), text.clone())
                .with_omitted_ranges(omitted_ranges)
        })
        .collect()
}

fn itemwise_bundle_items(
    items: &[FunctionCallOutputContentItem],
    policy: TruncationPolicy,
) -> Vec<BundleItemInput> {
    let mut remaining_budget = policy.token_budget();
    let mut text_index = 0usize;
    let mut bundle_items = Vec::new();
    for item in items {
        match item {
            FunctionCallOutputContentItem::InputText { text } => {
                text_index += 1;
                let cost = codex_utils_output_truncation::approx_token_count(text);
                let omitted_ranges = if remaining_budget == 0 {
                    vec![BundleRange::new(
                        /*start_char*/ 0,
                        /*end_char*/ text.chars().count(),
                    )]
                } else if cost <= remaining_budget {
                    remaining_budget = remaining_budget.saturating_sub(cost);
                    Vec::new()
                } else {
                    let snippet = codex_utils_output_truncation::truncate_text(
                        text,
                        TruncationPolicy::Tokens(remaining_budget),
                    );
                    remaining_budget = 0;
                    vec![omitted_range_for_snippet(text, &snippet)]
                };
                bundle_items.push(
                    BundleItemInput::new(format!("text-{text_index}"), text.clone())
                        .with_omitted_ranges(omitted_ranges),
                );
            }
            FunctionCallOutputContentItem::InputAudio { audio_url } => {
                remaining_budget =
                    remaining_budget.saturating_sub(estimate_audio_token_count(audio_url));
            }
            FunctionCallOutputContentItem::InputImage { .. }
            | FunctionCallOutputContentItem::EncryptedContent { .. } => {}
        }
    }
    bundle_items
}

fn omitted_range_for_snippet(original: &str, snippet: &str) -> BundleRange {
    let prefix = original
        .chars()
        .zip(snippet.chars())
        .take_while(|(left, right)| left == right)
        .count();
    let original_chars = original.chars().count();
    let suffix = original
        .chars()
        .rev()
        .zip(snippet.chars().rev())
        .take(original_chars.saturating_sub(prefix))
        .take_while(|(left, right)| left == right)
        .count();
    BundleRange::new(
        /*start_char*/ prefix,
        /*end_char*/ original_chars.saturating_sub(suffix),
    )
}

fn intersect_global_range(
    range: &BundleRange,
    item_start: usize,
    item_end: usize,
) -> Vec<BundleRange> {
    let start = range.start_char.max(item_start);
    let end = range.end_char.min(item_end);
    if start >= end {
        Vec::new()
    } else {
        vec![BundleRange::new(
            /*start_char*/ start.saturating_sub(item_start),
            /*end_char*/ end.saturating_sub(item_start),
        )]
    }
}
