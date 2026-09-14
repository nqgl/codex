use super::ContextualUserFragment;
use codex_protocol::models::ContentItemKind;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::truncate_text;

const MAX_TASK_TOKENS: usize = 1_000;
const MAX_MATERIAL_TOKENS: usize = 7_000;

pub(crate) enum QueryableBundleMaterial<'a> {
    Catalogue(&'a str),
    Contents(&'a str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QueryableBundleContext {
    task: String,
    material_heading: &'static str,
    material: String,
}

impl QueryableBundleContext {
    pub(crate) fn new(task: &str, material: QueryableBundleMaterial<'_>) -> Self {
        let (material_heading, material) = match material {
            QueryableBundleMaterial::Catalogue(material) => ("Bundle catalogue", material),
            QueryableBundleMaterial::Contents(material) => {
                ("Complete selected bundle contents", material)
            }
        };
        Self {
            task: truncate_text(task, TruncationPolicy::Tokens(MAX_TASK_TOKENS)),
            material_heading,
            material: truncate_text(material, TruncationPolicy::Tokens(MAX_MATERIAL_TOKENS)),
        }
    }
}

impl ContextualUserFragment for QueryableBundleContext {
    fn content_kind(&self) -> ContentItemKind {
        ContentItemKind("code_mode.queryable_bundle".to_string())
    }

    fn role(&self) -> &'static str {
        "user"
    }

    fn markers(&self) -> (&'static str, &'static str) {
        Self::type_markers()
    }

    fn type_markers() -> (&'static str, &'static str) {
        ("<queryable_bundle>", "</queryable_bundle>")
    }

    fn body(&self) -> String {
        format!(
            "\nTask:\n{}\n\n{}:\n{}\n",
            self.task, self.material_heading, self.material
        )
    }
}

#[cfg(test)]
#[path = "queryable_bundle_tests.rs"]
mod tests;
