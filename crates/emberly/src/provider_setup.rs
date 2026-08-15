//! Provider selection and the pure-Rust HTTPS client.
//!
//! The binary is the composition root: it installs the **pure-Rust** RustCrypto
//! rustls provider (HC-2 — no C toolchain needed to build) and constructs the
//! `reqwest::Client` that the provider clients borrow.
//!
//! Configuration comes from [`crate::config`] (config files + `EMBERLY_*`
//! overrides + `keys.toml`). When no provider is configured, the caller falls
//! back to the offline placeholder.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, bail, Context};
use emberly_providers::{
    AnthropicProvider, Auth, Effort, ModelInfo, OpenAiProvider, Pricing, Provider, StreamTimeouts,
};

use crate::config::{self, AuthFile, CliOverrides, ProfileFile, Resolved};
use emberly_tools::{default_registry, SearchAuth, SearchClient, ToolRegistry, WebSearchTool};

/// How long to allow for establishing a connection (DNS, TCP, TLS). Bounds only
/// the handshake, never a request already in flight, so it cannot cut short a
/// long streaming completion. Without it the wait falls back to the OS default,
/// which is minutes long and not a value to depend on.
///
/// There is deliberately no overall request timeout to sit beside this: a
/// streaming completion legitimately runs for minutes, so liveness is enforced
/// where it can tell a slow generation from a dead connection — the per-chunk
/// windows in `emberly-providers::wire`.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

/// A chosen live provider plus display/label info.
pub struct Selection {
    pub provider: Arc<dyn Provider>,
    pub model: String,
    pub label: String,
}

/// Build a live provider by resolving the active profile (Tech Spec §4.5), or
/// `None` to fall back to the offline placeholder (when no profile is active).
/// Adding a provider that reuses an existing adapter is config only — nothing
/// here is vendor-specific (P-8).
pub fn build(resolved: &Resolved) -> anyhow::Result<Option<Selection>> {
    let Some(profile_name) = resolved.provider.clone() else {
        return Ok(None);
    };
    let model = resolved.model.clone().context(
        "a model must be configured when a provider is set (EMBERLY_MODEL, --model, or config.toml)",
    )?;
    let provider = build_profile(
        &resolved.providers,
        &profile_name,
        &model,
        resolved.stream_timeouts,
    )?;
    Ok(Some(Selection {
        provider,
        label: format!("{profile_name}/{model}"),
        model,
    }))
}

/// The effort levels a model offers (P-9), from its config metadata. A model
/// declares an effort control by setting a default `effort`; its levels are the
/// named subset, or the full ladder when none is named. No `effort` ⇒ empty (no
/// control, so the picker is hidden). Unparseable level names are skipped.
fn effort_levels_from(meta: Option<&config::ModelFile>) -> Vec<Effort> {
    let Some(meta) = meta else {
        return Vec::new();
    };
    if meta.effort.is_none() {
        return Vec::new();
    }
    match &meta.effort_levels {
        Some(names) => names.iter().filter_map(|n| Effort::parse(n)).collect(),
        None => Effort::ALL.to_vec(),
    }
}

/// Build a provider from a single profile + model — the shared resolution used
/// by [`build`] and by the in-session [`ProviderFactory`](emberly_core::ProviderFactory).
/// Adding a provider that reuses an existing adapter is config only (P-8).
fn build_profile(
    providers: &HashMap<String, ProfileFile>,
    profile_name: &str,
    model: &str,
    stream_timeouts: StreamTimeouts,
) -> anyhow::Result<Arc<dyn Provider>> {
    let profile = providers.get(profile_name).ok_or_else(|| {
        let mut known: Vec<&String> = providers.keys().collect();
        known.sort();
        anyhow!(
            "unknown provider profile '{profile_name}' (configured: {})",
            known
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let adapter = profile.adapter.as_deref().ok_or_else(|| {
        anyhow!("provider profile '{profile_name}' has no `adapter` (expected \"anthropic\" or \"openai\")")
    })?;

    let auth = resolve_auth(profile.auth.as_ref(), profile_name)?;
    let meta = profile.models.get(model);
    let model_info = ModelInfo {
        model: model.to_string(),
        context_window: meta.and_then(|m| m.context_window).unwrap_or(200_000),
        max_output_tokens: meta.and_then(|m| m.max_output).unwrap_or(4_096),
        pricing: meta.and_then(|m| m.pricing).map(|p| Pricing {
            input_per_mtok: p.input,
            output_per_mtok: p.output,
        }),
        // Reasoning effort (P-9): a model declares a control by setting a
        // default `effort`. Levels default to the full ladder unless the config
        // names a subset. No `effort` ⇒ no control (empty levels).
        default_effort: meta
            .and_then(|m| m.effort.as_deref())
            .and_then(Effort::parse),
        effort_levels: effort_levels_from(meta),
        vision: meta.and_then(|m| m.vision).unwrap_or(false),
        documents: meta.and_then(|m| m.documents).unwrap_or(false),
    };
    let client = build_https_client()?;

    let provider: Arc<dyn Provider> = match adapter {
        "anthropic" => match &profile.base_url {
            Some(base) => Arc::new(AnthropicProvider::new(
                client,
                auth,
                base.clone(),
                model_info,
                stream_timeouts,
            )),
            None => Arc::new(AnthropicProvider::with_default_url(
                client,
                auth,
                model_info,
                stream_timeouts,
            )),
        },
        "openai" => {
            let base = profile
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
            Arc::new(OpenAiProvider::new(
                client,
                auth,
                base,
                model_info,
                stream_timeouts,
            ))
        }
        other => bail!(
            "unknown adapter '{other}' in profile '{profile_name}' \
             (expected \"anthropic\" or \"openai\")"
        ),
    };
    Ok(provider)
}

/// A [`ProviderFactory`](emberly_core::ProviderFactory) over the resolved
/// profiles, so the engine can switch models in-session (C-6) without
/// depending on config/wiring. Holds the merged profile map (cheap to clone).
pub struct ConfiguredProviders {
    providers: HashMap<String, ProfileFile>,
    stream_timeouts: StreamTimeouts,
}

impl ConfiguredProviders {
    #[must_use]
    pub fn new(resolved: &Resolved) -> Self {
        Self {
            providers: resolved.providers.clone(),
            stream_timeouts: resolved.stream_timeouts,
        }
    }
}

impl emberly_core::ProviderFactory for ConfiguredProviders {
    fn build(&self, profile: &str, model: &str) -> Result<emberly_core::ProviderChoice, String> {
        let provider = build_profile(&self.providers, profile, model, self.stream_timeouts)
            .map_err(|e| e.to_string())?;
        Ok(emberly_core::ProviderChoice {
            provider,
            profile: profile.to_string(),
            model: model.to_string(),
        })
    }

    fn profiles(&self) -> Vec<String> {
        let mut names: Vec<String> = self.providers.keys().cloned().collect();
        names.sort();
        names
    }
}

/// A [`ConfigReloader`](emberly_core::ConfigReloader) that re-runs
/// [`config::load`] for the project on an in-app edit (C-5), so `/config` and
/// `/prompt` take effect on the running session. Holds the launch-time
/// `sandbox.require` to detect a restart-only change, and the last-seen
/// provider profiles to detect a *content* change (e.g. filling in
/// `base_url`/`auth` on an already-present profile) that leaves the profile
/// *name* list untouched — `reload` takes `&self`, so this needs interior
/// mutability.
pub struct ConfiguredReloader {
    project_root: PathBuf,
    provider: Option<String>,
    model: Option<String>,
    launch_sandbox_require: bool,
    last_providers: Mutex<HashMap<String, ProfileFile>>,
}

impl ConfiguredReloader {
    #[must_use]
    pub fn new(
        project_root: PathBuf,
        cli: &CliOverrides,
        launch_sandbox_require: bool,
        initial_providers: HashMap<String, ProfileFile>,
    ) -> Self {
        Self {
            project_root,
            provider: cli.provider.clone(),
            model: cli.model.clone(),
            launch_sandbox_require,
            last_providers: Mutex::new(initial_providers),
        }
    }
}

impl emberly_core::ConfigReloader for ConfiguredReloader {
    fn reload(&self) -> Result<emberly_core::ReloadedConfig, String> {
        let cli = CliOverrides {
            provider: self.provider.clone(),
            model: self.model.clone(),
        };
        let resolved = config::load(&self.project_root, &cli).map_err(|e| e.to_string())?;
        let mut profiles: Vec<String> = resolved.providers.keys().cloned().collect();
        profiles.sort();
        let mut restart_notes = Vec::new();
        if resolved.sandbox_require != self.launch_sandbox_require {
            restart_notes.push("sandbox.require changed — restart to apply".to_string());
        }
        let provider_config_changed = {
            let mut last = self
                .last_providers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let changed = *last != resolved.providers;
            *last = resolved.providers.clone();
            changed
        };
        let (rule_specs, rule_warnings) = config::load_permission_rules(&self.project_root);
        let (tools, tool_warnings) = build_tool_registry(&resolved).map_err(|e| e.to_string())?;
        let mut warnings = rule_warnings;
        warnings.extend(tool_warnings);
        Ok(emberly_core::ReloadedConfig {
            system: resolved.system_prompt.clone(),
            summary_prompt: resolved.summary_prompt.clone(),
            provider_factory: Arc::new(ConfiguredProviders::new(&resolved)),
            profiles,
            configured_provider: resolved.provider.clone(),
            configured_model: resolved.model.clone(),
            provider_config_changed,
            tool_explanations: resolved.tool_explanations,
            loop_config: resolved.loop_config,
            completion_config: resolved.completion_config.clone(),
            completion_checks: resolved.completion_checks.clone(),
            truncate: resolved.truncate,
            context: resolved.context,
            image_max_bytes: resolved.image_max_bytes,
            document_max_bytes: resolved.document_max_bytes,
            memory: resolved.memory.clone(),
            skills: resolved.skills.clone(),
            agents: resolved.agents,
            tools,
            rule_specs,
            restart_notes,
            warnings,
        })
    }
}

/// Build the tool registry from resolved config (Tech Spec §5.5): the
/// built-in suite always, plus `web_search` when `[search]` is enabled and
/// configured. Shared by startup and by [`ConfiguredReloader::reload`] (C-5)
/// so an in-app config edit can add/remove `web_search` without a restart.
pub fn build_tool_registry(resolved: &Resolved) -> anyhow::Result<(ToolRegistry, Vec<String>)> {
    let mut tools = default_registry();
    let mut warnings = Vec::new();
    if resolved.search.enabled {
        if let Some(endpoint) = &resolved.search.endpoint {
            let adapter = resolved.search.adapter.as_deref().unwrap_or("json");
            let tool = build_search_tool(
                endpoint,
                adapter,
                resolved.search.auth.as_ref(),
                resolved.search.max_results,
            )
            .context("failed to build the web_search tool")?;
            tools.register(Arc::new(tool));
        } else if resolved.search.adapter.is_some() {
            warnings.push(
                "[search] has an adapter but no endpoint — set endpoint = \"…\" or search.enabled = false"
                    .to_string(),
            );
        }
    }
    Ok((tools, warnings))
}

/// Turn a profile's `auth` config into an [`Auth`], resolving the key
/// *reference* from env / `keys.toml`. A configured key reference that does not
/// resolve is a clear early error rather than a silent 401 later.
fn resolve_auth(auth: Option<&AuthFile>, profile: &str) -> anyhow::Result<Auth> {
    let Some(auth) = auth else {
        return Ok(Auth::None);
    };
    let scheme = auth.scheme.as_deref().unwrap_or("none");
    let key = match &auth.key {
        Some(reference) => crate::config::api_key(reference)?.ok_or_else(|| {
            anyhow!(
                "profile '{profile}' needs the '{reference}' key, but none is set \
                 (export {}_API_KEY or add `{reference} = \"…\"` to keys.toml)",
                reference.to_ascii_uppercase()
            )
        })?,
        None => String::new(),
    };
    Ok(match scheme {
        "none" => Auth::None,
        "bearer" => Auth::Bearer(key),
        "x-api-key" => Auth::XApiKey(key),
        "header" => Auth::Header {
            name: auth.header.clone().unwrap_or_default(),
            value: key,
        },
        other => bail!(
            "unknown auth scheme '{other}' in profile '{profile}' \
             (expected bearer, x-api-key, header, or none)"
        ),
    })
}

/// Build an HTTPS-capable client backed by the pure-Rust crypto provider.
///
/// Deliberately sets no overall request timeout: a streaming completion
/// legitimately runs for minutes, so liveness is enforced where it can tell a
/// slow generation from a dead connection — the per-chunk idle timeout in
/// `emberly-providers::wire` — not by a clock on the whole request.
fn build_https_client() -> anyhow::Result<reqwest::Client> {
    // Install the RustCrypto provider as the process default (idempotent — a
    // second call returns Err, which we ignore). reqwest's rustls integration,
    // built with the `*-no-provider` feature, uses this default.
    let _ = rustls_rustcrypto::provider().install_default();
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .context("failed to build the HTTPS client")
}

/// Turn a search `[search].auth` config into a [`SearchAuth`], resolving the
/// key *reference* from env / `keys.toml` (mirror [`resolve_auth`]). Handles the
/// search-side `query` scheme the provider `Auth` lacks (Tech Spec §5.5).
pub fn resolve_search_auth(auth: Option<&AuthFile>) -> anyhow::Result<SearchAuth> {
    let Some(auth) = auth else {
        return Ok(SearchAuth::None);
    };
    let scheme = auth.scheme.as_deref().unwrap_or("none");
    let key = match &auth.key {
        Some(reference) => config::api_key(reference)?.ok_or_else(|| {
            anyhow!(
                "search needs the '{reference}' key, but none is set \
                 (export {}_API_KEY or add `{reference} = \"…\"` to keys.toml)",
                reference.to_ascii_uppercase()
            )
        })?,
        None => String::new(),
    };
    Ok(match scheme {
        "none" => SearchAuth::None,
        "bearer" => SearchAuth::Bearer(key),
        "header" => SearchAuth::Header {
            name: auth.header.clone().unwrap_or_default(),
            value: key,
        },
        // The new search-side scheme: append `?param=<key>` to the URL (Tech
        // Spec §5.5). The provider `Auth` lacks `query` — G-24 candidate.
        "query" => SearchAuth::Query {
            param: auth.header.clone().unwrap_or_else(|| "key".to_string()),
            value: key,
        },
        other => bail!(
            "unknown auth scheme '{other}' in [search] \
             (expected bearer, header, query, or none)"
        ),
    })
}

/// Build the `web_search` tool from resolved config (Tech Spec §5.5). Called by
/// the binary composition root when `search.enabled && endpoint.is_some()`.
/// Mirrors [`build_profile`]: resolve auth, build the client, construct the tool.
pub fn build_search_tool(
    endpoint: &str,
    adapter: &str,
    auth: Option<&AuthFile>,
    max_results: usize,
) -> anyhow::Result<WebSearchTool> {
    let client = build_https_client()?;
    let search_auth = resolve_search_auth(auth)?;
    let search_client = SearchClient::new(client, endpoint, adapter, search_auth, max_results);
    Ok(WebSearchTool::new(search_client))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ModelFile;

    fn profile(adapter: &str, base_url: Option<&str>) -> ProfileFile {
        ProfileFile {
            adapter: Some(adapter.to_string()),
            base_url: base_url.map(str::to_string),
            auth: None, // Auth::None → no key needed, keeps the test hermetic.
            models: HashMap::new(),
        }
    }

    /// The P-8 property, as a unit test: which client is built is decided purely
    /// by the profile's `adapter` — no vendor branching, no network.
    #[test]
    fn build_profile_selects_adapter_from_config_only() {
        let mut providers = HashMap::new();
        providers.insert("a".to_string(), profile("anthropic", None));
        providers.insert(
            "o".to_string(),
            profile("openai", Some("http://localhost:0/v1")),
        );

        let anth = build_profile(&providers, "a", "m1", StreamTimeouts::default()).expect("anthropic builds");
        assert_eq!(anth.id().to_string(), "anthropic");
        assert_eq!(anth.model_info().model, "m1");

        let oai = build_profile(&providers, "o", "m2", StreamTimeouts::default()).expect("openai builds");
        assert_eq!(oai.id().to_string(), "openai-compat");
        assert_eq!(oai.model_info().model, "m2");
    }

    #[test]
    fn build_profile_errors_are_clear() {
        // `Arc<dyn Provider>` isn't Debug, so extract the error message by hand.
        fn err(result: anyhow::Result<Arc<dyn Provider>>) -> String {
            match result {
                Ok(_) => panic!("expected an error"),
                Err(e) => e.to_string(),
            }
        }

        let empty = HashMap::new();
        assert!(err(build_profile(&empty, "nope", "m", StreamTimeouts::default())).contains("unknown provider profile"));

        let mut weird = HashMap::new();
        weird.insert("x".to_string(), profile("weird", None));
        assert!(err(build_profile(&weird, "x", "m", StreamTimeouts::default())).contains("unknown adapter"));

        let mut no_adapter = HashMap::new();
        no_adapter.insert("y".to_string(), ProfileFile::default());
        assert!(err(build_profile(&no_adapter, "y", "m", StreamTimeouts::default())).contains("no `adapter`"));
    }

    #[test]
    fn per_model_metadata_feeds_model_info() {
        let mut prof = profile("openai", Some("http://localhost:0/v1"));
        prof.models.insert(
            "m".to_string(),
            ModelFile {
                context_window: Some(123_456),
                max_output: Some(4_321),
                pricing: None,
                effort: None,
                effort_levels: None,
                vision: None,
                documents: None,
            },
        );
        let mut providers = HashMap::new();
        providers.insert("p".to_string(), prof);
        let info = build_profile(&providers, "p", "m", StreamTimeouts::default())
            .expect("builds")
            .model_info();
        assert_eq!(info.context_window, 123_456);
        assert_eq!(info.max_output_tokens, 4_321);
    }

    #[test]
    fn documents_flag_feeds_model_info() {
        // P-12: a model declaring `documents = true` in config surfaces as
        // `ModelInfo.documents` (mirrors the `vision` flag round trip).
        let mut prof = profile("openai", Some("http://localhost:0/v1"));
        prof.models.insert(
            "m".to_string(),
            ModelFile {
                context_window: None,
                max_output: None,
                pricing: None,
                effort: None,
                effort_levels: None,
                vision: None,
                documents: Some(true),
            },
        );
        let mut providers = HashMap::new();
        providers.insert("p".to_string(), prof);
        let info = build_profile(&providers, "p", "m", StreamTimeouts::default())
            .expect("builds")
            .model_info();
        assert!(info.documents);
    }

    #[test]
    fn effort_config_declares_default_and_levels() {
        // A default `effort` with no explicit levels ⇒ the full ladder.
        let mut prof = profile("openai", Some("http://localhost:0/v1"));
        prof.models.insert(
            "m".to_string(),
            ModelFile {
                effort: Some("high".into()),
                ..Default::default()
            },
        );
        let mut providers = HashMap::new();
        providers.insert("p".to_string(), prof);
        let info = build_profile(&providers, "p", "m", StreamTimeouts::default())
            .expect("builds")
            .model_info();
        assert_eq!(info.default_effort, Some(Effort::High));
        assert_eq!(info.effort_levels, Effort::ALL.to_vec());
    }

    #[test]
    fn no_effort_config_means_no_control() {
        // No `effort` key ⇒ no levels, so the picker stays hidden (P-9).
        let mut prof = profile("openai", Some("http://localhost:0/v1"));
        prof.models.insert("m".to_string(), ModelFile::default());
        let mut providers = HashMap::new();
        providers.insert("p".to_string(), prof);
        let info = build_profile(&providers, "p", "m", StreamTimeouts::default())
            .expect("builds")
            .model_info();
        assert_eq!(info.default_effort, None);
        assert!(info.effort_levels.is_empty());
    }

    #[test]
    fn effort_config_honors_an_explicit_level_subset() {
        let mut prof = profile("openai", Some("http://localhost:0/v1"));
        prof.models.insert(
            "m".to_string(),
            ModelFile {
                effort: Some("low".into()),
                effort_levels: Some(vec!["low".into(), "high".into()]),
                ..Default::default()
            },
        );
        let mut providers = HashMap::new();
        providers.insert("p".to_string(), prof);
        let info = build_profile(&providers, "p", "m", StreamTimeouts::default())
            .expect("builds")
            .model_info();
        assert_eq!(info.effort_levels, vec![Effort::Low, Effort::High]);
    }

    /// [`ConfiguredReloader::reload`]'s `provider_config_changed` detection
    /// (C-5) relies on `HashMap<String, ProfileFile>` structural equality to
    /// notice a profile whose *content* changed without its *name* changing
    /// (e.g. filling in `base_url` on an already-present profile such as a
    /// built-in placeholder). This pins that assumption directly on the
    /// types involved, without touching disk / the real `~/.config` tree.
    #[test]
    fn profile_file_equality_distinguishes_content_not_just_names() {
        let mut before = HashMap::new();
        before.insert("openai".to_string(), profile("openai", None));
        let mut after = HashMap::new();
        after.insert(
            "openai".to_string(),
            profile("openai", Some("https://api.openai.com/v1")),
        );

        assert_eq!(
            before.keys().collect::<Vec<_>>(),
            after.keys().collect::<Vec<_>>(),
            "the name set is unchanged"
        );
        assert_ne!(
            before, after,
            "but the content differs, which the reloader must still catch"
        );
    }
}
