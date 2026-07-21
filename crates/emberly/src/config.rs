//! Configuration and secrets for the binary (Tech Spec §8; Phase 3 group 6).
//!
//! Resolution order (lowest → highest): built-in defaults → global
//! `~/.config/emberly/config.toml` → project `.agents/config.toml` →
//! `EMBERLY_*` environment overrides. API keys are read from the environment
//! first, else from `~/.config/emberly/keys.toml`, which must be `0600`
//! (secrets never live in project config and are never printed). Phase 5 adds
//! provenance tracking (C-3, [`show`]), project instructions (AGENTS.md/
//! CLAUDE.md, C-1), and per-model-family prompt resolution (P-7).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};
use emberly_core::{parse_rules, ConfigProvenance, Rule, RuleSource};
use serde::Deserialize;

/// A parsed `config.toml`. All fields optional so files can be partial.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ConfigFile {
    /// The active provider profile — which `[providers.<name>]` to use.
    pub provider: Option<String>,
    /// The active model id (looked up within the active profile).
    pub model: Option<String>,
    /// Named provider profiles (P-8, Tech Spec §4.5). Merged across tiers by
    /// name; baked-in profiles are injected at the lowest tier.
    #[serde(default)]
    pub providers: HashMap<String, ProfileFile>,
    /// Sandbox policy knobs (Requirements §6.7).
    #[serde(default)]
    pub sandbox: SandboxConfig,
    /// Reasoning-trail view default (Design §4.4): `collapsed` | `expanded` |
    /// `hidden`. A view choice only — the trace is always recorded (P-10).
    pub reasoning: Option<String>,
    /// `[ui]` presentation toggles (Tech Spec §8).
    #[serde(default)]
    pub ui: UiConfig,
    /// `[trust]` workspace-trust settings (FR-1). **Honored only from the global
    /// tier** — a project `[trust]` is ignored (a repo cannot self-trust); see
    /// [`global_trust_dirs`] and the notice in [`load`].
    #[serde(default)]
    pub trust: TrustConfig,
    /// `[loop]` guardrail settings (S-5). Threaded to the engine (Tech Spec §7).
    #[serde(default, rename = "loop")]
    pub loop_: LoopConfig,
    /// `[completion]` + `[[completion.check]]` — the completion gate (S-6,
    /// Tech Spec §7/§8). Threaded to the engine.
    #[serde(default)]
    pub completion: CompletionConfigFile,
    /// `[truncate]` tool-result reduction + size backstop (FR-2, Tech Spec §8).
    #[serde(default)]
    pub truncate: TruncateConfigFile,
    /// `[context]` adaptive window + compaction tail (FR-3, Tech Spec §7/§8).
    #[serde(default)]
    pub context: ContextConfigFile,
    /// `[image]` read_image size cap (P-11, Tech Spec §5.2).
    #[serde(default)]
    pub image: ImageConfigFile,
    /// `[document]` read_document size cap (P-12, Tech Spec §5.2).
    #[serde(default)]
    pub document: DocumentConfigFile,
    /// `[memory]` persistent memory (FR-6, Tech Spec §8.1).
    #[serde(default)]
    pub memory: MemoryConfigFile,
    /// `[skills]` skill system (FR-7, Tech Spec §8.2).
    #[serde(default)]
    pub skills: SkillsConfigFile,
    /// `[search]` web-search backend (T-14, Tech Spec §5.5).
    #[serde(default)]
    pub search: SearchConfigFile,
}

/// `[ui]` — presentation toggles that shape what the interface shows without
/// changing what the agent may do (Tech Spec §8).
#[derive(Debug, Default, Clone, Deserialize)]
pub struct UiConfig {
    /// Tool-call explanation line (T-9, Design §4.5). When on, the model is
    /// asked to caption non-obvious calls; when off, the schema property and the
    /// prompt instruction are both omitted so no tokens are spent. Default
    /// `true`.
    pub tool_explanations: Option<bool>,
    /// Pointer (mouse) interaction in the rich TUI (Design §3.4, Tech Spec §9).
    /// When on (the default), wheel scroll and click-to-select are enabled and
    /// the terminal's mouse is captured; when off, the terminal keeps its native
    /// pointer behavior (selection everywhere) and no capture happens. Additive
    /// convenience only — the keyboard can always do everything the mouse can.
    /// Ignored in degraded mode, which never captures the mouse (§7). Default
    /// `true`.
    pub mouse: Option<bool>,
}

/// `[trust]` — workspace trust (FR-1). `trusted_dirs` pre-declares folders
/// trusted without a prompt. **Global-tier only**: project config cannot
/// contribute here, so a repository can never pre-declare itself trusted.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct TrustConfig {
    #[serde(default)]
    pub trusted_dirs: Vec<String>,
}

/// `[loop]` — the loop-breaking guardrail (S-5, Tech Spec §7). All optional;
/// the engine applies defaults. Wired to the engine in a later group.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct LoopConfig {
    pub enabled: Option<bool>,
    pub repeat_window: Option<u32>,
    pub max_no_progress_turns: Option<u32>,
}

/// `[completion]` — the completion gate (S-6, Tech Spec §7). All optional;
/// the engine applies defaults when unset. Inert until at least one
/// `[[completion.check]]` is registered, regardless of `enabled`.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct CompletionConfigFile {
    pub enabled: Option<bool>,
    pub max_attempts: Option<usize>,
    /// Registered pass/fail checks the gate runs on a completion attempt.
    #[serde(default)]
    pub check: Vec<CompletionCheckFile>,
}

/// One `[[completion.check]]` entry: a named command and the exit code that
/// counts as a pass (S-6, Tech Spec §7).
#[derive(Debug, Clone, Deserialize)]
pub struct CompletionCheckFile {
    pub name: String,
    pub command: String,
    pub expect_exit: Option<i32>,
}

/// `[context]` — the adaptive context window and compaction tail (FR-3, Tech
/// Spec §7/§8). All optional; the engine applies defaults when unset.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ContextConfigFile {
    /// How many trailing non-pinned turns are sent (default 40). Older turns
    /// are elided from the sent context (FR-3).
    pub window_turns: Option<u32>,
    /// How many trailing messages compaction keeps verbatim (default 6).
    pub keep_recent_turns: Option<u32>,
    /// Whether automatic compaction is enabled (default `true`, FR-4).
    pub auto_compact: Option<bool>,
    /// Context-usage fraction that triggers automatic compaction (default
    /// `0.85`, FR-4). Must be in `(0.0, 1.0]`.
    pub auto_compact_threshold: Option<f64>,
    /// Whether the task list is pinned in the sent context (default `true`,
    /// T-11). When `false`, the task list is not appended to the system prompt.
    pub pin_task_list: Option<bool>,
}

/// `[image]` — the `read_image` size cap (P-11, Tech Spec §5.2). All optional;
/// the engine applies the 5 MiB default when unset.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ImageConfigFile {
    /// Maximum image file size in bytes (default 5 MiB).
    pub max_bytes: Option<usize>,
}

/// `[document]` — the `read_document` size cap (P-12, Tech Spec §5.2). All
/// optional; the engine applies the 32 MiB default when unset.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct DocumentConfigFile {
    /// Maximum document file size in bytes (default 32 MiB).
    pub max_bytes: Option<usize>,
}

/// `[memory]` — persistent memory (FR-6, Tech Spec §8.1). All optional; the
/// engine applies defaults when unset.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct MemoryConfigFile {
    /// Whether the memory system is enabled (default `true`).
    pub enabled: Option<bool>,
    /// Soft warn threshold for index growth (Tech Spec §16).
    pub max_index_entries: Option<usize>,
}

/// `[skills]` — skill system (FR-7, Tech Spec §8.2). All optional; the engine
/// applies defaults when unset.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct SkillsConfigFile {
    /// Whether the skill system is enabled (default `true`).
    pub enabled: Option<bool>,
}

/// `[search]` — web-search backend (T-14, Tech Spec §5.5). Mirrors the provider
/// profile pattern (P-8): a new search service is config, not code. All
/// optional; the binary applies defaults when unset.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct SearchConfigFile {
    /// Whether the web-search tool is registered at all (default `true`; when
    /// `false` the tool is absent from `registry.specs()` — Tech Spec §5.5).
    pub enabled: Option<bool>,
    /// Response-shape parser: `"brave"` | `"tavily"` | `"searxng"` | `"json"`.
    pub adapter: Option<String>,
    /// The search service endpoint URL.
    pub endpoint: Option<String>,
    /// Authentication scheme + key reference (never an inline secret). Reuses
    /// [`AuthFile`]; the `"query"` scheme is search-side (Tech Spec §5.5).
    pub auth: Option<AuthFile>,
    /// Maximum results sent to the model (default 5, Tech Spec §5.5).
    pub max_results: Option<usize>,
}

/// `[truncate]` — tool-result reduction and size backstop at ingestion
/// (FR-2, Requirements §8.1, Tech Spec §5.3/§8). All optional; the engine
/// applies defaults when unset.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct TruncateConfigFile {
    /// Whether salient tool-result reduction runs (default `true`).
    pub reduce: Option<bool>,
    /// Truncate when output exceeds this many lines.
    pub max_lines: Option<usize>,
    /// …or this many bytes.
    pub max_bytes: Option<usize>,
    /// Lines of head to keep.
    pub head_lines: Option<usize>,
    /// Lines of tail to keep.
    pub tail_lines: Option<usize>,
}

/// A `[providers.<name>]` profile: an adapter (wire format) plus the endpoint
/// and auth to reach it, so a new provider is configuration, not code (P-8).
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
pub struct ProfileFile {
    /// Wire-format adapter: `"anthropic"` or `"openai"`.
    pub adapter: Option<String>,
    /// Endpoint base URL. The `anthropic` adapter has a built-in default; the
    /// `openai` adapter defaults to the public API base.
    pub base_url: Option<String>,
    /// Authentication scheme + key reference (never an inline secret).
    pub auth: Option<AuthFile>,
    /// Optional per-model metadata, keyed by model id
    /// (`[providers.<name>.models."model-id"]`).
    #[serde(default)]
    pub models: HashMap<String, ModelFile>,
}

/// A profile's `auth = { scheme, header?, key }`. `key` is a *reference*
/// resolved from env / `keys.toml` ([`api_key`]), never a literal secret.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
pub struct AuthFile {
    /// `"bearer"`, `"x-api-key"`, `"header"`, or `"none"`.
    pub scheme: Option<String>,
    /// Header name when `scheme = "header"`.
    pub header: Option<String>,
    /// Key reference name (e.g. `"anthropic"`, `"zai"`).
    pub key: Option<String>,
}

/// Optional per-model metadata inside a profile.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
pub struct ModelFile {
    pub context_window: Option<u32>,
    pub max_output: Option<u32>,
    pub pricing: Option<PricingEntry>,
    /// The default reasoning-effort level (P-9): `low|medium|high|max`. Its
    /// presence declares that this model has an effort control; absent ⇒ no
    /// control and the effort picker is hidden.
    pub effort: Option<String>,
    /// The effort levels this model offers, if a subset. Omitted ⇒ the full
    /// ladder (`low|medium|high|max`) when `effort` is set.
    pub effort_levels: Option<Vec<String>>,
    /// Whether the model accepts image input (P-11, Tech Spec §4.2). Default
    /// `false`; set `true` for a vision-capable model.
    pub vision: Option<bool>,
    /// Whether the model accepts document input (P-12, Tech Spec §4.2).
    /// Default `false`; set `true` for a document-capable model.
    pub documents: Option<bool>,
}

impl ProfileFile {
    /// Field-merge `higher` (higher precedence) onto `self`, so a user tier can
    /// tweak one field of a baked-in profile without respecifying the rest.
    fn merge(&mut self, higher: ProfileFile) {
        if higher.adapter.is_some() {
            self.adapter = higher.adapter;
        }
        if higher.base_url.is_some() {
            self.base_url = higher.base_url;
        }
        if let Some(higher_auth) = higher.auth {
            match &mut self.auth {
                Some(cur) => cur.merge(higher_auth),
                None => self.auth = Some(higher_auth),
            }
        }
        self.models.extend(higher.models);
    }
}

impl AuthFile {
    fn merge(&mut self, higher: AuthFile) {
        if higher.scheme.is_some() {
            self.scheme = higher.scheme;
        }
        if higher.header.is_some() {
            self.header = higher.header;
        }
        if higher.key.is_some() {
            self.key = higher.key;
        }
    }
}

/// Baked-in provider profiles (Requirements C-1) — usable out of the box; a
/// user only supplies the key. `emberly init` materializes these for editing
/// (C-2). Z.ai's concrete profile is added in Phase 1 group 4.
fn builtin_profiles() -> HashMap<String, ProfileFile> {
    let auth = |scheme: &str, key: &str| {
        Some(AuthFile {
            scheme: Some(scheme.to_string()),
            header: None,
            key: Some(key.to_string()),
        })
    };
    let profile = |adapter: &str, base_url: Option<&str>, auth: Option<AuthFile>| ProfileFile {
        adapter: Some(adapter.to_string()),
        base_url: base_url.map(str::to_string),
        auth,
        models: HashMap::new(),
    };
    HashMap::from([
        (
            "anthropic".to_string(),
            profile("anthropic", None, auth("x-api-key", "anthropic")),
        ),
        (
            "openai".to_string(),
            profile(
                "openai",
                Some("https://api.openai.com/v1"),
                auth("bearer", "openai"),
            ),
        ),
        (
            // Z.ai coding plan — OpenAI-compatible (bearer). Ready to use with
            // ZAI_API_KEY / keys.toml; the user only supplies the key.
            "zai".to_string(),
            profile(
                "openai",
                Some("https://api.z.ai/api/paas/v4"),
                auth("bearer", "zai"),
            ),
        ),
        (
            // A local OpenAI-compatible server (Ollama default); keyless.
            "local".to_string(),
            profile("openai", Some("http://localhost:11434/v1"), None),
        ),
    ])
}

/// The `[sandbox]` config section (Requirements §6.7).
#[derive(Debug, Default, Clone, Deserialize)]
pub struct SandboxConfig {
    /// `require = true` refuses to start a session without kernel confinement
    /// (default `false` — permissive but honest). Optional so a higher-precedence
    /// file can override a lower one either way.
    pub require: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct PricingEntry {
    pub input: f64,
    pub output: f64,
}

impl ConfigFile {
    fn parse(text: &str) -> anyhow::Result<Self> {
        toml::from_str(text).context("failed to parse config.toml")
    }

    /// Overlay `higher` (higher precedence) onto `self`. Profiles field-merge
    /// by name; the active `provider`/`model` selectors are replaced.
    fn merge(&mut self, higher: ConfigFile) {
        if higher.provider.is_some() {
            self.provider = higher.provider;
        }
        if higher.model.is_some() {
            self.model = higher.model;
        }
        for (name, profile) in higher.providers {
            self.providers.entry(name).or_default().merge(profile);
        }
        if higher.sandbox.require.is_some() {
            self.sandbox.require = higher.sandbox.require;
        }
        if higher.reasoning.is_some() {
            self.reasoning = higher.reasoning;
        }
        if higher.ui.tool_explanations.is_some() {
            self.ui.tool_explanations = higher.ui.tool_explanations;
        }
        if higher.ui.mouse.is_some() {
            self.ui.mouse = higher.ui.mouse;
        }
        // `[loop]` (S-5) merges normally — a project may tune the guardrail.
        if higher.loop_.enabled.is_some() {
            self.loop_.enabled = higher.loop_.enabled;
        }
        if higher.loop_.repeat_window.is_some() {
            self.loop_.repeat_window = higher.loop_.repeat_window;
        }
        if higher.loop_.max_no_progress_turns.is_some() {
            self.loop_.max_no_progress_turns = higher.loop_.max_no_progress_turns;
        }
        // `[completion]` (S-6) merges scalar fields normally; a project's
        // `[[completion.check]]` list **replaces** the whole list rather than
        // concatenating with the lower tier's, mirroring "project wins per
        // key" (a project that wants the global checks too must repeat them).
        if higher.completion.enabled.is_some() {
            self.completion.enabled = higher.completion.enabled;
        }
        if higher.completion.max_attempts.is_some() {
            self.completion.max_attempts = higher.completion.max_attempts;
        }
        if !higher.completion.check.is_empty() {
            self.completion.check = higher.completion.check;
        }
        // `[truncate]` (FR-2) merges field-by-field.
        if higher.truncate.reduce.is_some() {
            self.truncate.reduce = higher.truncate.reduce;
        }
        if higher.truncate.max_lines.is_some() {
            self.truncate.max_lines = higher.truncate.max_lines;
        }
        if higher.truncate.max_bytes.is_some() {
            self.truncate.max_bytes = higher.truncate.max_bytes;
        }
        if higher.truncate.head_lines.is_some() {
            self.truncate.head_lines = higher.truncate.head_lines;
        }
        if higher.truncate.tail_lines.is_some() {
            self.truncate.tail_lines = higher.truncate.tail_lines;
        }
        // `[context]` (FR-3) merges field-by-field.
        if higher.context.window_turns.is_some() {
            self.context.window_turns = higher.context.window_turns;
        }
        if higher.context.keep_recent_turns.is_some() {
            self.context.keep_recent_turns = higher.context.keep_recent_turns;
        }
        if higher.context.auto_compact.is_some() {
            self.context.auto_compact = higher.context.auto_compact;
        }
        if higher.context.auto_compact_threshold.is_some() {
            self.context.auto_compact_threshold = higher.context.auto_compact_threshold;
        }
        if higher.context.pin_task_list.is_some() {
            self.context.pin_task_list = higher.context.pin_task_list;
        }
        // `[image]` (P-11) merges field-by-field.
        if higher.image.max_bytes.is_some() {
            self.image.max_bytes = higher.image.max_bytes;
        }
        // `[document]` (P-12) merges field-by-field.
        if higher.document.max_bytes.is_some() {
            self.document.max_bytes = higher.document.max_bytes;
        }
        // `[memory]` (FR-6) merges field-by-field.
        if higher.memory.enabled.is_some() {
            self.memory.enabled = higher.memory.enabled;
        }
        if higher.memory.max_index_entries.is_some() {
            self.memory.max_index_entries = higher.memory.max_index_entries;
        }
        // `[skills]` (FR-7) merges field-by-field.
        if higher.skills.enabled.is_some() {
            self.skills.enabled = higher.skills.enabled;
        }
        // `[search]` (T-14) merges field-by-field, including nested auth.
        if higher.search.enabled.is_some() {
            self.search.enabled = higher.search.enabled;
        }
        if higher.search.adapter.is_some() {
            self.search.adapter = higher.search.adapter;
        }
        if higher.search.endpoint.is_some() {
            self.search.endpoint = higher.search.endpoint;
        }
        if higher.search.max_results.is_some() {
            self.search.max_results = higher.search.max_results;
        }
        if let Some(higher_auth) = higher.search.auth {
            match &mut self.search.auth {
                Some(cur) => cur.merge(higher_auth),
                None => self.search.auth = Some(higher_auth),
            }
        }
        // `[trust]` is deliberately NOT merged — it is read only from the global
        // tier (FR-1); see `global_trust_dirs` and the project-[trust] notice.
    }
}

/// The fully-resolved configuration the binary wires from.
pub struct Resolved {
    /// Active provider profile name (looked up in `providers`), or `None` to
    /// run the offline placeholder.
    pub provider: Option<String>,
    /// Active model id.
    pub model: Option<String>,
    /// All provider profiles (baked-in + user tiers, merged). `provider_setup`
    /// resolves the active one (Tech Spec §4.5).
    pub providers: HashMap<String, ProfileFile>,
    /// The system prompt = base prompt + project instructions (C-1). Always
    /// `Some` (the baked-in default at minimum).
    pub system_prompt: Option<String>,
    /// A `/compact` summarization-prompt override, or `None` for the engine
    /// default (P-7).
    pub summary_prompt: Option<String>,
    /// Every active configuration piece whose source is not the baked-in
    /// default (C-3), for `session_start` and `config show`.
    pub provenance: Vec<ConfigProvenance>,
    /// One-time notices to surface at startup (e.g. AGENTS.md over CLAUDE.md).
    pub notices: Vec<String>,
    /// `sandbox.require`: refuse to start without kernel confinement (§6.7).
    pub sandbox_require: bool,
    /// The reasoning-trail view default (`collapsed`|`expanded`|`hidden`,
    /// Design §4.4), or `None` for the built-in default (`collapsed`).
    pub reasoning: Option<String>,
    /// Whether tool-call explanations are enabled (T-9, Design §4.5). Default
    /// `true`; when `false` the schema property and prompt instruction are both
    /// omitted (no tokens spent).
    pub tool_explanations: bool,
    /// Whether pointer (mouse) interaction is enabled in the rich TUI (Design
    /// §3.4, Tech Spec §9). Default `true`; when `false` the TUI never captures
    /// the mouse, leaving native terminal selection everywhere. Degraded mode
    /// ignores this and never captures regardless (§7).
    pub mouse: bool,
    /// Resolved loop-breaking guardrail tunables (S-5), ready for the engine.
    pub loop_config: emberly_core::LoopConfig,
    /// Resolved completion-gate tunables (S-6), ready for the engine.
    pub completion_config: emberly_core::CompletionConfig,
    /// Resolved completion checks (S-6) to register at startup.
    pub completion_checks: Vec<emberly_core::CompletionCheck>,
    /// Resolved truncation/reduction config (FR-2, §8.1), ready for the engine.
    pub truncate: emberly_tools::TruncateConfig,
    /// Resolved adaptive context-window config (FR-3, Tech Spec §7/§8).
    pub context: emberly_core::ContextConfig,
    /// Resolved image size limit in bytes for `read_image` (P-11, Tech Spec
    /// §5.2). Default 5 MiB.
    pub image_max_bytes: usize,
    /// Resolved document size limit in bytes for `read_document` (P-12, Tech
    /// Spec §5.2). Default 32 MiB.
    pub document_max_bytes: usize,
    /// Resolved memory config (FR-6, Tech Spec §8.1), ready for the engine.
    pub memory: emberly_core::MemoryConfig,
    /// Resolved skills config (FR-7, Tech Spec §8.2), ready for the engine.
    pub skills: emberly_core::SkillsConfig,
    /// Resolved search config (T-14, Tech Spec §5.5). The binary conditionally
    /// registers the `web_search` tool when `enabled` and an endpoint is set.
    pub search: SearchConfig,
}

/// Resolved web-search configuration (T-14, Tech Spec §5.5). Carried in
/// [`Resolved`] for the binary composition root; the tool itself is built from
/// this when `enabled && endpoint.is_some()`.
pub struct SearchConfig {
    /// Whether the web-search tool should be registered (default `true`).
    pub enabled: bool,
    /// Response-shape parser (`brave`/`tavily`/`searxng`/`json`).
    pub adapter: Option<String>,
    /// The search service endpoint URL.
    pub endpoint: Option<String>,
    /// Auth config (key reference, not the resolved secret — the binary resolves
    /// the key at tool-build time via `config::api_key`).
    pub auth: Option<AuthFile>,
    /// Maximum results sent to the model (default 5).
    pub max_results: usize,
}

/// Command-line overrides (`--provider`/`--model`) — the highest-precedence
/// tier (Tech Spec §10).
#[derive(Default)]
pub struct CliOverrides {
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// Environment overrides (`EMBERLY_*`), read once so provenance and the final
/// value agree. Profile-internal fields (base URL, window, pricing) live in
/// `[providers.*]` now, so only the active-selection knobs remain here.
#[derive(Default)]
struct EnvOverrides {
    provider: Option<String>,
    model: Option<String>,
}

impl EnvOverrides {
    fn read() -> Self {
        Self {
            provider: std::env::var("EMBERLY_PROVIDER")
                .ok()
                .filter(|s| !s.is_empty()),
            model: std::env::var("EMBERLY_MODEL")
                .ok()
                .filter(|s| !s.is_empty()),
        }
    }
}

/// Load and resolve configuration for a session rooted at `project_root`,
/// tracking where each active value came from (C-3). Resolution order (lowest →
/// highest): baked-in defaults → global → project → `EMBERLY_*` env.
pub fn load(project_root: &Path, cli: &CliOverrides) -> anyhow::Result<Resolved> {
    let global = read_config(global_config_path().as_deref())?;
    let project = read_config(Some(&project_root.join(".agents").join("config.toml")))?;
    let env = EnvOverrides::read();

    // Baked-in profiles are the lowest tier (C-1); user tiers field-merge onto
    // them, then env, then CLI select the active profile/model.
    let mut merged = ConfigFile {
        providers: builtin_profiles(),
        ..ConfigFile::default()
    };
    if let Some(g) = &global {
        merged.merge(g.clone());
    }
    if let Some(p) = &project {
        merged.merge(p.clone());
    }
    if let Some(v) = &env.provider {
        merged.provider = Some(v.clone());
    }
    if let Some(v) = &env.model {
        merged.model = Some(v.clone());
    }
    if let Some(p) = &cli.provider {
        merged.provider = Some(p.clone());
    }
    if let Some(m) = &cli.model {
        merged.model = Some(m.clone());
    }

    let mut provenance = Vec::new();
    // Source for each selector: cli > env > project > global > default.
    let src = |cli_set: bool, env_set: bool, project_has: bool, global_has: bool| {
        source_of(cli_set, env_set, project_has, global_has)
    };
    record(
        &mut provenance,
        "provider",
        src(
            cli.provider.is_some(),
            env.provider.is_some(),
            field(&project, |c| c.provider.is_some()),
            field(&global, |c| c.provider.is_some()),
        ),
        merged.provider.is_some(),
    );
    record(
        &mut provenance,
        "model",
        src(
            cli.model.is_some(),
            env.model.is_some(),
            field(&project, |c| c.model.is_some()),
            field(&global, |c| c.model.is_some()),
        ),
        merged.model.is_some(),
    );

    // Prompts (P-7): per-model-family variant, then the plain name, then the
    // baked-in default. The family comes from the resolved model.
    let prompts_dir = project_root.join(".agents").join("prompts");
    let family = family_of(merged.model.as_deref().unwrap_or_default());

    let (system_base, system_src) = load_prompt(&prompts_dir, "system", family).map_or_else(
        || (emberly_core::prompts::system().to_string(), "default"),
        |text| (text, "project"),
    );
    record(
        &mut provenance,
        "system_prompt",
        system_src.to_string(),
        true,
    );

    let (summary_prompt, summary_src) = match load_prompt(&prompts_dir, "compact", family) {
        Some(text) => (Some(text), "project"),
        None => (None, "default"),
    };
    if summary_src != "default" {
        record(
            &mut provenance,
            "compact_prompt",
            summary_src.to_string(),
            true,
        );
    }

    // Tool-call explanations (T-9): on unless a user tier turned it off; record
    // provenance only on a deviation from the default (speech about deviations).
    record(
        &mut provenance,
        "tool_explanations",
        source_of(
            false,
            false,
            field(&project, |c| c.ui.tool_explanations.is_some()),
            field(&global, |c| c.ui.tool_explanations.is_some()),
        ),
        merged.ui.tool_explanations.is_some(),
    );

    // Mouse interaction (Design §3.4): on unless a user tier turned it off;
    // record provenance only on a deviation from the default `true`.
    record(
        &mut provenance,
        "ui.mouse",
        source_of(
            false,
            false,
            field(&project, |c| c.ui.mouse.is_some()),
            field(&global, |c| c.ui.mouse.is_some()),
        ),
        merged.ui.mouse.is_some(),
    );

    // Completion gate (S-6): record provenance when a user tier sets any
    // `[completion]` scalar field, plus a count line when checks are
    // registered — closing the `[loop]` provenance gap (a pre-existing gap
    // this phase deliberately does not repeat for `[completion]`).
    if field(&project, |c: &ConfigFile| c.completion.enabled.is_some())
        || field(&global, |c: &ConfigFile| c.completion.enabled.is_some())
    {
        record(
            &mut provenance,
            "completion.enabled",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| c.completion.enabled.is_some()),
                field(&global, |c: &ConfigFile| c.completion.enabled.is_some()),
            ),
            true,
        );
    }
    if field(&project, |c: &ConfigFile| {
        c.completion.max_attempts.is_some()
    }) || field(&global, |c: &ConfigFile| {
        c.completion.max_attempts.is_some()
    }) {
        record(
            &mut provenance,
            "completion.max_attempts",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| {
                    c.completion.max_attempts.is_some()
                }),
                field(&global, |c: &ConfigFile| {
                    c.completion.max_attempts.is_some()
                }),
            ),
            true,
        );
    }
    if !merged.completion.check.is_empty() {
        record(
            &mut provenance,
            "completion.check",
            format!("{} registered", merged.completion.check.len()),
            true,
        );
    }

    // Truncation/reduction (FR-2): record provenance when a user tier sets any
    // `[truncate]` field (speech about deviations from the baked-in defaults).
    if field(&project, |c| c.truncate.reduce.is_some())
        || field(&global, |c| c.truncate.reduce.is_some())
    {
        record(
            &mut provenance,
            "truncate.reduce",
            source_of(
                false,
                false,
                field(&project, |c| c.truncate.reduce.is_some()),
                field(&global, |c| c.truncate.reduce.is_some()),
            ),
            true,
        );
    }

    // Context window + compaction (FR-3): record provenance when a user tier
    // sets any `[context]` field.
    if field(&project, |c: &ConfigFile| c.context.window_turns.is_some())
        || field(&global, |c: &ConfigFile| c.context.window_turns.is_some())
    {
        record(
            &mut provenance,
            "context.window_turns",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| c.context.window_turns.is_some()),
                field(&global, |c: &ConfigFile| c.context.window_turns.is_some()),
            ),
            true,
        );
    }
    if field(&project, |c: &ConfigFile| {
        c.context.keep_recent_turns.is_some()
    }) || field(&global, |c: &ConfigFile| {
        c.context.keep_recent_turns.is_some()
    }) {
        record(
            &mut provenance,
            "context.keep_recent_turns",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| {
                    c.context.keep_recent_turns.is_some()
                }),
                field(&global, |c: &ConfigFile| {
                    c.context.keep_recent_turns.is_some()
                }),
            ),
            true,
        );
    }
    if field(&project, |c: &ConfigFile| c.context.auto_compact.is_some())
        || field(&global, |c: &ConfigFile| c.context.auto_compact.is_some())
    {
        record(
            &mut provenance,
            "context.auto_compact",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| c.context.auto_compact.is_some()),
                field(&global, |c: &ConfigFile| c.context.auto_compact.is_some()),
            ),
            true,
        );
    }
    if field(&project, |c: &ConfigFile| {
        c.context.auto_compact_threshold.is_some()
    }) || field(&global, |c: &ConfigFile| {
        c.context.auto_compact_threshold.is_some()
    }) {
        record(
            &mut provenance,
            "context.auto_compact_threshold",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| {
                    c.context.auto_compact_threshold.is_some()
                }),
                field(&global, |c: &ConfigFile| {
                    c.context.auto_compact_threshold.is_some()
                }),
            ),
            true,
        );
    }
    if field(&project, |c: &ConfigFile| c.context.pin_task_list.is_some())
        || field(&global, |c: &ConfigFile| c.context.pin_task_list.is_some())
    {
        record(
            &mut provenance,
            "context.pin_task_list",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| c.context.pin_task_list.is_some()),
                field(&global, |c: &ConfigFile| c.context.pin_task_list.is_some()),
            ),
            true,
        );
    }

    // Image size cap (P-11): record provenance when a user tier sets
    // `[image] max_bytes` (speech about deviations, silent on the default).
    if field(&project, |c: &ConfigFile| c.image.max_bytes.is_some())
        || field(&global, |c: &ConfigFile| c.image.max_bytes.is_some())
    {
        record(
            &mut provenance,
            "image.max_bytes",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| c.image.max_bytes.is_some()),
                field(&global, |c: &ConfigFile| c.image.max_bytes.is_some()),
            ),
            true,
        );
    }

    // Document size cap (P-12): record provenance when a user tier sets
    // `[document] max_bytes` (speech about deviations, silent on the default).
    if field(&project, |c: &ConfigFile| c.document.max_bytes.is_some())
        || field(&global, |c: &ConfigFile| c.document.max_bytes.is_some())
    {
        record(
            &mut provenance,
            "document.max_bytes",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| c.document.max_bytes.is_some()),
                field(&global, |c: &ConfigFile| c.document.max_bytes.is_some()),
            ),
            true,
        );
    }

    // Memory (FR-6): record provenance when a user tier sets any `[memory]`
    // field.
    if field(&project, |c: &ConfigFile| c.memory.enabled.is_some())
        || field(&global, |c: &ConfigFile| c.memory.enabled.is_some())
    {
        record(
            &mut provenance,
            "memory.enabled",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| c.memory.enabled.is_some()),
                field(&global, |c: &ConfigFile| c.memory.enabled.is_some()),
            ),
            true,
        );
    }
    if field(&project, |c: &ConfigFile| {
        c.memory.max_index_entries.is_some()
    }) || field(&global, |c: &ConfigFile| {
        c.memory.max_index_entries.is_some()
    }) {
        record(
            &mut provenance,
            "memory.max_index_entries",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| {
                    c.memory.max_index_entries.is_some()
                }),
                field(&global, |c: &ConfigFile| {
                    c.memory.max_index_entries.is_some()
                }),
            ),
            true,
        );
    }

    // Skills (FR-7): record provenance when a user tier sets `[skills] enabled`.
    if field(&project, |c: &ConfigFile| c.skills.enabled.is_some())
        || field(&global, |c: &ConfigFile| c.skills.enabled.is_some())
    {
        record(
            &mut provenance,
            "skills.enabled",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| c.skills.enabled.is_some()),
                field(&global, |c: &ConfigFile| c.skills.enabled.is_some()),
            ),
            true,
        );
    }

    // Search (T-14): record provenance when a user tier sets any `[search]`
    // field (speech about deviations from the defaults).
    if field(&project, |c: &ConfigFile| c.search.enabled.is_some())
        || field(&global, |c: &ConfigFile| c.search.enabled.is_some())
    {
        record(
            &mut provenance,
            "search.enabled",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| c.search.enabled.is_some()),
                field(&global, |c: &ConfigFile| c.search.enabled.is_some()),
            ),
            true,
        );
    }
    if field(&project, |c: &ConfigFile| c.search.adapter.is_some())
        || field(&global, |c: &ConfigFile| c.search.adapter.is_some())
    {
        record(
            &mut provenance,
            "search.adapter",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| c.search.adapter.is_some()),
                field(&global, |c: &ConfigFile| c.search.adapter.is_some()),
            ),
            true,
        );
    }
    if field(&project, |c: &ConfigFile| c.search.endpoint.is_some())
        || field(&global, |c: &ConfigFile| c.search.endpoint.is_some())
    {
        record(
            &mut provenance,
            "search.endpoint",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| c.search.endpoint.is_some()),
                field(&global, |c: &ConfigFile| c.search.endpoint.is_some()),
            ),
            true,
        );
    }
    if field(&project, |c: &ConfigFile| c.search.max_results.is_some())
        || field(&global, |c: &ConfigFile| c.search.max_results.is_some())
    {
        record(
            &mut provenance,
            "search.max_results",
            source_of(
                false,
                false,
                field(&project, |c: &ConfigFile| c.search.max_results.is_some()),
                field(&global, |c: &ConfigFile| c.search.max_results.is_some()),
            ),
            true,
        );
    }

    // Project instructions (C-1): AGENTS.md native; CLAUDE.md as a fallback;
    // both present → AGENTS.md wins with a notice.
    let mut notices = Vec::new();
    // A project `[trust]` is ignored — trust is global-only so a repo can't
    // pre-declare itself trusted (FR-1). Say so rather than silently dropping it.
    if project
        .as_ref()
        .is_some_and(|p| !p.trust.trusted_dirs.is_empty())
    {
        notices.push(
            "ignoring [trust] in project config — workspace trust is global-only (FR-1)".into(),
        );
    }
    let mut system_prompt = system_base;
    if let Some((instructions, source, notice)) = load_project_instructions(project_root)? {
        system_prompt.push_str("\n\n# Project instructions\n\n");
        system_prompt.push_str(&instructions);
        record(&mut provenance, "project_instructions", source, true);
        if let Some(notice) = notice {
            notices.push(notice);
        }
    }

    Ok(Resolved {
        provider: merged.provider,
        model: merged.model,
        providers: merged.providers,
        system_prompt: Some(system_prompt),
        summary_prompt,
        provenance,
        notices,
        sandbox_require: merged.sandbox.require.unwrap_or(false),
        reasoning: merged.reasoning,
        tool_explanations: merged.ui.tool_explanations.unwrap_or(true),
        mouse: merged.ui.mouse.unwrap_or(true),
        loop_config: {
            // Override only the fields the user set; the engine owns the
            // defaults (S-5, Tech Spec §7 — "initial; tune with use").
            let d = emberly_core::LoopConfig::default();
            emberly_core::LoopConfig {
                enabled: merged.loop_.enabled.unwrap_or(d.enabled),
                repeat_window: merged
                    .loop_
                    .repeat_window
                    .map_or(d.repeat_window, |v| v as usize),
                max_no_progress_turns: merged
                    .loop_
                    .max_no_progress_turns
                    .map_or(d.max_no_progress_turns, |v| v as usize),
            }
        },
        completion_config: {
            let d = emberly_core::CompletionConfig::default();
            emberly_core::CompletionConfig {
                enabled: merged.completion.enabled.unwrap_or(d.enabled),
                max_attempts: merged.completion.max_attempts.unwrap_or(d.max_attempts),
            }
        },
        completion_checks: merged
            .completion
            .check
            .iter()
            .map(|c| emberly_core::CompletionCheck {
                name: c.name.clone(),
                command: c.command.clone(),
                expect_exit: c.expect_exit.unwrap_or(0),
            })
            .collect(),
        truncate: {
            let d = emberly_tools::TruncateConfig::default();
            emberly_tools::TruncateConfig {
                reduce: merged.truncate.reduce.unwrap_or(d.reduce),
                max_lines: merged.truncate.max_lines.unwrap_or(d.max_lines),
                max_bytes: merged.truncate.max_bytes.unwrap_or(d.max_bytes),
                head_lines: merged.truncate.head_lines.unwrap_or(d.head_lines),
                tail_lines: merged.truncate.tail_lines.unwrap_or(d.tail_lines),
            }
        },
        context: {
            let d = emberly_core::ContextConfig::default();
            let threshold = merged
                .context
                .auto_compact_threshold
                .unwrap_or(d.auto_compact_threshold);
            if threshold <= 0.0 || threshold > 1.0 {
                anyhow::bail!(
                    "context.auto_compact_threshold must be in (0.0, 1.0], got {threshold}"
                );
            }
            emberly_core::ContextConfig {
                window_turns: merged
                    .context
                    .window_turns
                    .map_or(d.window_turns, |v| v as usize),
                keep_recent_turns: merged
                    .context
                    .keep_recent_turns
                    .map_or(d.keep_recent_turns, |v| v as usize),
                auto_compact: merged.context.auto_compact.unwrap_or(d.auto_compact),
                auto_compact_threshold: threshold,
                pin_task_list: merged.context.pin_task_list.unwrap_or(d.pin_task_list),
            }
        },
        image_max_bytes: merged.image.max_bytes.unwrap_or(5 * 1024 * 1024),
        document_max_bytes: merged.document.max_bytes.unwrap_or(32 * 1024 * 1024),
        memory: {
            let d = emberly_core::MemoryConfig::default();
            emberly_core::MemoryConfig {
                enabled: merged.memory.enabled.unwrap_or(d.enabled),
                max_index_entries: merged
                    .memory
                    .max_index_entries
                    .unwrap_or(d.max_index_entries),
            }
        },
        skills: {
            let d = emberly_core::SkillsConfig::default();
            emberly_core::SkillsConfig {
                enabled: merged.skills.enabled.unwrap_or(d.enabled),
            }
        },
        search: SearchConfig {
            enabled: merged.search.enabled.unwrap_or(true),
            adapter: merged.search.adapter,
            endpoint: merged.search.endpoint,
            auth: merged.search.auth,
            max_results: merged.search.max_results.unwrap_or(5),
        },
    })
}

/// Load permission rules from the global and project `permissions.toml` files
/// (Requirements §6.1, precedence order global → project). A malformed file is
/// skipped with a warning, never silently applied — a broken rules file must
/// not quietly widen *or* narrow access.
pub fn load_permission_rules(project_root: &Path) -> (Vec<Rule>, Vec<String>) {
    let mut rules = Vec::new();
    let mut warnings = Vec::new();
    let mut load = |path: Option<PathBuf>, source: RuleSource| {
        let Some(path) = path else { return };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        match parse_rules(&text, source) {
            Ok(mut parsed) => rules.append(&mut parsed),
            Err(error) => warnings.push(format!("ignoring {}: {error}", path.display())),
        }
    };
    load(global_permissions_path(), RuleSource::Global);
    load(
        Some(project_root.join(".agents").join("permissions.toml")),
        RuleSource::Project,
    );
    (rules, warnings)
}

fn global_permissions_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("permissions.toml"))
}

/// Read a config file if it exists.
fn read_config(path: Option<&Path>) -> anyhow::Result<Option<ConfigFile>> {
    let Some(path) = path else { return Ok(None) };
    if !path.exists() {
        return Ok(None);
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(ConfigFile::parse(&text)?))
}

/// `true` if `config` is present and the field predicate holds.
fn field(config: &Option<ConfigFile>, has: impl Fn(&ConfigFile) -> bool) -> bool {
    config.as_ref().is_some_and(has)
}

/// The winning tier's label for a field.
fn source_of(cli: bool, env: bool, project: bool, global: bool) -> String {
    if cli {
        "cli"
    } else if env {
        "env"
    } else if project {
        "project:.agents/config.toml"
    } else if global {
        "global:~/.config/emberly/config.toml"
    } else {
        "default"
    }
    .to_string()
}

/// Record a provenance entry when the value is active and non-default.
fn record(provenance: &mut Vec<ConfigProvenance>, piece: &str, source: String, active: bool) {
    if active && source != "default" {
        provenance.push(ConfigProvenance {
            piece: piece.to_string(),
            source,
        });
    }
}

/// The model family for prompt-variant resolution (P-7).
fn family_of(model: &str) -> &'static str {
    let m = model.to_ascii_lowercase();
    if m.contains("claude") {
        "claude"
    } else if m.contains("gpt") || m.contains("o1") || m.contains("o3") {
        "gpt"
    } else {
        "generic"
    }
}

/// Resolve a prompt: `<name>.<family>.md`, then `<name>.md`, else `None`
/// (caller uses the built-in default). P-7.
fn load_prompt(prompts_dir: &Path, name: &str, family: &str) -> Option<String> {
    for candidate in [format!("{name}.{family}.md"), format!("{name}.md")] {
        if let Ok(text) = std::fs::read_to_string(prompts_dir.join(&candidate)) {
            let text = text.trim().to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

/// Load project instructions (C-1): returns `(text, source, notice)`.
/// `AGENTS.md` is native; `CLAUDE.md` is read only when `AGENTS.md` is absent;
/// with both present, `AGENTS.md` wins and a notice is returned.
fn load_project_instructions(
    root: &Path,
) -> anyhow::Result<Option<(String, String, Option<String>)>> {
    let agents = root.join("AGENTS.md");
    let claude = root.join("CLAUDE.md");
    let read = |p: &Path| std::fs::read_to_string(p).map(|t| t.trim().to_string());

    if agents.exists() {
        let text = read(&agents).with_context(|| format!("reading {}", agents.display()))?;
        let notice = claude.exists().then(|| {
            "using AGENTS.md for project instructions (CLAUDE.md is present but ignored)"
                .to_string()
        });
        return Ok((!text.is_empty()).then_some((text, "AGENTS.md".to_string(), notice)));
    }
    if claude.exists() {
        let text = read(&claude).with_context(|| format!("reading {}", claude.display()))?;
        return Ok((!text.is_empty()).then_some((text, "CLAUDE.md".to_string(), None)));
    }
    Ok(None)
}

/// `emberly config show`: print every active piece with its provenance tier
/// (C-3). Secrets are reported as set/unset, never printed.
pub fn show(project_root: &Path) -> anyhow::Result<()> {
    let resolved = load(project_root, &CliOverrides::default())?;
    println!(
        "emberly configuration (project: {})",
        project_root.display()
    );
    println!();
    print_value("active provider", resolved.provider.as_deref());
    print_value("active model", resolved.model.as_deref());

    println!();
    println!("provider profiles (name → adapter, endpoint, key):");
    let mut names: Vec<&String> = resolved.providers.keys().collect();
    names.sort();
    for name in names {
        let p = &resolved.providers[name];
        let adapter = p.adapter.as_deref().unwrap_or("(unset)");
        let base = p.base_url.as_deref().unwrap_or("(adapter default)");
        let key_ref = p.auth.as_ref().and_then(|a| a.key.as_deref());
        let key = key_ref.map_or_else(
            || "no key".to_string(),
            |r| format!("key '{r}' {}", key_status(r)),
        );
        let active = if resolved.provider.as_deref() == Some(name) {
            " (active)"
        } else {
            ""
        };
        println!("  {name}{active}: {adapter} @ {base}, {key}");
    }

    println!();
    println!("prompts & instructions:");
    for piece in ["system_prompt", "compact_prompt", "project_instructions"] {
        let source = resolved
            .provenance
            .iter()
            .find(|p| p.piece == piece)
            .map_or("default", |p| p.source.as_str());
        println!("  {piece}: {source}");
    }
    if !resolved.provenance.is_empty() {
        println!();
        println!("overrides (non-default sources):");
        for entry in &resolved.provenance {
            println!("  {} ← {}", entry.piece, entry.source);
        }
    }

    // Skill shadow notices (FR-7, Tech Spec §8.2): when a project skill
    // shadows a user-global skill of the same name, surface it so the override
    // is visible, not silent. Discovery scans project skills only when a
    // project skills dir exists (always trusted here — `config show` runs after
    // the trust gate, or the user invoked it explicitly).
    if resolved.skills.enabled {
        if let (Some(user_dir), Some(project_dir)) = (
            skills_dir(),
            Some(project_root.join(".agents").join("skills")),
        ) {
            let catalog = emberly_core::skills::SkillCatalog::new(user_dir, Some(project_dir));
            let (_, shadows) = catalog.discover();
            if !shadows.is_empty() {
                println!();
                println!("skill overrides (project shadows user-global):");
                for shadow in &shadows {
                    println!("  skill `{}`: project shadows user-global", shadow.name);
                }
            }
        }
    }

    // Web search (T-14, Tech Spec §5.5): show adapter, endpoint, and key status
    // — never the key itself (mirror the provider profile block above).
    println!();
    println!("web search:");
    if !resolved.search.enabled {
        println!("  disabled (search.enabled = false)");
    } else {
        let adapter = resolved.search.adapter.as_deref().unwrap_or("(unset)");
        let endpoint = resolved
            .search
            .endpoint
            .as_deref()
            .unwrap_or("(unset — tool not registered)");
        let key_ref = resolved.search.auth.as_ref().and_then(|a| a.key.as_deref());
        let key = key_ref.map_or_else(
            || "no key".to_string(),
            |r| format!("key '{r}' {}", key_status(r)),
        );
        println!("  adapter: {adapter}");
        println!("  endpoint: {endpoint}");
        println!("  {key}");
        let registered = resolved.search.enabled && resolved.search.endpoint.is_some();
        println!(
            "  status: {}",
            if registered {
                "registered"
            } else {
                "not registered"
            }
        );
    }

    Ok(())
}

fn print_value(name: &str, value: Option<&str>) {
    println!("  {name:<18} {}", value.unwrap_or("(unset)"));
}

/// Whether the key for `reference` is resolvable (never prints the secret).
fn key_status(reference: &str) -> &'static str {
    match api_key(reference) {
        Ok(Some(_)) => "set",
        _ => "unset",
    }
}

/// Resolve the API key for a key *reference* (a profile's `auth.key`, e.g.
/// `"anthropic"`, `"openai"`, `"zai"`): the environment variable
/// `<REF>_API_KEY` first, then the matching entry in `keys.toml` (which must
/// be `0600`). Returns `None` if unset. `keys.toml` is a flat table of
/// `ref = "secret"`, never sourced from project config.
pub fn api_key(reference: &str) -> anyhow::Result<Option<String>> {
    let env_var = format!("{}_API_KEY", reference.to_ascii_uppercase());
    if let Ok(key) = std::env::var(&env_var) {
        if !key.is_empty() {
            return Ok(Some(key));
        }
    }

    let Some(path) = global_keys_path() else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    enforce_private_permissions(&path)?;
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let keys: HashMap<String, String> =
        toml::from_str(&text).context("failed to parse keys.toml")?;
    Ok(keys.get(reference).filter(|s| !s.is_empty()).cloned())
}

fn config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("emberly"));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(|home| PathBuf::from(home).join(".config").join("emberly"))
}

fn global_config_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

fn global_keys_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("keys.toml"))
}

/// The global workspace-trust store (FR-1, Tech Spec §6.7). Global-only by
/// construction — `config_dir()` never consults the project root.
#[must_use]
pub fn global_trust_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("trust.toml"))
}

/// The user-global memory directory (FR-6, Tech Spec §8.1):
/// `~/.config/emberly/memory/`. Always loaded when a home directory exists.
#[must_use]
pub fn memory_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("memory"))
}

/// The user-global skills directory (FR-7, Tech Spec §8.2):
/// `~/.config/emberly/skills/`. Always loaded when a home directory exists.
#[must_use]
pub fn skills_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("skills"))
}

/// The `trust.trusted_dirs` pre-trust allowlist, read **only** from the global
/// config tier (FR-1 — project config cannot contribute). Missing/unparyable
/// global config yields an empty list; the gate then relies on the store alone.
#[must_use]
pub fn global_trust_dirs() -> Vec<String> {
    let Some(path) = global_config_path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    ConfigFile::parse(&text)
        .map(|c| c.trust.trusted_dirs)
        .unwrap_or_default()
}

/// Refuse a secrets/trust file readable by group/other (Tech Spec §8, §6.7).
#[cfg(unix)]
pub(crate) fn enforce_private_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)
        .with_context(|| format!("reading permissions of {}", path.display()))?
        .permissions()
        .mode();
    if mode & 0o077 != 0 {
        bail!(
            "{} is group/world-accessible (mode {:o}); run `chmod 600 {}` — \
             secret keys must be private",
            path.display(),
            mode & 0o777,
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn enforce_private_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("emberly-cfg-{}-{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn family_maps_models_to_prompt_variants() {
        assert_eq!(family_of("claude-sonnet-5"), "claude");
        assert_eq!(family_of("gpt-5.2"), "gpt");
        assert_eq!(family_of("o3-mini"), "gpt");
        assert_eq!(family_of("llama-3"), "generic");
    }

    #[test]
    fn agents_md_wins_over_claude_md_with_a_notice() {
        let dir = tmp();
        std::fs::write(dir.join("AGENTS.md"), "agents rules").unwrap();
        std::fs::write(dir.join("CLAUDE.md"), "claude rules").unwrap();
        let (text, source, notice) = load_project_instructions(&dir).unwrap().unwrap();
        assert_eq!(text, "agents rules");
        assert_eq!(source, "AGENTS.md");
        assert!(notice.is_some(), "a notice explains CLAUDE.md is ignored");
    }

    #[test]
    fn claude_md_is_the_fallback() {
        let dir = tmp();
        std::fs::write(dir.join("CLAUDE.md"), "claude rules").unwrap();
        let (text, source, notice) = load_project_instructions(&dir).unwrap().unwrap();
        assert_eq!(text, "claude rules");
        assert_eq!(source, "CLAUDE.md");
        assert!(notice.is_none());
        // No instructions at all → None.
        assert!(load_project_instructions(&tmp()).unwrap().is_none());
    }

    #[test]
    fn prompt_prefers_family_variant_then_falls_back() {
        let dir = tmp();
        let prompts = dir.join("prompts");
        std::fs::create_dir_all(&prompts).unwrap();
        std::fs::write(prompts.join("system.claude.md"), "claude system").unwrap();
        std::fs::write(prompts.join("system.md"), "generic system").unwrap();
        assert_eq!(
            load_prompt(&prompts, "system", "claude").as_deref(),
            Some("claude system")
        );
        assert_eq!(
            load_prompt(&prompts, "system", "gpt").as_deref(),
            Some("generic system")
        );
        assert_eq!(load_prompt(&prompts, "missing", "claude"), None);
    }

    #[test]
    fn load_records_project_provenance_and_builds_system_prompt() {
        let dir = tmp();
        let agents = dir.join(".agents");
        std::fs::create_dir_all(&agents).unwrap();
        // Select a profile in the project tier and field-merge a base_url onto
        // the baked-in `openai` profile.
        std::fs::write(
            agents.join("config.toml"),
            "provider = \"openai\"\nmodel = \"llama\"\n\
             [providers.openai]\nbase_url = \"http://localhost:1234/v1\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("AGENTS.md"), "be careful").unwrap();

        let resolved = load(&dir, &CliOverrides::default()).unwrap();
        // The active profile selector came from the project tier.
        assert!(resolved
            .provenance
            .iter()
            .any(|p| p.piece == "provider" && p.source.starts_with("project")));
        // Field-merge kept the baked-in adapter and applied the project base_url.
        let openai = resolved.providers.get("openai").expect("openai profile");
        assert_eq!(openai.adapter.as_deref(), Some("openai"));
        assert_eq!(openai.base_url.as_deref(), Some("http://localhost:1234/v1"));
        assert!(resolved
            .provenance
            .iter()
            .any(|p| p.piece == "project_instructions" && p.source == "AGENTS.md"));
        // The system prompt is the default plus the project instructions.
        let system = resolved.system_prompt.unwrap();
        assert!(system.contains("Emberly Code"));
        assert!(system.contains("be careful"));
    }

    #[test]
    fn parses_provider_profile_with_model_metadata() {
        let text = r#"
            provider = "zai"
            model = "glm-x"
            [providers.zai]
            adapter = "anthropic"
            base_url = "https://example.test/api"
            auth = { scheme = "x-api-key", key = "zai" }
            [providers.zai.models."glm-x"]
            context_window = 128000
            max_output = 8192
            pricing = { input = 1.0, output = 2.0 }
        "#;
        let config = ConfigFile::parse(text).expect("parse");
        let zai = config.providers.get("zai").expect("zai profile");
        assert_eq!(zai.adapter.as_deref(), Some("anthropic"));
        assert_eq!(zai.base_url.as_deref(), Some("https://example.test/api"));
        let auth = zai.auth.as_ref().expect("auth");
        assert_eq!(auth.scheme.as_deref(), Some("x-api-key"));
        assert_eq!(auth.key.as_deref(), Some("zai"));
        let meta = zai.models.get("glm-x").expect("model meta");
        assert_eq!(meta.context_window, Some(128_000));
        assert_eq!(meta.max_output, Some(8_192));
        let pricing = meta.pricing.expect("pricing");
        assert!((pricing.input - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn builtin_profiles_present_and_field_merge() {
        let mut base = ConfigFile {
            providers: builtin_profiles(),
            ..ConfigFile::default()
        };
        for name in ["anthropic", "openai", "local"] {
            assert!(base.providers.contains_key(name), "baked-in: {name}");
        }
        // A user tier tweaks one field; the rest of the baked-in profile stays.
        let higher =
            ConfigFile::parse("[providers.openai]\nbase_url = \"http://x/v1\"").expect("higher");
        base.merge(higher);
        let openai = base.providers.get("openai").expect("openai");
        assert_eq!(openai.adapter.as_deref(), Some("openai")); // preserved
        assert_eq!(openai.base_url.as_deref(), Some("http://x/v1")); // overridden
    }

    #[test]
    fn merge_replaces_active_selectors() {
        let mut base = ConfigFile::parse("provider = \"openai\"\nmodel = \"a\"").expect("base");
        let higher = ConfigFile::parse("model = \"b\"").expect("higher");
        base.merge(higher);
        assert_eq!(base.provider.as_deref(), Some("openai")); // untouched
        assert_eq!(base.model.as_deref(), Some("b")); // overridden
    }

    #[test]
    fn ui_tool_explanations_parses_and_merges() {
        // Absent unless set (resolves to the `true` default in `load`).
        assert_eq!(
            ConfigFile::parse("").expect("empty").ui.tool_explanations,
            None
        );
        // Parsed from the `[ui]` table.
        let off = ConfigFile::parse("[ui]\ntool_explanations = false").expect("ui");
        assert_eq!(off.ui.tool_explanations, Some(false));
        // A higher tier overrides a lower one.
        let mut base = ConfigFile::parse("[ui]\ntool_explanations = true").expect("base");
        base.merge(off);
        assert_eq!(base.ui.tool_explanations, Some(false));
    }

    #[test]
    fn ui_mouse_parses_and_merges() {
        // Absent unless set (resolves to the `true` default in `load`, Design §3.4).
        assert_eq!(ConfigFile::parse("").expect("empty").ui.mouse, None);
        // Parsed from the `[ui]` table.
        let off = ConfigFile::parse("[ui]\nmouse = false").expect("ui");
        assert_eq!(off.ui.mouse, Some(false));
        // A higher tier overrides a lower one, independently of tool_explanations.
        let mut base =
            ConfigFile::parse("[ui]\nmouse = true\ntool_explanations = true").expect("base");
        base.merge(off);
        assert_eq!(base.ui.mouse, Some(false));
        assert_eq!(base.ui.tool_explanations, Some(true)); // untouched
    }

    #[test]
    fn trust_parses_but_is_not_merged_global_only() {
        // A `[trust]` section parses into the file struct…
        let cfg = ConfigFile::parse("[trust]\ntrusted_dirs = [\"/a\", \"~/code\"]").expect("trust");
        assert_eq!(cfg.trust.trusted_dirs, vec!["/a", "~/code"]);
        // …but merge does NOT carry it, so a project tier can never contribute
        // trust (FR-1 — a repo can't self-trust). The gate reads global only.
        let mut base = ConfigFile::default();
        base.merge(cfg);
        assert!(
            base.trust.trusted_dirs.is_empty(),
            "trust is never merged from a higher tier"
        );
    }

    #[test]
    fn loop_config_parses_and_merges() {
        let cfg = ConfigFile::parse("[loop]\nenabled = false\nrepeat_window = 5").expect("loop");
        assert_eq!(cfg.loop_.enabled, Some(false));
        assert_eq!(cfg.loop_.repeat_window, Some(5));
        // Unlike trust, [loop] merges (a project may tune the guardrail).
        let mut base = ConfigFile::default();
        base.merge(cfg);
        assert_eq!(base.loop_.enabled, Some(false));
        assert_eq!(base.loop_.repeat_window, Some(5));
    }

    #[test]
    fn completion_config_parses_and_merges_scalars() {
        let cfg = ConfigFile::parse(
            "[completion]\nenabled = false\nmax_attempts = 5\n\
             [[completion.check]]\nname = \"tests\"\ncommand = \"cargo test\"\n",
        )
        .expect("completion");
        assert_eq!(cfg.completion.enabled, Some(false));
        assert_eq!(cfg.completion.max_attempts, Some(5));
        assert_eq!(cfg.completion.check.len(), 1);
        assert_eq!(cfg.completion.check[0].name, "tests");
        assert_eq!(cfg.completion.check[0].expect_exit, None);

        let mut base = ConfigFile::default();
        base.merge(cfg);
        assert_eq!(base.completion.enabled, Some(false));
        assert_eq!(base.completion.max_attempts, Some(5));
        assert_eq!(base.completion.check.len(), 1);
    }

    #[test]
    fn completion_check_list_replaces_rather_than_concatenates_on_merge() {
        // A project's [[completion.check]] list replaces the lower tier's
        // whole list (mirrors "project wins per key") rather than
        // concatenating global + project checks.
        let global = ConfigFile::parse(
            "[[completion.check]]\nname = \"global-check\"\ncommand = \"true\"\n",
        )
        .expect("global");
        let project = ConfigFile::parse(
            "[[completion.check]]\nname = \"project-check\"\ncommand = \"true\"\n",
        )
        .expect("project");

        let mut merged = ConfigFile::default();
        merged.merge(global);
        merged.merge(project);
        assert_eq!(merged.completion.check.len(), 1);
        assert_eq!(merged.completion.check[0].name, "project-check");
    }

    #[test]
    fn completion_gate_resolves_defaults_and_checks_with_provenance() {
        let dir = tmp();
        let agents = dir.join(".agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("config.toml"),
            "[completion]\nmax_attempts = 5\n\
             [[completion.check]]\nname = \"tests\"\ncommand = \"cargo test\"\n",
        )
        .unwrap();

        let resolved = load(&dir, &CliOverrides::default()).unwrap();
        // `enabled` defaults to true (unset), `max_attempts` came from project.
        assert!(resolved.completion_config.enabled);
        assert_eq!(resolved.completion_config.max_attempts, 5);
        assert_eq!(resolved.completion_checks.len(), 1);
        assert_eq!(resolved.completion_checks[0].name, "tests");
        assert_eq!(resolved.completion_checks[0].command, "cargo test");
        // expect_exit defaults to 0 when unset in the file.
        assert_eq!(resolved.completion_checks[0].expect_exit, 0);

        assert!(resolved
            .provenance
            .iter()
            .any(|p| p.piece == "completion.max_attempts" && p.source.starts_with("project")));
        assert!(resolved
            .provenance
            .iter()
            .any(|p| p.piece == "completion.check" && p.source == "1 registered"));
    }

    #[test]
    fn completion_gate_inert_defaults_when_unconfigured() {
        // No [completion] section anywhere: defaults apply and no checks are
        // registered — the gate stays inert (S-6), with no provenance noise.
        let dir = tmp();
        let resolved = load(&dir, &CliOverrides::default()).unwrap();
        assert!(resolved.completion_config.enabled);
        assert_eq!(resolved.completion_config.max_attempts, 3);
        assert!(resolved.completion_checks.is_empty());
        assert!(!resolved
            .provenance
            .iter()
            .any(|p| p.piece.starts_with("completion")));
    }

    #[test]
    fn document_config_parses_and_merges_scalars() {
        let cfg = ConfigFile::parse("[document]\nmax_bytes = 1048576\n").expect("document");
        assert_eq!(cfg.document.max_bytes, Some(1_048_576));

        let mut base = ConfigFile::default();
        base.merge(cfg);
        assert_eq!(base.document.max_bytes, Some(1_048_576));
    }

    #[test]
    fn document_max_bytes_resolves_with_provenance() {
        let dir = tmp();
        let agents = dir.join(".agents");
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(
            agents.join("config.toml"),
            "[document]\nmax_bytes = 1048576\n",
        )
        .unwrap();

        let resolved = load(&dir, &CliOverrides::default()).unwrap();
        assert_eq!(resolved.document_max_bytes, 1_048_576);
        assert!(resolved
            .provenance
            .iter()
            .any(|p| p.piece == "document.max_bytes" && p.source.starts_with("project")));
    }

    #[test]
    fn document_max_bytes_defaults_to_32_mib_when_unconfigured() {
        // No [document] section anywhere: the 32 MiB default applies (Tech
        // Spec §5.2, P-12) with no provenance noise (silent on the default).
        let dir = tmp();
        let resolved = load(&dir, &CliOverrides::default()).unwrap();
        assert_eq!(resolved.document_max_bytes, 32 * 1024 * 1024);
        assert!(!resolved
            .provenance
            .iter()
            .any(|p| p.piece == "document.max_bytes"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_group_readable_keys_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("emberly-keys-{}", std::process::id()));
        if let Err(e) = std::fs::create_dir_all(&dir) {
            panic!("mkdir: {e}");
        }
        let path = dir.join("keys.toml");
        if let Err(e) = std::fs::write(&path, "openai = \"sk-x\"\n") {
            panic!("write: {e}");
        }
        // 0644 = group/other readable → must be refused.
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
        assert!(enforce_private_permissions(&path).is_err());
        // 0600 → accepted.
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        assert!(enforce_private_permissions(&path).is_ok());
    }
}
