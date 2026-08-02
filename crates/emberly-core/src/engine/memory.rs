//! The durable memory tool (FR-6, T-13): index refresh, the memory
//! operations themselves, and the read-back the UI shows.
//!
//! Part of the `Engine` inherent impl; the engine is the sole owner of the
//! state these methods touch.

use super::*;

impl Engine {
    /// Reload the memory index strings from the store into the cached fields.
    pub(super) fn refresh_memory_indexes(&mut self) {
        if let Some(store) = &self.memory.store {
            self.memory.user_index = store.user_index();
            self.memory.project_index = store.project_index();
        } else {
            self.memory.user_index.clear();
            self.memory.project_index.clear();
        }
    }

    /// The single validated memory write path, shared by the `memory` tool
    /// (`on_memory_op`) and the inspector's `MemoryMutate` command (FR-6, C-5).
    /// Routes to the store (whose `slug` guard re-validates the name), refreshes
    /// the pinned indexes, emits `MemoryStatus`, and warns once past the soft
    /// cap. Extracting it means the tool and the inspector **cannot diverge** —
    /// the harness performs the write in exactly one place, never the TUI.
    pub(super) async fn execute_memory_op(
        &mut self,
        req: &emberly_tools::MemoryRequest,
    ) -> emberly_tools::MemoryOutcome {
        // Compute the result first so the immutable borrow of the store ends
        // before the mutable refresh + emit.
        let computed = if self.memory.config.enabled {
            self.memory.store.as_ref().map(|store| store.execute(req))
        } else {
            None
        };
        match computed {
            Some(result) => {
                self.refresh_memory_indexes();
                let (user_count, project_count) = self
                    .memory
                    .store
                    .as_ref()
                    .map_or((0, 0), |s| s.status_counts());
                self.emit(UiEvent::MemoryStatus {
                    user: user_count,
                    project: project_count,
                })
                .await;
                // Soft-cap warn (Tech Spec §16): warn once when the combined
                // index exceeds `max_index_entries`. Do not truncate.
                let total = user_count + project_count;
                if !self.memory.warn_emitted && total > self.memory.config.max_index_entries {
                    self.memory.warn_emitted = true;
                    self.emit(UiEvent::Notice {
                        message: format!(
                            "memory index has {total} entries (soft cap {}) — consider trimming or consolidating",
                            self.memory.config.max_index_entries
                        ),
                    })
                    .await;
                }
                result
            }
            None => emberly_tools::MemoryOutcome::Rejected {
                reason: "memory is disabled".into(),
            },
        }
    }

    /// Handle a memory op from the `memory` tool (T-13, FR-6): run the shared
    /// write path and reply over the oneshot.
    pub(super) async fn on_memory_op(&mut self, ask: MemoryAsk) {
        let outcome = self.execute_memory_op(&ask.req).await;
        let _ = ask.reply.send(Ok(outcome));
    }

    /// List the memory entries grouped by scope for the inspector (FR-6, Design
    /// §4.9), and emit them as `MemoryEntries`. Summaries only — no bodies read
    /// (progressive disclosure). Both groups are empty when memory is disabled;
    /// `project` is empty on an untrusted root (`list_entries` returns empty for
    /// an unavailable scope), which makes the project section silently absent
    /// (FR-1).
    pub(super) async fn emit_memory_entries(&self) {
        let (user, project) = match &self.memory.store {
            Some(store) if self.memory.config.enabled => (
                store.list_entries(emberly_tools::MemoryScope::User),
                store.list_entries(emberly_tools::MemoryScope::Project),
            ),
            _ => (Vec::new(), Vec::new()),
        };
        self.emit(UiEvent::MemoryEntries { user, project }).await;
    }

    /// Fetch a single memory entry's body for the inspector (FR-6, §4.6) and
    /// emit it as `MemoryBody`. Read-only — a `Recall` through the store, which
    /// applies the same trust gating (an untrusted project scope has no store
    /// dir, so its body reads back empty). The body is fetched on demand and
    /// never pinned (progressive disclosure).
    pub(super) async fn emit_memory_body(&self, scope: emberly_tools::MemoryScope, name: String) {
        let body = if self.memory.config.enabled {
            self.memory.store.as_ref().and_then(|store| {
                match store.execute(&emberly_tools::MemoryRequest {
                    op: emberly_tools::MemoryOp::Recall,
                    scope,
                    name: name.clone(),
                    description: None,
                    type_: None,
                    body: None,
                }) {
                    emberly_tools::MemoryOutcome::Recalled { body, .. } => body,
                    _ => None,
                }
            })
        } else {
            None
        };
        self.emit(UiEvent::MemoryBody {
            scope,
            name,
            body: body.unwrap_or_default(),
        })
        .await;
    }
}
