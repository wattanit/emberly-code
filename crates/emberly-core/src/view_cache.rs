//! Derived conversation-state cache (FR-5, Tech Spec §3.2a).
//!
//! [`ViewCache`] serializes the engine's *derived* view — the in-context
//! conversation, turn map, and token-accounting totals — to a sidecar
//! `<session-id>-view.json` so a resume restores that state directly instead of
//! replaying and re-tokenizing the whole JSONL transcript. The cache is a pure
//! function of the transcript: it holds no fact the transcript lacks (HC-7 does
//! not apply to it). Losing, corrupting, or deleting it loses nothing — the
//! replay fallback ([`crate::resume::rebuild_conversation`]) is always correct.
//!
//! The cache has its own version ([`VIEW_CACHE_VERSION`]), independent of the
//! transcript's [`SCHEMA_VERSION`](crate::transcript::SCHEMA_VERSION): the
//! sidecar shape differs from `TranscriptRecord`. A cache whose version is
//! unknown or older is treated as stale → replay (never a crash).

use std::path::{Path, PathBuf};

use emberly_providers::Message;
use serde::{Deserialize, Serialize};

use crate::id::SessionId;
use crate::types::TokenUsage;

/// Version of the `-view.json` sidecar shape. Independent of the transcript
/// [`SCHEMA_VERSION`](crate::transcript::SCHEMA_VERSION) — the cache
/// serializes the engine's derived view types, not `TranscriptRecord`. A cache
/// whose version is unknown/older is treated as stale → replay (never a crash).
pub const VIEW_CACHE_VERSION: u32 = 1;

/// The engine's derived conversation state, serialized to a sidecar
/// `<session-id>-view.json` (FR-5, Tech Spec §3.2a). Purely derived from the
/// transcript — never authoritative (HC-7 subordination).
///
/// Fields mirror the engine state reconstructed on resume
/// ([`Engine`](crate::engine::Engine)): the in-context conversation, the
/// parallel turn map, and compaction/accounting totals. `auto_compact_armed`
/// is deliberately not persisted — it is transient FR-4 hysteresis state
/// re-derived to a safe default on resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewCache {
    /// The [`VIEW_CACHE_VERSION`] this cache was written under. A mismatch on
    /// read → stale → replay.
    pub version: u32,
    /// The session id this cache claims to derive from, for a sanity check
    /// against the transcript's `session_start`.
    pub session_id: SessionId,

    // --- The engine's derived conversation view ---
    /// Normalized messages — what the model sees (reduced, windowed, compacted).
    pub conversation: Vec<Message>,
    /// Parallel to `conversation`: the stable monotonic turn number of each
    /// message (FR-3).
    pub turn_map: Vec<usize>,
    /// The next turn number to assign (monotonic; never decremented).
    pub next_turn: usize,
    /// Whether a compaction has run (pinned summary at `conversation[1]`).
    pub compacted: bool,
    /// Whether the first user message (the pinned original task) has been
    /// recorded.
    pub original_task_recorded: bool,

    // --- Token-accounting totals ---
    /// Cumulative billed tokens this session.
    pub session_usage: TokenUsage,
    /// Running session cost estimate in USD (zero when pricing is absent).
    pub session_cost_usd: f64,
    /// Most recent authoritative prompt-token count (current context size).
    /// `None` until the first provider `Usage` event.
    pub context_tokens_authoritative: Option<u64>,

    // --- Staleness guard inputs (captured at write time, group 3) ---
    /// The transcript file's byte length when this cache was written. In the
    /// append-only model the cache is always built through the whole current
    /// file, so this length doubles as the built-through byte offset. On
    /// resume, only an exact match with the current transcript size is
    /// trusted; grown, shorter, or unreadable → stale → replay.
    pub transcript_byte_len: u64,
}

/// Derive the `-view.json` sidecar path from a transcript path, mirroring the
/// `{stem}-outputs` sidecar pattern in [`FileTranscript`](crate::transcript::FileTranscript)
/// (Tech Spec §3.2). One helper so writer and reader always agree.
#[must_use]
pub fn view_cache_path(transcript_path: &Path) -> PathBuf {
    let stem = transcript_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session");
    transcript_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{stem}-view.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_cache_path_mirrors_sidecar_pattern() {
        let transcript = Path::new(".agents/sessions/abc123.jsonl");
        let cache = view_cache_path(transcript);
        assert_eq!(
            cache,
            Path::new(".agents/sessions/abc123-view.json")
        );
    }

    #[test]
    fn view_cache_path_handles_no_parent() {
        let transcript = Path::new("abc123.jsonl");
        let cache = view_cache_path(transcript);
        assert_eq!(cache, Path::new("abc123-view.json"));
    }

    #[test]
    fn view_cache_round_trips_through_serde() {
        let cache = ViewCache {
            version: VIEW_CACHE_VERSION,
            session_id: SessionId::new(),
            conversation: vec![Message::user_text("hello"), Message::assistant_text("hi")],
            turn_map: vec![0, 1],
            next_turn: 2,
            compacted: false,
            original_task_recorded: true,
            session_usage: TokenUsage::default(),
            session_cost_usd: 0.0,
            context_tokens_authoritative: None,
            transcript_byte_len: 1024,
        };
        let json = serde_json::to_string(&cache).unwrap_or_else(|e| panic!("serialize: {e}"));
        let restored: ViewCache =
            serde_json::from_str(&json).unwrap_or_else(|e| panic!("deserialize: {e}"));
        assert_eq!(restored.version, VIEW_CACHE_VERSION);
        assert_eq!(restored.conversation.len(), 2);
        assert_eq!(restored.transcript_byte_len, 1024);
    }
}
