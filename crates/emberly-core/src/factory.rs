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

/// The reloadable configuration a [`ConfigReloader`] produces on a save (C-5):
/// the fresh prompt set and provider profiles, plus any change that only takes
/// effect on restart (named to the user rather than silently ignored).
pub struct ReloadedConfig {
    /// The resolved system prompt (base + project instructions), or `None`.
    pub system: Option<String>,
    /// The resolved `/compact` summary-prompt override, or `None`.
    pub summary_prompt: Option<String>,
    /// A fresh factory over the reloaded provider profiles.
    pub provider_factory: Arc<dyn ProviderFactory>,
    /// The reloaded profile names, sorted (for the picker).
    pub profiles: Vec<String>,
    /// Human-readable notes for changes that need a restart (e.g. a changed
    /// `sandbox.require`) — surfaced, never applied silently.
    pub restart_notes: Vec<String>,
}

/// Re-resolves configuration from disk after an in-app edit (C-5), injected by
/// the host so the engine can apply a saved edit without owning the config
/// resolution (which lives in the binary) — the same seam as
/// [`ProviderFactory`].
pub trait ConfigReloader: Send + Sync {
    /// Re-read config + prompts from disk. `Err` (e.g. a malformed
    /// `config.toml`) is surfaced as a harness error; the live config is kept.
    fn reload(&self) -> Result<ReloadedConfig, String>;
}
