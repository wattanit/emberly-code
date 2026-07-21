//! [`ProviderProfileWriter`] — the guided provider/model setup wizard's
//! write seam (Requirements C-7, Design Guideline §4.6). Mirrors
//! [`ConfigReloader`](crate::ConfigReloader): the frontend (`emberly-tui`)
//! collects the wizard's answers and calls this trait; the host binary
//! implements it against `.agents/config.toml` / `keys.toml`, so the
//! frontend never needs to know either file's TOML schema (A-1).

/// The wizard's five answers: a profile name distinct from the adapter (wire
/// format), plus endpoint, model id, and API key.
pub struct NewProviderProfile {
    /// The `[providers.<name>]` table name the user chose (e.g. `deepseek`).
    pub name: String,
    /// Wire-format adapter: `"anthropic"` or `"openai"`.
    pub adapter: String,
    /// Endpoint base URL; `None`/empty defers to the adapter's built-in
    /// default.
    pub base_url: Option<String>,
    /// The model id to use with this profile.
    pub model_id: String,
    /// The API key, in the clear. Never logged, never transcripted, and
    /// never written anywhere but the keys file (same handling a hand-placed
    /// `keys.toml` entry already gets — C-7 grants the guided path no
    /// exemption).
    pub api_key: String,
}

/// Frontend-injected seam (mirrors [`ConfigReloader`](crate::ConfigReloader))
/// so `emberly-tui` can create a new provider profile + its key entry
/// without owning `config.toml`/`keys.toml` schema knowledge itself (A-1).
/// Implemented by the binary composition root.
pub trait ProviderProfileWriter: Send + Sync {
    /// Write `profile` to the project config tier and its key to the global
    /// keys file. Returns a human-readable error (a name collision, or an
    /// I/O/permission failure) for the wizard to show on its summary step —
    /// the write does not otherwise validate the profile (Tech Spec C-7: a
    /// wizard-created profile is only as validated as a hand-edited one,
    /// checked at the same point — `/model` switch time).
    fn write_profile(&self, profile: NewProviderProfile) -> Result<(), String>;
}
