//! [`ProviderFactory`] — how the engine builds a provider for a profile name
//! when the user switches models in-session (C-6). Provider construction
//! (config, endpoints, auth, the HTTPS client) lives in the host binary, not
//! the engine; the host injects a factory so the engine can switch without
//! depending on any of that (keeping the engine/frontend boundary clean, A-1).

use std::sync::Arc;

use emberly_providers::Provider;

/// A provider chosen for the active profile: the client, the resolved model id,
/// and the profile name used as the display/transcript label.
pub struct ProviderChoice {
    pub provider: Arc<dyn Provider>,
    /// The profile name (e.g. `zai`), shown in the sidebar and recorded.
    pub profile: String,
    /// The resolved model id (e.g. `glm-4.6`).
    pub model: String,
}

/// Builds a provider from a profile name, injected by the host (C-6). The
/// engine calls this on [`Command::SwitchModel`](crate::command::Command::SwitchModel).
pub trait ProviderFactory: Send + Sync {
    /// Build the provider for `profile`, using `model` as the model id.
    /// Returns a human-readable error message on failure (unknown profile,
    /// missing key, …); the engine surfaces it as a harness-world error.
    fn build(&self, profile: &str, model: &str) -> Result<ProviderChoice, String>;

    /// The configured profile names, sorted — for validation and the picker.
    fn profiles(&self) -> Vec<String>;
}
