//! `emberly` — the thin binary: CLI, wiring, and the top-level supervisor that
//! guarantees a clean exit (Requirements HC-3, S-2; Tech Spec §10).
//!
//! `anyhow` lives here at the edge only; library crates use `thiserror`
//! (Tech Spec §1, §11). As the composition root, this crate depends on the
//! concrete `providers` and `tools` crates to construct the provider and tool
//! registry it wires into the engine (in Phase 3 it will build the live
//! Anthropic/OpenAI providers here).
//!
//! Phase 1: the supervisor skeleton plus an end-to-end wiring of the engine
//! and the line-mode frontend, driven by a placeholder provider. Transcript
//! persistence and the `abnormal_exit` record land in Phase 5; the panic hook
//! marks where they attach.
#![forbid(unsafe_code)]

use std::sync::Arc;

use emberly_core::{channel, Engine, EngineConfig};
use emberly_providers::Provider;
use emberly_tools::{default_registry, TruncateConfig};
use emberly_tui::{frontend, SessionInfo};

mod config;
mod placeholder;
mod provider_setup;
use placeholder::PlaceholderProvider;

#[tokio::main]
async fn main() {
    install_panic_hook();
    if let Err(error) = run().await {
        // Harness-world failure, surfaced in harness voice (Design §6.1).
        eprintln!("\nemberly: {error}");
        std::process::exit(1);
    }
}

/// Install the top-level panic hook (HC-3). Phase 1's line mode makes no
/// terminal changes to restore; the rich TUI (Phase 4) will restore the
/// terminal here, and Phase 5 will flush the transcript and append
/// `abnormal_exit`. For now, surface the panic calmly and keep the default
/// backtrace behavior.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        eprintln!("\nemberly: an unexpected internal error occurred (this is a bug).");
        eprintln!("session persistence and resume arrive in a later phase.");
        default_hook(info);
    }));
}

async fn run() -> anyhow::Result<()> {
    let mut force_plain = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--version" => {
                println!("emberly {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            // Force degraded/line mode (Design §7). Also implied by `NO_COLOR`,
            // `TERM=dumb`, and a non-tty stdout — see `frontend::detect`.
            "--plain" => force_plain = true,
            other => {
                anyhow::bail!("unknown argument: {other}");
            }
        }
    }

    let project_root = std::env::current_dir()?;

    // Resolve config (files + env + keys), then select a live provider or fall
    // back to the offline placeholder when none is configured.
    let resolved = config::load(&project_root)?;
    let (provider, model, label) = match provider_setup::build(&resolved)? {
        Some(selection) => (selection.provider, selection.model, selection.label),
        None => (
            Arc::new(PlaceholderProvider::new()) as Arc<dyn Provider>,
            "placeholder".to_string(),
            "placeholder (offline — set EMBERLY_PROVIDER + EMBERLY_MODEL + API key)".to_string(),
        ),
    };

    let kind = frontend::detect(force_plain);

    let session = SessionInfo {
        title: String::new(),
        provider: resolved.provider.clone().unwrap_or_default(),
        model: model.clone(),
        project_root: project_root.display().to_string(),
    };

    // In line mode the banner is the session header; the rich TUI shows the
    // same information in-pane (and the alternate screen would wipe stdout
    // anyway), so print it only in degraded mode.
    if kind == frontend::FrontendKind::Plain {
        // ASCII-only chrome in degraded mode (Design §7).
        println!("emberly code - running in {}", project_root.display());
        println!("model: {label}");
        println!("Ctrl-D to exit.");
        println!();
    }

    let config = EngineConfig {
        provider,
        tools: default_registry(),
        project_root,
        model,
        system: None,
        truncate: TruncateConfig::default(),
        retry: emberly_core::RetryPolicy::default(),
    };

    let (engine_ports, frontend_ports) = channel();
    let (engine, asks_rx) = Engine::new(config, engine_ports.events_tx);
    let engine_task = tokio::spawn(engine.run(engine_ports.commands_rx, asks_rx));

    // Drive the session until the user quits or the engine closes its events.
    // The frontend drops its command sender on quit, the engine finishes, and
    // its events channel closes. The terminal is restored by the TUI's guard
    // (HC-3) on every exit path, including panics.
    frontend::run(kind, frontend_ports, session).await?;

    match engine_task.await {
        Ok(()) => {}
        Err(join_error) if join_error.is_panic() => {
            eprintln!("emberly: the engine stopped unexpectedly. run again to continue.");
        }
        Err(_) => {}
    }

    // The terminal is back to normal here (guard dropped). A single closing
    // line; the richer clean-exit summary (name, duration, cost) is Phase 5.
    println!("session ended.");
    Ok(())
}
