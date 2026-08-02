//! The skill system (FR-7, T-15): catalog refresh, inspection, and
//! invocation.
//!
//! Part of the `Engine` inherent impl, split out of one 2,700-line
//! block; the engine is still the single owner of this state.

use super::*;

impl Engine {
    /// Rebuild the skill catalog from disk and refresh the cached fields
    /// (Tech Spec §8.2). Called at session start and on session switch so a new
    /// root re-scans project skills. When skills are disabled or no user dir
    /// exists, the catalog is cleared.
    pub(super) fn refresh_skill_catalog(&mut self) {
        if let Some(catalog) = &self.skill_catalog {
            let (metas, shadows) = catalog.discover();
            self.skill_catalog_text = render_catalog(&metas);
            self.skill_metas = metas;
            self.skill_shadows = shadows;
        } else {
            self.skill_catalog_text.clear();
            self.skill_metas.clear();
            self.skill_shadows.clear();
        }
    }

    /// Fetch a skill's instruction body for the inspector (FR-7, Design §4.9)
    /// and emit it as `SkillBody`. Resolved through the catalog so project
    /// precedence + untrusted-root gating still apply; loading the body for
    /// display runs **no** bundled script (FR-7). An unknown/disabled/untrusted-
    /// absent skill emits a `Notice` rather than a `SkillBody`, so the inspector
    /// never opens an empty overlay.
    pub(super) async fn inspect_skill(&self, name: String) {
        let invocation = if self.skills_config.enabled {
            self.skill_catalog
                .as_ref()
                .and_then(|cat| cat.invoke(&name))
        } else {
            None
        };
        match invocation {
            Some(inv) => {
                self.emit(UiEvent::SkillBody {
                    name,
                    origin: inv.origin,
                    body: inv.body,
                    resources: inv.resources,
                })
                .await;
            }
            None => {
                self.emit(UiEvent::Notice {
                    message: format!("skill '{name}' is not available"),
                })
                .await;
            }
        }
    }

    /// Handle a skill invoke from the `skill` tool (T-15, FR-7). Resolves the
    /// body via the catalog and replies over the oneshot. An unknown/untrusted-
    /// absent skill returns `Ok(None)` which the tool maps to an HC-6 failure
    /// (never a panic). **Read-only — emits nothing** (the catalog does not
    /// change on invoke; contrast `on_memory_op` which refreshes `MemoryStatus`).
    pub(super) async fn on_skill_invoke(&mut self, ask: SkillAsk) {
        let result = if self.skills_config.enabled {
            self.skill_catalog
                .as_ref()
                .and_then(|cat| cat.invoke(&ask.name))
        } else {
            None
        };
        let _ = ask.reply.send(Ok(result));
    }
}
