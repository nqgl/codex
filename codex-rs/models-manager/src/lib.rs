pub mod cache;
pub mod collaboration_mode_presets;
pub(crate) mod config;
pub mod manager;
pub mod model_info;
pub mod model_presets;
pub mod test_support;

pub use codex_protocol::auth::AuthMode;
pub use config::ModelsManagerConfig;

// Cargo leaves source builds at 0.0.0, but the models endpoint uses the client
// version to decide which models this binary can run. Keep this in step with
// the upstream release tag integrated into the custom harness.
const SOURCE_BUILD_MODEL_CATALOG_VERSION: &str = "0.156.1";

/// Load the bundled model catalog shipped with `codex-models-manager`.
pub fn bundled_models_response()
-> std::result::Result<codex_protocol::openai_models::ModelsResponse, serde_json::Error> {
    serde_json::from_str(include_str!("../models.json"))
}

/// Return the release-compatible whole version used for model discovery and caching.
/// Packaged builds use their package version; source builds use the last integrated release.
pub fn client_version_to_whole() -> String {
    let package_version = format!(
        "{}.{}.{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    );
    if package_version == "0.0.0" {
        SOURCE_BUILD_MODEL_CATALOG_VERSION.to_string()
    } else {
        package_version
    }
}
