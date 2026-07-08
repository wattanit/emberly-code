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
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, Instant};

use anyhow::Context;
use emberly_core::{
    channel, resume, Engine, EngineConfig, FileTranscript, Message, RuleEngine, SessionId,
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

/// The active session's transcript path — a shared handle the engine updates on
/// every in-session switch (`/new`, `/resume`), so the panic hook and error
/// path (which cannot reach the engine's live sink) always append the
/// `abnormal_exit` line to the *current* session (HC-3, S-2).
static SESSION_PATH: OnceLock<Arc<RwLock<PathBuf>>> = OnceLock::new();

/// The current session transcript path, if a session file has been opened.
fn current_session_path() -> Option<PathBuf> {
    let handle = SESSION_PATH.get()?;
    handle.read().ok().map(|path| path.clone())
}

fn main() {
    // Self-exec Landlock shim (Phase 2 group 3): if we were re-executed as the
    // confined-exec subcommand, restrict this process and exec the command —
    // BEFORE any async runtime or worker threads exist, because Landlock's
    // `restrict_self` is per-thread and the restricting thread must be the one
    // that execs. A normal launch falls straight through.
    maybe_run_sandbox_shim();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("emberly: could not start the async runtime: {error}");
            std::process::exit(1);
        }
    };
    runtime.block_on(async_main());
}

async fn async_main() {
    install_panic_hook();
    if let Err(error) = run().await {
        // A harness-world failure that may have interrupted a live session:
        // record it and point at resume (Design §6.1, §8.3).
        if let Some(path) = current_session_path() {
            emberly_core::append_abnormal_exit(&path, &error.to_string());
        }
        eprintln!("\nemberly: {error}");
        print_resume_hint();
        std::process::exit(1);
    }
}

/// If argv is the confined-exec shim invocation, apply the Landlock ruleset and
/// `exec` the command — never returning on success (Phase 2 group 3). Fails
/// closed: any setup problem exits non-zero rather than running unconfined. A
/// no-op for a normal launch and on non-Linux.
#[cfg(target_os = "linux")]
fn maybe_run_sandbox_shim() {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some(emberly_sandbox::SANDBOX_EXEC_ARG) {
        return;
    }
    let Some(command) = args.next() else {
        eprintln!("emberly: sandbox shim invoked without a command");
        std::process::exit(127);
    };
    let Some(spec) = emberly_sandbox::SandboxSpec::from_env() else {
        eprintln!("emberly: sandbox shim invoked without a valid confinement spec");
        std::process::exit(127);
    };
    // Never returns on success (the process image is replaced under the fence).
    let error = emberly_sandbox::confine::exec_confined(&spec, &command);
    eprintln!("emberly: sandbox shim failed: {error}");
    std::process::exit(127);
}

#[cfg(not(target_os = "linux"))]
fn maybe_run_sandbox_shim() {}

/// Install the top-level panic hook (HC-3, S-2). The rich TUI's guard wraps this
/// to restore the terminal first (Phase 4); here we record the crash to the
/// transcript — the engine's live sink is unreachable from a panic hook, so we
/// append one `abnormal_exit` line directly (safe: every prior event was
/// fsynced) — then surface it calmly and point at resume.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(path) = current_session_path() {
            emberly_core::append_abnormal_exit(
                &path,
                "an internal error (panic) ended the session",
            );
        }
        eprintln!("\nemberly: an unexpected internal error occurred (this is a bug).");
        print_resume_hint();
        default_hook(info);
    }));
}

/// Tell the user their session is recoverable (Design §8.3). No-op before a
/// session file exists.
fn print_resume_hint() {
    if let Some(path) = current_session_path() {
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

/// Coarse "how long ago" for the session list — the exact minute rarely matters,
/// the day and hour do.
fn format_ago(modified: std::time::SystemTime) -> String {
    let Ok(elapsed) = modified.elapsed() else {
        return "just now".into();
    };
    let secs = elapsed.as_secs();
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

/// `emberly sessions` — list this project's saved sessions, newest first, with
/// the id needed to resume each (Design §8.3, items 1–2).
fn list_sessions(sessions_dir: &Path) {
    let sessions = resume::list_sessions(sessions_dir);
    if sessions.is_empty() {
        println!("No sessions yet in {}.", sessions_dir.display());
        println!("Start one with `emberly`; it is saved automatically.");
        return;
    }
    println!("Sessions in {} (newest first):", sessions_dir.display());
    println!();
    for s in &sessions {
        let title = s.title.as_deref().unwrap_or("(untitled)");
        let flag = if s.interrupted { " · interrupted" } else { "" };
        println!("  {title}");
        println!(
            "    {} · {}/{} · {} · {} events{flag}",
            s.id,
            s.provider,
            s.model,
            format_ago(s.modified),
            s.events,
        );
        println!("    resume: emberly resume {}", s.id);
        println!();
    }
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

/// The parsed command line (Tech Spec §10).
#[derive(Debug, PartialEq, Eq)]
enum Cli {
    Version,
    Init,
    ConfigShow,
    Sessions,
    Run(RunOpts),
}

/// Options for a session run.
#[derive(Debug, Default, PartialEq, Eq)]
struct RunOpts {
    force_plain: bool,
    resume: bool,
    resume_id: Option<String>,
    provider: Option<String>,
    model: Option<String>,
}

/// Parse argv (without the program name). Subcommands (`init`, `config show`,
/// `--version`) short-circuit; flags accumulate into a [`RunOpts`].
fn parse_args(args: impl Iterator<Item = String>) -> anyhow::Result<Cli> {
    let mut args = args.peekable();
    let mut opts = RunOpts::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" => return Ok(Cli::Version),
            "init" => return Ok(Cli::Init),
            "sessions" => return Ok(Cli::Sessions),
            "config" => match args.next().as_deref() {
                Some("show") => return Ok(Cli::ConfigShow),
                other => anyhow::bail!(
                    "unknown config subcommand: {} (try `config show`)",
                    other.unwrap_or("(none)")
                ),
            },
            // Force degraded/line mode (Design §7); also implied by `NO_COLOR`,
            // `TERM=dumb`, and a non-tty stdout — see `frontend::detect`.
            "--plain" => opts.force_plain = true,
            // `resume [id]` — an id may follow (Tech Spec §3.3).
            "resume" => {
                opts.resume = true;
                if let Some(next) = args.peek() {
                    if !next.starts_with('-') {
                        opts.resume_id = args.next();
                    }
                }
            }
            "--provider" => {
                opts.provider = Some(args.next().context("--provider needs a value")?);
            }
            "--model" => {
                opts.model = Some(args.next().context("--model needs a value")?);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok(Cli::Run(opts))
}

async fn run() -> anyhow::Result<()> {
    let started = Instant::now();
    let opts = match parse_args(std::env::args().skip(1))? {
        Cli::Version => {
            println!("emberly {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Cli::Init => {
            init::init(&std::env::current_dir()?)?;
            return Ok(());
        }
        Cli::ConfigShow => {
            config::show(&std::env::current_dir()?)?;
            return Ok(());
        }
        Cli::Sessions => {
            let sessions_dir = std::env::current_dir()?.join(".agents").join("sessions");
            list_sessions(&sessions_dir);
            return Ok(());
        }
        Cli::Run(opts) => opts,
    };
    let force_plain = opts.force_plain;
    let resume_requested = opts.resume;
    let resume_id = opts.resume_id;
    let cli_overrides = config::CliOverrides {
        provider: opts.provider,
        model: opts.model,
    };

    let project_root = std::env::current_dir()?;
    let sessions_dir = project_root.join(".agents").join("sessions");
    // First run without any project config still just works on defaults; point
    // at `emberly init` (Design §8.1).
    let initialized = project_root.join(".agents").join("config.toml").exists();

    // Resolve config (files + env + CLI + keys), then select a live provider or
    // fall back to the offline placeholder when none is configured.
    let resolved = config::load(&project_root, &cli_overrides)?;

    // Probe OS confinement (Requirements §6.7) and honor `sandbox.require`
    // *before* any session is opened, so a refusal doesn't leave a stray
    // transcript. Landlock confines children on Linux and Seatbelt
    // (`sandbox-exec`) on macOS; on any other platform (or when the backend is
    // blocked) the probe reports an honest `unavailable` and the harness runs
    // the degraded path.
    let sandbox = emberly_core::probe();
    if resolved.sandbox_require && !sandbox.is_confined() {
        anyhow::bail!(
            "sandbox.require is set but OS confinement is not active ({}). \
             Refusing to start (Requirements §6.7). Unset sandbox.require to run degraded.",
            match &sandbox {
                emberly_core::SandboxStatus::Unavailable { reason } => reason.as_str(),
                _ => "partial",
            }
        );
    }

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
    // Publish the path through a shared handle so the panic hook / error path
    // record the abnormal exit against the current session — and so the engine
    // can keep it current across in-session switches (`/new`, `/resume`).
    let active_session_path = Arc::new(RwLock::new(session_path.clone()));
    let _ = SESSION_PATH.set(active_session_path.clone());

    let session = SessionInfo {
        session_id,
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
        if !initialized {
            println!("no project config yet — running on defaults; `emberly init` materializes it");
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

    // Build the permission rule engine: built-in defaults + config rules, with
    // the bash allowlist suspended when confinement is unavailable (§6.7).
    let (rule_specs, rule_warnings) = config::load_permission_rules(&project_root);
    for warning in &rule_warnings {
        eprintln!("emberly: {warning}");
    }
    let rules = RuleEngine::new(rule_specs, sandbox.bash_allowlist_active());

    // Build the confined-spawn handle: record the canonical git binary now
    // (`which git`, canonicalized) so only genuine git earns the `.git/`-writable
    // profile under confinement (§6.4). Nothing is `.git/`-writable when degraded.
    let path_env = std::env::var("PATH").unwrap_or_default();
    let git_binary = if sandbox.is_confined() {
        emberly_sandbox::git::record_git_binary(&path_env)
    } else {
        None
    };
    let sandbox_spawn: Arc<dyn emberly_tools::Sandbox> = Arc::new(
        emberly_core::spawn::HostSandbox::new(sandbox.is_confined(), git_binary, path_env),
    );

    let config = EngineConfig {
        provider,
        tools: default_registry(),
        project_root,
        model,
        system: resolved.system_prompt.clone(),
        truncate: TruncateConfig::default(),
        retry: emberly_core::RetryPolicy::default(),
        session_id,
        sessions_dir: sessions_dir.clone(),
        active_session_path: active_session_path.clone(),
        provider_label: resolved
            .provider
            .clone()
            .unwrap_or_else(|| "placeholder".into()),
        sandbox,
        rules,
        sandbox_spawn: Some(sandbox_spawn),
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
    frontend::run(kind, frontend_ports, session, history, sessions_dir.clone()).await?;

    match engine_task.await {
        Ok(()) => {}
        Err(join_error) if join_error.is_panic() => {
            eprintln!("emberly: the engine stopped unexpectedly. run again to continue.");
        }
        Err(_) => {}
    }

    // Clean exit (Design §8.3): the engine has recorded `session_end`; the
    // terminal is back to normal (guard dropped). One closing line with the
    // duration and where the transcript lives. The current session may differ
    // from the one we launched (an in-session `/new` or `/resume`), so read the
    // final path from the shared handle rather than the launch-time local.
    let final_path = current_session_path().unwrap_or(session_path);
    let final_id = final_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("session");
    println!(
        "session ended · {} · session {final_id}",
        format_duration(started.elapsed()),
    );
    println!("  transcript: {}", final_path.display());
    println!("  to resume:  emberly resume {final_id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&str]) -> anyhow::Result<Cli> {
        parse_args(argv.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn subcommands_short_circuit() {
        assert_eq!(parse(&["--version"]).unwrap(), Cli::Version);
        assert_eq!(parse(&["init"]).unwrap(), Cli::Init);
        assert_eq!(parse(&["config", "show"]).unwrap(), Cli::ConfigShow);
        assert_eq!(parse(&["sessions"]).unwrap(), Cli::Sessions);
    }

    #[test]
    fn flags_accumulate_into_run_opts() {
        let cli = parse(&["--plain", "--model", "gpt-5.2", "--provider", "openai"]).unwrap();
        assert_eq!(
            cli,
            Cli::Run(RunOpts {
                force_plain: true,
                resume: false,
                resume_id: None,
                provider: Some("openai".into()),
                model: Some("gpt-5.2".into()),
            })
        );
    }

    #[test]
    fn resume_takes_an_optional_id() {
        assert_eq!(
            parse(&["resume"]).unwrap(),
            Cli::Run(RunOpts {
                resume: true,
                ..RunOpts::default()
            })
        );
        assert_eq!(
            parse(&["resume", "abc123"]).unwrap(),
            Cli::Run(RunOpts {
                resume: true,
                resume_id: Some("abc123".into()),
                ..RunOpts::default()
            })
        );
    }

    #[test]
    fn errors_are_clean() {
        assert!(parse(&["--model"]).is_err(), "missing value");
        assert!(parse(&["bogus"]).is_err(), "unknown argument");
        assert!(parse(&["config", "nope"]).is_err(), "unknown subcommand");
    }
}
