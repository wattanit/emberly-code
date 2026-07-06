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

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use emberly_core::{channel, Engine, EngineConfig, FileTranscript, SandboxStatus, SessionId};
use emberly_providers::Provider;
use emberly_tools::{default_registry, TruncateConfig};
use emberly_tui::{frontend, SessionInfo};

mod config;
mod placeholder;
mod provider_setup;
use placeholder::PlaceholderProvider;

/// The active session's transcript path, set once the file is opened. Global so
/// the panic hook — which cannot reach the engine's live sink — can append an
/// `abnormal_exit` line and point the user at resume (HC-3, S-2).
static SESSION_PATH: OnceLock<PathBuf> = OnceLock::new();

#[tokio::main]
async fn main() {
    install_panic_hook();
    if let Err(error) = run().await {
        // A harness-world failure that may have interrupted a live session:
        // record it and point at resume (Design §6.1, §8.3).
        if let Some(path) = SESSION_PATH.get() {
            emberly_core::append_abnormal_exit(path, &error.to_string());
        }
        eprintln!("\nemberly: {error}");
        print_resume_hint();
        std::process::exit(1);
    }
}

/// Install the top-level panic hook (HC-3, S-2). The rich TUI's guard wraps this
/// to restore the terminal first (Phase 4); here we record the crash to the
/// transcript — the engine's live sink is unreachable from a panic hook, so we
/// append one `abnormal_exit` line directly (safe: every prior event was
/// fsynced) — then surface it calmly and point at resume.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(path) = SESSION_PATH.get() {
            emberly_core::append_abnormal_exit(path, "an internal error (panic) ended the session");
        }
        eprintln!("\nemberly: an unexpected internal error occurred (this is a bug).");
        print_resume_hint();
        default_hook(info);
    }));
}

/// Tell the user their session is recoverable (Design §8.3). No-op before a
/// session file exists.
fn print_resume_hint() {
    if let Some(path) = SESSION_PATH.get() {
        eprintln!(
            "your session was saved — run `emberly resume` to continue ({}).",
            path.display()
        );
    }
}

/// Human-friendly elapsed time for the clean-exit summary.
fn format_duration(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    format!("{}m{:02}s", secs / 60, secs % 60)
}

async fn run() -> anyhow::Result<()> {
    let started = Instant::now();
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

    // Open the durable, append-only transcript for this session (HC-7). A
    // failure here is non-fatal — the agent still runs, just unrecorded.
    let session_id = SessionId::new();
    let sessions_dir = project_root.join(".agents").join("sessions");
    let session_path = sessions_dir.join(format!("{session_id}.jsonl"));
    // Publish the path so the panic hook / error path can record an abnormal
    // exit and print the resume hint.
    let _ = SESSION_PATH.set(session_path.clone());
    let transcript: Box<dyn emberly_core::TranscriptSink> =
        match FileTranscript::create(&sessions_dir, session_id) {
            Ok(file) => Box::new(file),
            Err(error) => {
                eprintln!(
                    "emberly: could not open the session transcript ({error}); \
                     continuing without it."
                );
                EngineConfig::no_transcript()
            }
        };

    let config = EngineConfig {
        provider,
        tools: default_registry(),
        project_root,
        model,
        system: None,
        truncate: TruncateConfig::default(),
        retry: emberly_core::RetryPolicy::default(),
        session_id,
        provider_label: resolved
            .provider
            .clone()
            .unwrap_or_else(|| "placeholder".into()),
        // Real OS confinement arrives with Seatbelt (Phase 5 group 8) / Landlock
        // (Phase 2); until then the session records an honest "unavailable".
        sandbox: SandboxStatus::Unavailable {
            reason: "no OS sandbox configured yet".into(),
        },
        config_provenance: Vec::new(),
        transcript,
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

    // Clean exit (Design §8.3): the engine has recorded `session_end`; the
    // terminal is back to normal (guard dropped). One closing line with the
    // duration and where the transcript lives. (Title/cost live engine-side;
    // surfacing them here is a later refinement.)
    println!(
        "session ended · {} · transcript: {}",
        format_duration(started.elapsed()),
        session_path.display()
    );
    Ok(())
}
