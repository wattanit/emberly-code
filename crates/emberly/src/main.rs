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
use emberly_tui::line;

mod placeholder;
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
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--version" => {
                println!("emberly {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            // `--plain` is the only mode in Phase 1; accept it as a no-op so
            // scripts and the release smoke run work unchanged.
            "--plain" => {}
            other => {
                anyhow::bail!("unknown argument: {other}");
            }
        }
    }

    let project_root = std::env::current_dir()?;
    println!("emberly code — running in {}", project_root.display());
    println!(
        "Phase 1 scaffold: a placeholder model replies; live providers arrive in Phase 3. \
         Ctrl-D to exit."
    );
    println!();

    let provider: Arc<dyn Provider> = Arc::new(PlaceholderProvider::new());
    let config = EngineConfig {
        provider,
        tools: default_registry(),
        project_root,
        model: "placeholder".into(),
        system: None,
        truncate: TruncateConfig::default(),
    };

    let (engine_ports, frontend) = channel();
    let (engine, asks_rx) = Engine::new(config, engine_ports.events_tx);
    let engine_task = tokio::spawn(engine.run(engine_ports.commands_rx, asks_rx));

    // Drive the session until stdin closes; the frontend then drops its
    // command sender, the engine finishes, and its events channel closes.
    line::run(frontend).await?;

    match engine_task.await {
        Ok(()) => {}
        Err(join_error) if join_error.is_panic() => {
            eprintln!("emberly: the engine stopped unexpectedly. run again to continue.");
        }
        Err(_) => {}
    }

    println!("\nsession ended.");
    Ok(())
}
