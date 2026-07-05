//! `emberly` — the thin binary: CLI argument handling, wiring the engine to a
//! frontend, and the top-level supervisor that guarantees the session
//! transcript survives any abnormal exit and the terminal is always restored
//! (Requirements HC-3, S-2; Tech Spec §10).
//!
//! `anyhow` lives here at the edge only; library crates use `thiserror`
//! (Tech Spec §1, §11). Phase 0: skeleton entry point only. CLI surface,
//! wiring, and the supervisor are built out across Phases 1 and 5.
#![forbid(unsafe_code)]

fn main() -> anyhow::Result<()> {
    println!("emberly code — scaffold (Phase 0). Not yet interactive.");
    Ok(())
}
