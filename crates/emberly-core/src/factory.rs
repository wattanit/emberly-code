//! [`ProviderFactory`] — how the engine builds a provider for a profile name
//! when the user switches models in-session (C-6). Provider construction
//! (config, endpoints, auth, the HTTPS client) lives in the host binary, not
//! the engine; the host injects a factory so the engine can switch without
//! depending on any of that (keeping the engine/frontend boundary clean, A-1).

use std::sync::Arc;

use emberly_providers::Provider;
use emberly_sandbox::Rule;
use emberly_tools::{ToolRegistry, TruncateConfig};

use crate::engine::{
    AgentsConfig, CompletionCheck, CompletionConfig, ContextConfig, LoopConfig, MemoryConfig,
    SkillsConfig,
};

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

/// The reloadable configuration a [`ConfigReloader`] produces on a save (C-5).
/// Everything here live-reloads into the running [`Engine`](crate::engine::Engine)
/// except what's structurally tied to already-constructed process state (OS
/// confinement, the active provider/model) — those surface via
/// `restart_notes` instead of being applied.
pub struct ReloadedConfig {
    /// The resolved system prompt (base + project instructions), or `None`.
    pub system: Option<String>,
    /// The resolved `/compact` summary-prompt override, or `None`.
    pub summary_prompt: Option<String>,
    /// A fresh factory over the reloaded provider profiles.
    pub provider_factory: Arc<dyn ProviderFactory>,
    /// The reloaded profile names, sorted (for the picker).
    pub profiles: Vec<String>,
    /// The `provider =` / `model =` selectors config.toml currently names as
    /// the default — distinct from the *active* session provider, which a
    /// reload never auto-switches (C-5, C-6). A changed value here is
    /// reported with a concrete `/model` command to follow it.
    pub configured_provider: Option<String>,
    pub configured_model: Option<String>,
    /// True when any provider profile's *content* changed — a new/removed
    /// profile (already reflected in `profiles`) or a field edited on an
    /// existing one (e.g. filling in `base_url`/`auth` on a profile that was
    /// already present, which leaves `profiles` byte-for-byte identical).
    /// Lets the engine report the change even though the name list didn't
    /// move; `provider_factory` above is always the fresh one either way.
    pub provider_config_changed: bool,
    /// `ui.tool_explanations` (T-9, Tech Spec §5.4).
    pub tool_explanations: bool,
    /// Loop-breaking guardrail tunables (S-5, Tech Spec §7).
    pub loop_config: LoopConfig,
    /// Completion-gate tunables (S-6, Tech Spec §7).
    pub completion_config: CompletionConfig,
    /// Completion checks from `[[completion.check]]` config (S-6).
    pub completion_checks: Vec<CompletionCheck>,
    /// Truncation-at-ingestion knobs (Tech Spec §5.3).
    pub truncate: TruncateConfig,
    /// Adaptive context-window + compaction config (FR-3, Tech Spec §7/§8).
    pub context: ContextConfig,
    /// Maximum image file size in bytes (Tech Spec §5.2).
    pub image_max_bytes: usize,
    /// Maximum document file size in bytes (Tech Spec §5.2).
    pub document_max_bytes: usize,
    /// Memory config (FR-6, Tech Spec §8.1). A changed value triggers a
    /// memory-store rebuild, not just a struct swap.
    pub memory: MemoryConfig,
    /// Skills config (FR-7, Tech Spec §8.2). A changed value triggers a
    /// skill-catalog rebuild, not just a struct swap.
    pub skills: SkillsConfig,
    /// Multi-agent subsystem config (FR-9, Tech Spec §8.4).
    pub agents: AgentsConfig,
    /// The rebuilt tool registry (default suite plus e.g. `web_search` when
    /// `[search]` is enabled and configured).
    pub tools: ToolRegistry,
    /// The raw config-sourced permission rules (global + project
    /// `permissions.toml`), for [`RuleEngine::reload_config_rules`]
    /// (`emberly_sandbox`) to rebuild in place without dropping session
    /// grants.
    pub rule_specs: Vec<Rule>,
    /// Human-readable notes for changes that need a restart (e.g. a changed
    /// `sandbox.require`) — surfaced, never applied silently.
    pub restart_notes: Vec<String>,
    /// Non-restart-related issues worth surfacing (e.g. a malformed
    /// `permissions.toml` entry, a `[search]` adapter with no endpoint) —
    /// shown alongside the reload notice, distinct from `restart_notes`.
    pub warnings: Vec<String>,
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
