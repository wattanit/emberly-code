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

use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Context;
use emberly_core::{
    channel, resume, Engine, EngineConfig, FileTranscript, Message, SandboxStatus, SessionId,
    TranscriptRecord, TranscriptSink,
};
use emberly_providers::Provider;
use emberly_tools::{default_registry, TruncateConfig};
use emberly_tui::{frontend, SessionInfo};

mod config;
mod init;
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

/// Box a transcript result, degrading to a no-op sink (with a notice) on error
/// so a transcript problem never stops the agent (HC-7).
fn open_transcript(result: std::io::Result<FileTranscript>) -> Box<dyn TranscriptSink> {
    match result {
        Ok(file) => Box::new(file),
        Err(error) => {
            eprintln!(
                "emberly: could not open the session transcript ({error}); continuing without it."
            );
            EngineConfig::no_transcript()
        }
    }
}

/// Offer to resume the most recent *interrupted* session in this project
/// (Design §8.3). Returns the chosen path only on an explicit "y" — never
/// auto-resumes. Silent when there is nothing interrupted to offer.
fn offer_resume(sessions_dir: &Path) -> Option<PathBuf> {
    let path = resume::latest_session(sessions_dir)?;
    let loaded = resume::read_records(&path).ok()?;
    if !resume::interrupted(&loaded.records) {
        return None;
    }
    let title = resume::session_title(&loaded.records).unwrap_or_else(|| "untitled".into());
    print!("Found an interrupted session \"{title}\" — resume? (y/N) ");
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    if std::io::stdin().read_line(&mut line).is_err() {
        return None;
    }
    line.trim().eq_ignore_ascii_case("y").then_some(path)
}

async fn run() -> anyhow::Result<()> {
    let started = Instant::now();
    let mut force_plain = false;
    let mut resume_requested = false;
    let mut resume_id: Option<String> = None;
    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" => {
                println!("emberly {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            // Force degraded/line mode (Design §7). Also implied by `NO_COLOR`,
            // `TERM=dumb`, and a non-tty stdout — see `frontend::detect`.
            "--plain" => force_plain = true,
            // `emberly resume [id]` (Tech Spec §3.3). An id may follow.
            "resume" => {
                resume_requested = true;
                if let Some(next) = args.peek() {
                    if !next.starts_with('-') {
                        resume_id = args.next();
                    }
                }
            }
            // `emberly init` — materialize `.agents/` defaults (C-2).
            "init" => {
                init::init(&std::env::current_dir()?)?;
                return Ok(());
            }
            // `emberly config show` (C-3).
            "config" => match args.next().as_deref() {
                Some("show") => {
                    config::show(&std::env::current_dir()?)?;
                    return Ok(());
                }
                other => anyhow::bail!(
                    "unknown config subcommand: {} (try `config show`)",
                    other.unwrap_or("(none)")
                ),
            },
            other => {
                anyhow::bail!("unknown argument: {other}");
            }
        }
    }

    let project_root = std::env::current_dir()?;
    let sessions_dir = project_root.join(".agents").join("sessions");

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

    // Decide which session to resume, if any (Tech Spec §3.3, Design §8.3).
    // Explicit `resume [id]` wins; otherwise, on an interactive launch, offer to
    // resume the most recent *interrupted* session — never auto-resume.
    let resume_path: Option<PathBuf> = if resume_requested {
        Some(match &resume_id {
            Some(id) => sessions_dir.join(format!("{id}.jsonl")),
            None => resume::latest_session(&sessions_dir)
                .context("no sessions found to resume in .agents/sessions")?,
        })
    } else if std::io::stdin().is_terminal() {
        offer_resume(&sessions_dir)
    } else {
        None
    };

    // Build the session: either resumed (restore the conversation, append to the
    // same transcript file) or fresh (new id + file). Transcript failures are
    // non-fatal — the agent still runs, just unrecorded (HC-7).
    let mut history: Vec<TranscriptRecord> = Vec::new();
    let mut initial_conversation: Vec<Message> = Vec::new();
    let resuming;
    let mut title = String::new();
    let session_id;
    let session_path;
    let transcript: Box<dyn TranscriptSink>;

    if let Some(path) = resume_path {
        let loaded = resume::read_records(&path)
            .with_context(|| format!("cannot read session {}", path.display()))?;
        for warning in &loaded.warnings {
            eprintln!("emberly: {warning}");
        }
        initial_conversation = resume::rebuild_conversation(&loaded.records);
        title = resume::session_title(&loaded.records).unwrap_or_default();
        session_id = resume::session_id(&loaded.records).unwrap_or_default();
        transcript = open_transcript(FileTranscript::open(&path));
        session_path = path;
        history = loaded.records;
        resuming = true;
    } else {
        session_id = SessionId::new();
        session_path = sessions_dir.join(format!("{session_id}.jsonl"));
        transcript = open_transcript(FileTranscript::create(&sessions_dir, session_id));
        resuming = false;
    }
    // Publish the path so the panic hook / error path can record an abnormal
    // exit and print the resume hint.
    let _ = SESSION_PATH.set(session_path.clone());

    let session = SessionInfo {
        title,
        provider: resolved.provider.clone().unwrap_or_default(),
        model: model.clone(),
        project_root: project_root.display().to_string(),
    };

    // In line mode the banner is the session header; the rich TUI shows the
    // same information in-pane, so print it only in degraded mode.
    if kind == frontend::FrontendKind::Plain {
        // ASCII-only chrome in degraded mode (Design §7).
        println!("emberly code - running in {}", project_root.display());
        println!("model: {label}");
        if resuming {
            println!("resumed session ({} earlier events)", history.len());
        }
        // Silence about defaults; speech about deviations (Design §8.2).
        for notice in &resolved.notices {
            println!("note: {notice}");
        }
        for entry in &resolved.provenance {
            println!("  config: {} <- {}", entry.piece, entry.source);
        }
        println!("Ctrl-D to exit.");
        println!();
    }

    let config = EngineConfig {
        provider,
        tools: default_registry(),
        project_root,
        model,
        system: resolved.system_prompt.clone(),
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
        config_provenance: resolved.provenance.clone(),
        transcript,
        initial_conversation,
        resuming,
        summary_prompt: resolved.summary_prompt.clone(),
    };

    let (engine_ports, frontend_ports) = channel();
    let (engine, asks_rx) = Engine::new(config, engine_ports.events_tx);
    let engine_task = tokio::spawn(engine.run(engine_ports.commands_rx, asks_rx));

    // Drive the session until the user quits or the engine closes its events.
    // The frontend drops its command sender on quit, the engine finishes, and
    // its events channel closes. The terminal is restored by the TUI's guard
    // (HC-3) on every exit path, including panics.
    frontend::run(kind, frontend_ports, session, history).await?;

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
