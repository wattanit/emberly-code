//! Session lifecycle: starting fresh, resuming an existing session,
//! adopting its state, and the transcript / view-cache writes that record it
//! (HC-7).
//!
//! Part of the `Engine` inherent impl; the engine is the sole owner of the
//! state these methods touch.

use super::*;

impl Engine {
    /// Start a fresh session in place (`/new`): end the current transcript
    /// cleanly and roll a new one under `session_id`, resetting the conversation
    /// and per-session accounting. The new transcript is created *before* the
    /// old one is ended, so a creation failure leaves the current session intact
    /// (transcript failures are never fatal — HC-7).
    pub(super) async fn start_new_session(&mut self, session_id: SessionId) {
        let path = self.session.dir.join(format!("{session_id}.jsonl"));
        let sink = match FileTranscript::create(&self.session.dir, session_id) {
            Ok(file) => file,
            Err(error) => {
                self.emit(UiEvent::HarnessError {
                    what: "could not start a new session".into(),
                    why: error.to_string(),
                    next: "staying on the current session".into(),
                })
                .await;
                return;
            }
        };
        self.end_departing_session();
        self.session.transcript = Box::new(sink);
        self.adopt_session(session_id, path, Vec::new(), AdoptedState::fresh(), false);
        self.write_transcript(TranscriptEvent::SessionStart {
            session_id,
            provider: self.provider.label.clone(),
            model: self.provider.model.clone(),
            project_root: self.project_root.display().to_string(),
            sandbox: self.safety.sandbox.clone(),
            config_provenance: self.config_provenance.clone(),
            prompts_version: crate::prompts::VERSION,
        });
        self.emit_context_usage().await;
    }

    /// Resume a saved session by id (`/resume` from the picker): end the current
    /// transcript, reopen the target for append, and replace the live
    /// conversation with the one rebuilt from it. Reading the target *before*
    /// ending the current session keeps the current one intact on any failure.
    /// Tries the derived view cache first (FR-5 fast path); falls back to a
    /// full transcript replay when the cache is absent or stale.
    pub(super) async fn resume_session(&mut self, session_id: SessionId) {
        let path = self.session.dir.join(format!("{session_id}.jsonl"));
        let loaded = match crate::resume::read_records(&path) {
            Ok(loaded) => loaded,
            Err(error) => {
                self.emit(UiEvent::HarnessError {
                    what: "could not read that session".into(),
                    why: error.to_string(),
                    next: "staying on the current session".into(),
                })
                .await;
                return;
            }
        };
        let sink = match FileTranscript::open(&path) {
            Ok(file) => file,
            Err(error) => {
                self.emit(UiEvent::HarnessError {
                    what: "could not open that session for writing".into(),
                    why: error.to_string(),
                    next: "staying on the current session".into(),
                })
                .await;
                return;
            }
        };
        self.end_departing_session();
        self.session.transcript = Box::new(sink);

        // FR-5: try the cache first. A valid cache restores the full derived
        // state directly — no per-line re-tokenization. The fast path is silent
        // (Design §8.6).
        if let Some(cache) = crate::resume::try_load_view_cache(&path) {
            let (state, conversation) = AdoptedState::from_cache(cache);
            self.adopt_session(session_id, path, conversation, state, true);
        } else {
            // Fallback: replay the transcript, re-deriving the view. Announce
            // the slow path in one dimmed harness-voice line (Design §8.6).
            self.emit(UiEvent::Notice {
                message: "Rebuilding the session from its transcript…".into(),
            })
            .await;
            let conversation = crate::resume::rebuild_conversation(&loaded.records);
            let compacted = crate::resume::has_compaction(&loaded.records);
            self.adopt_session(
                session_id,
                path,
                conversation.clone(),
                AdoptedState::replayed(&conversation, compacted),
                true,
            );
        }
        self.emit_context_usage().await;
    }

    /// Close out the session being switched away from: record its `session_end`
    /// and refresh its view cache, in that order, while its path and derived
    /// state are still the live ones. Must run before the sink swap.
    ///
    /// The refresh is the whole point: `session_end` grows the transcript past
    /// the byte length the cache recorded, so without it the guard
    /// (`try_load_view_cache`) rejects the cache forever and coming back to this
    /// session replays — which rebuilds the conversation but has no way to
    /// re-derive token accounting, reporting a session with real history as
    /// "0 in / 0 out" (issue #18).
    fn end_departing_session(&mut self) {
        self.write_transcript(TranscriptEvent::SessionEnd { reason: None });
        self.write_view_cache();
    }

    /// Reset session-scoped state to a freshly adopted session and publish the
    /// new transcript path to the shared handle so the host's panic/exit path
    /// names the current session (HC-3). Carries the full derived state —
    /// turn map, compaction flag, and token accounting — so both the cache
    /// fast path (FR-5) and the replay fallback restore correct turn state
    /// (fixing the pre-existing `adopt_session` gap).
    fn adopt_session(
        &mut self,
        session_id: SessionId,
        path: PathBuf,
        conversation: Vec<Message>,
        state: AdoptedState,
        resuming: bool,
    ) {
        self.session.id = session_id;
        // Re-point the scratch store at the new session's own directory (FR-8)
        // — scratch space is never carried across a `/new` or `/resume` switch.
        self.gates.scratch = Arc::new(ScratchStore::new(scratch_dir_for(
            &self.project_root,
            session_id,
        )));
        self.history.messages = conversation;
        self.history.turn_map = state.turn_map;
        self.history.next_turn = state.next_turn;
        self.context.compacted = state.compacted;
        self.session.original_task_recorded = state.original_task_recorded;
        self.session.resuming = resuming;
        self.session.replayed = false;
        self.context.pending = None;
        self.context.auto_armed = true;
        self.session.usage = state.session_usage;
        self.session.cost_usd = state.session_cost_usd;
        self.context.tokens_authoritative = state.context_tokens_authoritative;
        self.next_permission_id = 0;
        self.completion.attempts = 0;
        self.task_list.clear();
        // A staged-but-unsent attachment (FR-10) belongs to the composing
        // message, not the session it was composed in — a switch drops it
        // rather than silently attaching it to the first message of a
        // different session.
        self.pending_attachments.clear();
        // Reload memory indexes for the new session (user-global unchanged,
        // project re-pointed to the new root). The store reads from disk, so a
        // resumed session re-reads the current store (Tech Spec §8.1). Re-emit
        // `MemoryStatus` so the sidebar/inspector are never stale after a
        // session switch — matching the `SkillsAvailable` re-emit below (this
        // closes a pre-existing gap where `/resume` left the memory count
        // stale, Design §4.9).
        self.refresh_memory_indexes();
        let (user_count, project_count) = self
            .memory
            .store
            .as_ref()
            .map_or((0, 0), |s| s.status_counts());
        let _ = self.events_tx.try_send(UiEvent::MemoryStatus {
            user: user_count,
            project: project_count,
        });
        // Re-derive the skill catalog for the new session (user-global
        // unchanged, project re-pointed to the new root). Emit
        // `SkillsAvailable` so the TUI Skills section is never stale after a
        // session switch (Tech Spec §8.2, §3.1).
        self.refresh_skill_catalog();
        let _ = self.events_tx.try_send(UiEvent::SkillsAvailable {
            skills: self.skills.metas.clone(),
        });
        if let Ok(mut guard) = self.session.active_path.write() {
            *guard = path;
        }
    }

    /// Record a user message durably, tagging the first one as the pinned
    /// `original_task` and deriving the session title from it (Tech Spec §7,
    /// §16). `images` is the durable, byte-free record of any attachments
    /// (FR-10) — always recorded regardless of whether the active model has
    /// vision, since it states what the *user* attached, not what reached
    /// the model.
    pub(super) fn record_user_message(&mut self, text: &str, images: Vec<AttachedImageMeta>) {
        let original_task = !self.session.original_task_recorded;
        self.write_transcript(TranscriptEvent::UserMessage {
            text: text.to_string(),
            original_task,
            images,
        });
        if original_task {
            self.session.original_task_recorded = true;
            self.write_transcript(TranscriptEvent::SessionTitle {
                title: clip_title(text),
            });
        }
    }

    /// Resolve a turn-number range to the messages it contains (T-10, FR-3).
    /// Returns the engine's normalized, in-memory messages for those turns —
    /// never raw JSONL. `from`/`to` are inclusive stable turn numbers as shown
    /// in the elision marker. Returns an empty vec if the range matches no
    /// turns (e.g. out of bounds or turns retired by compaction).
    #[must_use]
    pub fn recall_turns(&self, from: usize, to: usize) -> Vec<Message> {
        if from > to || self.history.turn_map.is_empty() {
            return Vec::new();
        }
        let mut result = Vec::new();
        for (i, msg) in self.history.messages.iter().enumerate() {
            let turn = self.history.turn_map[i];
            if turn >= from && turn <= to {
                result.push(msg.clone());
            }
        }
        result
    }

    /// Append one durable event to the transcript (HC-7). Best-effort: the sink
    /// swallows I/O errors. Stamped with the current time at the write edge.
    pub(super) fn write_transcript(&mut self, event: TranscriptEvent) {
        let record = TranscriptRecord::new(OffsetDateTime::now_utc(), event);
        self.session.transcript.record(&record);
        // The log just grew past what the sidecar recorded, so the cache would
        // now fail its own staleness guard (Tech Spec §3.2a). Flagged here, at
        // the one place transcript bytes are ever added, rather than at each
        // caller — a write that forgot to refresh the cache is what made
        // resume drop the session's token accounting (issue #18).
        self.session.cache_dirty = true;
    }

    /// Write the derived conversation-state cache best-effort (FR-5, Tech Spec
    /// §3.2a). Called after the view settles — a turn completes, a compaction
    /// runs, a session is left behind — and at the idle boundary whenever the
    /// transcript has grown since the last write. Losing this file loses
    /// nothing: the replay fallback is always correct (HC-7 subordination). Any
    /// I/O or serialization error is swallowed (HC-3 — never a panic); the cache
    /// is never relied upon.
    pub(super) fn write_view_cache(&mut self) {
        // Cleared up front: the cache is now as reconciled with the transcript
        // as this call can make it. An early return below means there is no
        // transcript to match (a `NoopSink` in tests), and a failed write leaves
        // a cache the staleness guard rejects — retrying fixes neither.
        self.session.cache_dirty = false;
        let transcript_path = {
            let guard = match self.session.active_path.read() {
                Ok(g) => g,
                Err(_) => return,
            };
            guard.clone()
        };
        let byte_len = match std::fs::metadata(&transcript_path) {
            Ok(m) => m.len(),
            Err(_) => return, // no transcript file (e.g. NoopSink in tests)
        };
        let cache = ViewCache {
            version: VIEW_CACHE_VERSION,
            session_id: self.session.id,
            conversation: self.history.messages.clone(),
            turn_map: self.history.turn_map.clone(),
            next_turn: self.history.next_turn,
            compacted: self.context.compacted,
            original_task_recorded: self.session.original_task_recorded,
            session_usage: self.session.usage,
            session_cost_usd: self.session.cost_usd,
            context_tokens_authoritative: self.context.tokens_authoritative,
            transcript_byte_len: byte_len,
        };
        let cache_path = view_cache_path(&transcript_path);
        let json = match serde_json::to_string(&cache) {
            Ok(j) => j,
            Err(_) => return,
        };
        let _ = std::fs::write(&cache_path, json);
    }
}
