//! The skill gate as seen by tools (T-15, FR-7, Tech Spec §5.2/§8.2).
//!
//! Skills are named, progressively-disclosed capability folders the model
//! discovers and invokes on demand. The `skill` tool **reads instruction text
//! only — it executes nothing**; running a skill's bundled script is a separate
//! ordinary `bash` call under the full permission/sandbox model (FR-7 honesty
//! clause, Tech Spec §8.2). It is **not permission-gated** (reads from
//! trust-resolved dirs, like `recall`). Project-scope skills load **only under
//! a trusted root** (FR-1, Tech Spec §6.7).

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Which scope a skill lives in (Tech Spec §8.2). Mirrors `MemoryScope` — origin
/// drives the Design §4.9 tool line and precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillOrigin {
    /// User-global (`~/.config/emberly/skills/`), always loaded.
    User,
    /// Project-scoped (`.agents/skills/`), loaded only under a trusted root.
    Project,
}

/// One entry in the skill catalog — pinned into context so the model always
/// knows what it can invoke (Tech Spec §7, §8.2). Only metadata is standing;
/// the body loads on demand via the `skill` tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub origin: SkillOrigin,
}

/// The result of invoking a skill: the `SKILL.md` body (post-frontmatter) and
/// the paths of bundled resources (every file in the skill folder except
/// `SKILL.md`), plus the origin for the Design §4.9 tool line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInvocation {
    pub body: String,
    pub resources: Vec<String>,
    pub origin: SkillOrigin,
}

/// The skill gate is unreachable (engine gone). Fail-closed: the tool maps this
/// to a [`ToolOutcome::failure`](crate::tool::ToolOutcome::failure) (HC-6),
/// never a panic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillError;

/// The gate the `skill` tool calls to load a skill's instruction body (T-15,
/// Tech Spec §5.2/§8.2). Implemented by the engine; injected into
/// [`ToolCtx`](crate::ctx::ToolCtx). The tool never executes anything — it
/// reads instruction text from trust-resolved dirs, so it is not
/// permission-gated (FR-7 honesty clause).
#[async_trait]
pub trait SkillGate: Send + Sync {
    /// Load the named skill's `SKILL.md` body and resource paths. Returns
    /// `Err` if the engine is unreachable (fail-closed, HC-6).
    async fn invoke_skill(&self, name: String) -> Result<Option<SkillInvocation>, SkillError>;
}

/// The default gate installed by [`ToolCtx::new`](crate::ctx::ToolCtx::new):
/// returns `Err` for every request. The engine replaces it with a real
/// channel-backed gate via
/// [`with_skill_gate`](crate::ctx::ToolCtx::with_skill_gate); contexts that
/// never invoke skills (most tests) keep this safe no-op.
pub(crate) struct DropSkillGate;

#[async_trait]
impl SkillGate for DropSkillGate {
    async fn invoke_skill(&self, _name: String) -> Result<Option<SkillInvocation>, SkillError> {
        Err(SkillError)
    }
}
