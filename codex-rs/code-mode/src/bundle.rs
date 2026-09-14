use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use serde_json::Value as JsonValue;
use serde_json::json;

#[path = "bundle_snapshot.rs"]
mod snapshot;

const BUNDLE_REF_TAG: &str = "__codex_bundle_ref";
const VIRTUAL_CHUNK_CHARS: usize = 4_096;

const MAX_BUNDLE_BYTES: usize = 64 * 1024 * 1024;
const MAX_STORE_BYTES: usize = 128 * 1024 * 1024;
const MAX_BUNDLES: usize = 64;
const MAX_BUNDLE_ITEMS: usize = 256;
const MAX_BUNDLE_ID_CHARS: usize = 128;
const MAX_ITEM_NAME_CHARS: usize = 128;
const MAX_CONTEXT_CHARS: usize = 8_000;
const MAX_READ_CHARS: usize = 16_384;
const MAX_READ_LINES: usize = 400;
const MAX_READ_CHUNKS: usize = 4;
const MAX_SEARCH_RESULTS: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundleOrigin {
    Manual,
    TruncatedExecOutput,
    QueryResults,
}

impl BundleOrigin {
    fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::TruncatedExecOutput => "truncated_exec_output",
            Self::QueryResults => "query_results",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleRange {
    pub start_char: usize,
    pub end_char: usize,
}

impl BundleRange {
    pub fn new(start_char: usize, end_char: usize) -> Self {
        Self {
            start_char,
            end_char,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleItemInput {
    pub name: String,
    pub text: String,
    pub omitted_ranges: Vec<BundleRange>,
}

impl BundleItemInput {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            text: text.into(),
            omitted_ranges: Vec::new(),
        }
    }

    pub fn with_omitted_ranges(mut self, omitted_ranges: Vec<BundleRange>) -> Self {
        self.omitted_ranges = omitted_ranges;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BundleView {
    Full,
    Omitted,
}

impl BundleView {
    fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Omitted => "omitted",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleReference {
    pub id: String,
    pub selected: Vec<String>,
    pub view: BundleView,
}

impl BundleReference {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            selected: Vec::new(),
            view: BundleView::Full,
        }
    }

    pub fn from_json(value: &JsonValue) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or_else(|| "bundle reference must be an object".to_string())?;
        let id = object
            .get("id")
            .and_then(JsonValue::as_str)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "bundle reference requires a non-empty id".to_string())?
            .to_string();
        if id.chars().count() > MAX_BUNDLE_ID_CHARS {
            return Err(format!(
                "bundle reference id exceeds {MAX_BUNDLE_ID_CHARS} characters"
            ));
        }
        let selected = object
            .get("selected")
            .map(|selected| {
                selected
                    .as_array()
                    .ok_or_else(|| "bundle reference selected field must be an array".to_string())?
                    .iter()
                    .map(|name| {
                        name.as_str().map(str::to_string).ok_or_else(|| {
                            "bundle reference selected names must be strings".to_string()
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?
            .unwrap_or_default();
        if selected.len() > MAX_BUNDLE_ITEMS {
            return Err(format!(
                "bundle reference selects {} items; the limit is {MAX_BUNDLE_ITEMS}",
                selected.len()
            ));
        }
        for name in &selected {
            validate_item_name(name)?;
        }
        let view = match object
            .get("view")
            .and_then(JsonValue::as_str)
            .unwrap_or("full")
        {
            "full" => BundleView::Full,
            "omitted" => BundleView::Omitted,
            view => return Err(format!("unknown bundle view `{view}`")),
        };
        Ok(Self { id, selected, view })
    }

    pub fn to_json(&self) -> JsonValue {
        json!({
            "id": self.id,
            "selected": self.selected,
            "view": self.view.as_str(),
        })
    }

    pub fn to_marker_json(&self) -> JsonValue {
        json!({ BUNDLE_REF_TAG: self.to_json() })
    }
}

#[derive(Clone, Debug)]
struct StoredBundleItem {
    name: String,
    text: Arc<str>,
    omitted_ranges: Vec<BundleRange>,
}

#[derive(Clone, Debug)]
struct StoredBundle {
    origin: BundleOrigin,
    created_at_unix_seconds: u64,
    context: Option<String>,
    operation_metadata: Option<JsonValue>,
    items: Vec<StoredBundleItem>,
    bytes: usize,
}

#[derive(Clone, Debug)]
pub struct BundleSnapshot {
    id: String,
    origin: BundleOrigin,
    created_at_unix_seconds: u64,
    context: Option<String>,
    operation_metadata: Option<JsonValue>,
    view: BundleView,
    items: Vec<StoredBundleItem>,
}

#[derive(Debug)]
pub struct QueryableBundleStore {
    bundles: HashMap<String, StoredBundle>,
    lru: VecDeque<String>,
    total_bytes: usize,
    next_id: u64,
}

impl Default for QueryableBundleStore {
    fn default() -> Self {
        Self {
            bundles: HashMap::new(),
            lru: VecDeque::new(),
            total_bytes: 0,
            next_id: 1,
        }
    }
}

impl QueryableBundleStore {
    pub fn insert(
        &mut self,
        origin: BundleOrigin,
        items: Vec<BundleItemInput>,
        context: Option<String>,
        operation_metadata: Option<JsonValue>,
    ) -> Result<BundleReference, String> {
        if items.is_empty() {
            return Err("a bundle requires at least one item".to_string());
        }
        if items.len() > MAX_BUNDLE_ITEMS {
            return Err(format!(
                "bundle has {} items; the limit is {MAX_BUNDLE_ITEMS}",
                items.len()
            ));
        }

        let mut seen_names = HashSet::new();
        let mut bytes = 0usize;
        let mut stored_items = Vec::with_capacity(items.len());
        for item in items {
            validate_item_name(&item.name)?;
            if !seen_names.insert(item.name.clone()) {
                return Err(format!("bundle item name `{}` is duplicated", item.name));
            }
            bytes = bytes.saturating_add(item.text.len());
            if bytes > MAX_BUNDLE_BYTES {
                return Err(format!(
                    "bundle exceeds the {} MiB per-bundle limit",
                    MAX_BUNDLE_BYTES / (1024 * 1024)
                ));
            }
            let char_count = item.text.chars().count();
            stored_items.push(StoredBundleItem {
                name: item.name,
                text: Arc::from(item.text),
                omitted_ranges: normalize_ranges(item.omitted_ranges, char_count),
            });
        }

        let id = format!("bundle_{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        let context = context.map(|context| take_chars(&context, MAX_CONTEXT_CHARS));
        let created_at_unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let item_count = stored_items.len();
        self.evict_for(bytes);
        self.bundles.insert(
            id.clone(),
            StoredBundle {
                origin,
                created_at_unix_seconds,
                context,
                operation_metadata,
                items: stored_items,
                bytes,
            },
        );
        self.lru.push_back(id.clone());
        self.total_bytes = self.total_bytes.saturating_add(bytes);
        tracing::info!(
            target: "codex_code_mode::bundles",
            event = "bundle_created",
            bundle_id = id,
            origin = origin.as_str(),
            item_count,
            bytes,
        );
        Ok(BundleReference::new(id))
    }

    pub fn snapshot(&mut self, reference: &BundleReference) -> Result<BundleSnapshot, String> {
        let bundle = self.bundles.get(&reference.id).cloned().ok_or_else(|| {
            format!(
                "bundle `{}` is unavailable; it may have expired or been evicted",
                reference.id
            )
        })?;
        self.touch(&reference.id);

        let selected = reference
            .selected
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        let available = bundle
            .items
            .iter()
            .map(|item| item.name.as_str())
            .collect::<HashSet<_>>();
        if let Some(missing) = reference
            .selected
            .iter()
            .find(|name| !available.contains(name.as_str()))
        {
            let mut names = available.into_iter().collect::<Vec<_>>();
            names.sort_unstable();
            return Err(format!(
                "bundle item `{missing}` does not exist; available items: {}",
                names.join(", ")
            ));
        }
        let items = bundle
            .items
            .into_iter()
            .filter(|item| selected.is_empty() || selected.contains(item.name.as_str()))
            .filter(|item| reference.view == BundleView::Full || !item.omitted_ranges.is_empty())
            .collect();
        Ok(BundleSnapshot {
            id: reference.id.clone(),
            origin: bundle.origin,
            created_at_unix_seconds: bundle.created_at_unix_seconds,
            context: bundle.context,
            operation_metadata: bundle.operation_metadata,
            view: reference.view,
            items,
        })
    }

    fn evict_for(&mut self, incoming_bytes: usize) {
        while self.bundles.len() >= MAX_BUNDLES
            || self.total_bytes.saturating_add(incoming_bytes) > MAX_STORE_BYTES
        {
            let Some(id) = self.lru.pop_front() else {
                break;
            };
            if let Some(bundle) = self.bundles.remove(&id) {
                self.total_bytes = self.total_bytes.saturating_sub(bundle.bytes);
                tracing::info!(
                    target: "codex_code_mode::bundles",
                    event = "bundle_evicted",
                    bundle_id = id,
                    bytes = bundle.bytes,
                );
            }
        }
    }

    fn touch(&mut self, id: &str) {
        if let Some(index) = self.lru.iter().position(|candidate| candidate == id) {
            self.lru.remove(index);
        }
        self.lru.push_back(id.to_string());
    }
}

pub fn bundle_items_from_json(value: JsonValue) -> Result<Vec<BundleItemInput>, String> {
    match value {
        JsonValue::Object(items) => {
            if items.is_empty() {
                return Err("cannot create a bundle from an empty object".to_string());
            }
            items
                .into_iter()
                .map(|(name, value)| Ok(BundleItemInput::new(name, render_json_value(value)?)))
                .collect()
        }
        JsonValue::Array(items) => {
            if items.is_empty() {
                return Err("cannot create a bundle from an empty array".to_string());
            }
            items
                .into_iter()
                .enumerate()
                .map(|(index, value)| {
                    Ok(BundleItemInput::new(
                        format!("item-{}", index + 1),
                        render_json_value(value)?,
                    ))
                })
                .collect()
        }
        value => Ok(vec![BundleItemInput::new(
            "document",
            render_json_value(value)?,
        )]),
    }
}

fn render_json_value(value: JsonValue) -> Result<String, String> {
    match value {
        JsonValue::String(text) => Ok(text),
        value => serde_json::to_string_pretty(&value)
            .map_err(|error| format!("failed to serialize bundle item: {error}")),
    }
}

fn validate_item_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("bundle item names must not be empty".to_string());
    }
    if name.chars().count() > MAX_ITEM_NAME_CHARS {
        return Err(format!(
            "bundle item name `{name}` exceeds {MAX_ITEM_NAME_CHARS} characters"
        ));
    }
    if name.chars().any(char::is_control) {
        return Err("bundle item names must not contain control characters".to_string());
    }
    Ok(())
}

fn normalize_ranges(mut ranges: Vec<BundleRange>, char_count: usize) -> Vec<BundleRange> {
    ranges.iter_mut().for_each(|range| {
        range.start_char = range.start_char.min(char_count);
        range.end_char = range.end_char.min(char_count);
    });
    ranges.retain(|range| range.start_char < range.end_char);
    ranges.sort_by_key(|range| range.start_char);
    let mut normalized: Vec<BundleRange> = Vec::new();
    for range in ranges {
        if let Some(previous) = normalized.last_mut()
            && range.start_char <= previous.end_char
        {
            previous.end_char = previous.end_char.max(range.end_char);
        } else {
            normalized.push(range);
        }
    }
    normalized
}

fn take_chars(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

#[cfg(test)]
#[path = "bundle_tests.rs"]
mod tests;
