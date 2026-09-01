//! MCP connection reporting (FR-11, Tech Spec §5.6/§8.5).
//!
//! Connecting to configured servers, discovering their tools, and inserting
//! the result into the `ToolRegistry` is composition-root logic (the
//! `emberly` binary, `provider_setup::build_tool_registry`) — mirroring
//! exactly how `web_search` is conditionally built there (Tech Spec §5.5).
//! By the time an `Engine` exists, connecting has already happened; this
//! module only reports what happened, once, so it is never a silent
//! background fact (Requirements §1, extends HC-7).
//!
//! Part of the `Engine` inherent impl; the engine is the sole owner of the
//! state these methods touch.

use super::*;

impl Engine {
    /// Report every MCP connection outcome handed in via `EngineConfig`
    /// (FR-11): one `UiEvent` plus one `TranscriptEvent::McpConnection` per
    /// server, drained so this never repeats mid-session (reconnecting only
    /// happens on `/reload`, which reports its own fresh outcomes inline).
    pub(super) async fn report_mcp_connections(&mut self) {
        let outcomes = std::mem::take(&mut self.mcp_connections);
        for outcome in outcomes {
            self.record_and_emit_mcp_outcome(outcome).await;
        }
    }

    /// Write the transcript record and emit the paired `UiEvent` for one MCP
    /// connection outcome (FR-11) — shared by the startup report and by
    /// `/reload`'s re-connection report so the two never diverge.
    pub(super) async fn record_and_emit_mcp_outcome(&mut self, outcome: McpConnectionOutcome) {
        match outcome {
            McpConnectionOutcome::Connected { server, tools } => {
                self.write_transcript(TranscriptEvent::McpConnection {
                    server: server.clone(),
                    tools: tools.clone(),
                    error: None,
                });
                self.emit(UiEvent::McpServerConnected {
                    name: server,
                    tools,
                })
                .await;
            }
            McpConnectionOutcome::Failed { server, reason } => {
                self.write_transcript(TranscriptEvent::McpConnection {
                    server: server.clone(),
                    tools: Vec::new(),
                    error: Some(reason.clone()),
                });
                self.emit(UiEvent::McpServerFailed {
                    name: server,
                    reason,
                })
                .await;
            }
        }
    }
}
