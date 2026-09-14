use serde_json::Value as JsonValue;
use serde_json::json;

use super::*;

const MAX_SEARCH_QUERY_CHARS: usize = 1_024;

impl BundleSnapshot {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn freshness_json(&self) -> JsonValue {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(self.created_at_unix_seconds);
        json!({
            "status": "snapshot",
            "sourceStatus": "unknown",
            "ageSeconds": now.saturating_sub(self.created_at_unix_seconds),
            "warning": "This is an immutable snapshot. Source freshness is not guaranteed; rerun the source tool when current state matters.",
        })
    }

    pub fn info_json(&self) -> JsonValue {
        let items = self
            .items
            .iter()
            .map(|item| {
                let char_count = item.text.chars().count();
                json!({
                    "name": item.name,
                    "bytes": item.text.len(),
                    "chars": char_count,
                    "lines": line_count(&item.text),
                    "virtualChunks": char_count.div_ceil(VIRTUAL_CHUNK_CHARS),
                    "omittedRanges": ranges_json(&item.text, &item.omitted_ranges),
                })
            })
            .collect::<Vec<_>>();
        json!({
            "id": self.id,
            "origin": self.origin.as_str(),
            "createdAtUnixSeconds": self.created_at_unix_seconds,
            "view": self.view.as_str(),
            "items": items,
            "context": self.context,
            "operation": self.operation_metadata,
            "freshness": self.freshness_json(),
        })
    }

    pub fn catalogue(&self) -> String {
        let mut lines = vec![
            format!(
                "Bundle {} ({}, {} view), captured at Unix time {}.",
                self.id,
                self.origin.as_str(),
                self.view.as_str(),
                self.created_at_unix_seconds
            ),
            format!(
                "Freshness: {}",
                serde_json::to_string(&self.freshness_json()).unwrap_or_else(|_| {
                    "immutable snapshot; source freshness is unknown".to_string()
                })
            ),
        ];
        if let Some(context) = &self.context {
            lines.push(format!("Retained inline context:\n{context}"));
        }
        lines.push("Items:".to_string());
        for item in &self.items {
            let preview = self.item_preview(item, 320);
            lines.push(format!(
                "- {}: {} chars, {} lines, omitted ranges [{}]\n  preview: {}",
                item.name,
                item.text.chars().count(),
                line_count(&item.text),
                format_ranges(&item.text, &item.omitted_ranges),
                preview.replace('\n', "\\n")
            ));
        }
        lines.join("\n")
    }

    pub fn contents_for_query(&self) -> String {
        let mut sections = vec![
            format!(
                "Bundle {} ({}, {} view), captured at Unix time {}.",
                self.id,
                self.origin.as_str(),
                self.view.as_str(),
                self.created_at_unix_seconds
            ),
            format!(
                "Freshness: {}",
                serde_json::to_string(&self.freshness_json()).unwrap_or_else(|_| {
                    "immutable snapshot; source freshness is unknown".to_string()
                })
            ),
        ];
        if let Some(context) = &self.context {
            sections.push(format!("Retained inline context:\n{context}"));
        }
        for item in &self.items {
            let ranges = self.allowed_ranges(item);
            if ranges.is_empty() {
                continue;
            }
            for range in ranges {
                let start_line = line_at_char(&item.text, range.start_char);
                let end_line = line_at_char(&item.text, range.end_char.saturating_sub(1));
                let text = slice_chars(&item.text, range.start_char, range.end_char);
                sections.push(format!(
                    "Bundle item {:?}, original range C{}-C{} / L{}-L{}:\n<bundle_item>\n{}\n</bundle_item>",
                    item.name,
                    range.start_char,
                    range.end_char,
                    start_line,
                    end_line,
                    text
                ));
            }
        }
        sections.join("\n\n")
    }

    pub fn read_json(&self, args: &JsonValue) -> Result<JsonValue, String> {
        let item_name = required_string(args, "item")?;
        let item = self.item(item_name)?;
        let total_chars = item.text.chars().count();
        let requested = requested_char_range(args, &item.text)?;
        let blocks = self
            .allowed_ranges(item)
            .into_iter()
            .filter_map(|range| intersect_range(&range, requested.start_char, requested.end_char))
            .map(|range| {
                json!({
                    "startChar": range.start_char,
                    "endChar": range.end_char,
                    "startLine": line_at_char(&item.text, range.start_char),
                    "endLine": line_at_char(&item.text, range.end_char.saturating_sub(1)),
                    "text": slice_chars(&item.text, range.start_char, range.end_char),
                })
            })
            .collect::<Vec<_>>();
        let continuation = (requested.end_char < requested.requested_end_char).then(|| {
            json!({
                "startChar": requested.end_char,
                "remainingChars": requested.requested_end_char - requested.end_char,
                "hint": "Continue with character or virtual-chunk addressing.",
            })
        });
        Ok(json!({
            "item": item.name,
            "view": self.view.as_str(),
            "coordinateUnit": "Unicode scalar values",
            "addressing": requested.addressing,
            "requestedRange": {
                "startChar": requested.start_char,
                "endChar": requested.requested_end_char,
            },
            "returnedRange": {
                "startChar": requested.start_char,
                "endChar": requested.end_char,
            },
            "capped": continuation.is_some(),
            "continuation": continuation,
            "originalChars": total_chars,
            "blocks": blocks,
        }))
    }

    pub fn search_json(&self, query: &str, limit: usize) -> Result<JsonValue, String> {
        if query.is_empty() {
            return Err("bundle search query must not be empty".to_string());
        }
        if query.chars().count() > MAX_SEARCH_QUERY_CHARS {
            return Err(format!(
                "bundle search query exceeds {MAX_SEARCH_QUERY_CHARS} characters"
            ));
        }
        let limit = limit.clamp(1, MAX_SEARCH_RESULTS);
        let needle = query.to_ascii_lowercase();
        let mut matches = Vec::new();
        for item in &self.items {
            let haystack = item.text.to_ascii_lowercase();
            let allowed = self.allowed_ranges(item);
            for (byte_start, _) in haystack.match_indices(&needle) {
                let start_char = item.text[..byte_start].chars().count();
                let end_char = start_char.saturating_add(query.chars().count());
                let Some(allowed_range) = allowed
                    .iter()
                    .find(|range| range.start_char <= start_char && end_char <= range.end_char)
                else {
                    continue;
                };
                let preview_start = start_char.saturating_sub(120).max(allowed_range.start_char);
                let preview_end = end_char.saturating_add(120).min(allowed_range.end_char);
                matches.push(json!({
                    "item": item.name,
                    "startChar": start_char,
                    "endChar": end_char,
                    "line": line_at_char(&item.text, start_char),
                    "preview": slice_chars(&item.text, preview_start, preview_end),
                }));
                if matches.len() >= limit {
                    return Ok(json!({"query": query, "matches": matches, "limited": true}));
                }
            }
        }
        Ok(json!({"query": query, "matches": matches, "limited": false}))
    }

    pub fn item_names(&self) -> Vec<String> {
        self.items.iter().map(|item| item.name.clone()).collect()
    }

    fn item(&self, name: &str) -> Result<&StoredBundleItem, String> {
        self.items
            .iter()
            .find(|item| item.name == name)
            .ok_or_else(|| {
                let names = self
                    .items
                    .iter()
                    .map(|item| item.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("bundle item `{name}` does not exist; selected items: {names}")
            })
    }

    fn allowed_ranges(&self, item: &StoredBundleItem) -> Vec<BundleRange> {
        match self.view {
            BundleView::Full => vec![BundleRange::new(
                /*start_char*/ 0,
                /*end_char*/ item.text.chars().count(),
            )],
            BundleView::Omitted => item.omitted_ranges.clone(),
        }
    }

    fn item_preview(&self, item: &StoredBundleItem, max_chars: usize) -> String {
        let text = self
            .allowed_ranges(item)
            .first()
            .map(|range| slice_chars(&item.text, range.start_char, range.end_char))
            .unwrap_or_default();
        take_chars(&text, max_chars)
    }
}

struct RequestedCharRange {
    start_char: usize,
    end_char: usize,
    requested_end_char: usize,
    addressing: JsonValue,
}

fn requested_char_range(args: &JsonValue, text: &str) -> Result<RequestedCharRange, String> {
    let total_chars = text.chars().count();
    let has_line = args.get("startLine").is_some() || args.get("lineCount").is_some();
    let has_char = args.get("startChar").is_some() || args.get("charCount").is_some();
    let has_chunk = args.get("chunk").is_some() || args.get("chunkCount").is_some();
    if usize::from(has_line) + usize::from(has_char) + usize::from(has_chunk) > 1 {
        return Err(
            "bundle read accepts one addressing mode: lines, characters, or virtual chunks"
                .to_string(),
        );
    }
    if has_char {
        let start = optional_usize(args, "startChar")?.unwrap_or_default();
        let requested_count = optional_usize(args, "charCount")?.unwrap_or(MAX_READ_CHARS);
        let count = requested_count.min(MAX_READ_CHARS);
        let start_char = start.min(total_chars);
        let end_char = start.saturating_add(count).min(total_chars);
        let requested_end_char = start.saturating_add(requested_count).min(total_chars);
        return Ok(RequestedCharRange {
            start_char,
            end_char,
            requested_end_char,
            addressing: json!({
                "mode": "characters",
                "startChar": start,
                "charCount": requested_count,
            }),
        });
    }
    if has_chunk {
        let chunk = optional_usize(args, "chunk")?.unwrap_or(1).max(1);
        let requested_chunk_count = optional_usize(args, "chunkCount")?.unwrap_or(1).max(1);
        let chunk_count = requested_chunk_count.min(MAX_READ_CHUNKS);
        let start = chunk.saturating_sub(1).saturating_mul(VIRTUAL_CHUNK_CHARS);
        let end = start
            .saturating_add(chunk_count.saturating_mul(VIRTUAL_CHUNK_CHARS))
            .min(total_chars);
        let requested_end_char = start
            .saturating_add(requested_chunk_count.saturating_mul(VIRTUAL_CHUNK_CHARS))
            .min(total_chars);
        return Ok(RequestedCharRange {
            start_char: start.min(total_chars),
            end_char: end,
            requested_end_char,
            addressing: json!({
                "mode": "virtual_chunks",
                "chunk": chunk,
                "chunkCount": requested_chunk_count,
            }),
        });
    }

    let start_line = optional_usize(args, "startLine")?.unwrap_or(1).max(1);
    let requested_line_count = optional_usize(args, "lineCount")?.unwrap_or(100).max(1);
    let line_count = requested_line_count.min(MAX_READ_LINES);
    let (start_char, line_limited_end_char) = line_range_chars(text, start_line, line_count);
    let (_, requested_end_char) = line_range_chars(text, start_line, requested_line_count);
    let end_char = line_limited_end_char.min(start_char.saturating_add(MAX_READ_CHARS));
    Ok(RequestedCharRange {
        start_char,
        end_char,
        requested_end_char,
        addressing: json!({
            "mode": "lines",
            "startLine": start_line,
            "lineCount": requested_line_count,
        }),
    })
}

fn required_string<'a>(args: &'a JsonValue, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("bundle operation requires a non-empty `{name}` string"))
}

fn optional_usize(args: &JsonValue, name: &str) -> Result<Option<usize>, String> {
    args.get(name)
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| format!("bundle operation `{name}` must be a non-negative integer"))
        })
        .transpose()
}

fn intersect_range(range: &BundleRange, start: usize, end: usize) -> Option<BundleRange> {
    let start_char = range.start_char.max(start);
    let end_char = range.end_char.min(end);
    (start_char < end_char).then(|| BundleRange::new(start_char, end_char))
}

fn ranges_json(text: &str, ranges: &[BundleRange]) -> Vec<JsonValue> {
    ranges
        .iter()
        .map(|range| {
            json!({
                "startChar": range.start_char,
                "endChar": range.end_char,
                "chars": range.end_char.saturating_sub(range.start_char),
                "startLine": line_at_char(text, range.start_char),
                "endLine": line_at_char(text, range.end_char.saturating_sub(1)),
            })
        })
        .collect()
}

fn format_ranges(text: &str, ranges: &[BundleRange]) -> String {
    if ranges.is_empty() {
        return "none".to_string();
    }
    ranges
        .iter()
        .map(|range| {
            format!(
                "C{}-C{} ({} chars, L{}-L{})",
                range.start_char,
                range.end_char,
                range.end_char.saturating_sub(range.start_char),
                line_at_char(text, range.start_char),
                line_at_char(text, range.end_char.saturating_sub(1)),
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.chars().filter(|character| *character == '\n').count() + 1
    }
}

fn line_at_char(text: &str, char_offset: usize) -> usize {
    text.chars()
        .take(char_offset)
        .filter(|character| *character == '\n')
        .count()
        + 1
}

fn line_range_chars(text: &str, start_line: usize, count: usize) -> (usize, usize) {
    let start_index = start_line.saturating_sub(1);
    let mut starts = vec![0usize];
    for (index, character) in text.chars().enumerate() {
        if character == '\n' {
            starts.push(index + 1);
        }
    }
    let total_chars = text.chars().count();
    let start = starts.get(start_index).copied().unwrap_or(total_chars);
    let end = starts
        .get(start_index.saturating_add(count))
        .copied()
        .unwrap_or(total_chars);
    (start, end)
}

fn slice_chars(text: &str, start: usize, end: usize) -> String {
    text.chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}
