//! The line-mode frontend (Tech Spec §9 degraded contract, A-1). Plain,
//! append-only, ASCII-only output and line-buffered input — no color, no
//! cursor repositioning. It is the seed of `--plain`/degraded mode and the
//! contract for a future headless frontend; the rich `ratatui` TUI arrives in
//! Phase 4.
//!
//! Rendering ([`LineRenderer::render`]) is a pure function over a writer, so it
//! is unit-testable without a terminal. The async [`run`] driver wires stdin
//! lines and [`UiEvent`]s to the engine's channels.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use emberly_core::{
    AskAnswer, AskId, CheckResult, Command, EntrySummary, FrontendPorts, GateResolution,
    LoopResolution, MemoryOp, MemoryScope, Mode, PermissionDecision, PermissionId,
    PermissionRendering, SandboxStatus, SkillMeta, SkillOrigin, UiEvent,
};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::app::ReasoningView;

/// Renders [`UiEvent`]s as append-only lines. Holds no color or cursor state
/// (degraded mode, Design §7); the only state is the reasoning-block toggle so
/// the plain `--- reasoning ---` block is labeled once (Design §4.4).
pub struct LineRenderer {
    reasoning_view: ReasoningView,
    /// True while streaming a reasoning block, so the answer that follows gets a
    /// separating label.
    in_reasoning: bool,
}

impl LineRenderer {
    #[must_use]
    pub fn new(reasoning_view: ReasoningView) -> Self {
        Self {
            reasoning_view,
            in_reasoning: false,
        }
    }

    /// Write one event to `out`. Assistant deltas stream without a trailing
    /// newline; `AssistantDone` closes the line.
    pub fn render(&mut self, event: &UiEvent, out: &mut impl Write) -> io::Result<()> {
        match event {
            UiEvent::AssistantDelta { text } => {
                // Close a preceding reasoning block with a plain label so the
                // answer is never confused with the thinking (Design §4.4, §7).
                if self.in_reasoning {
                    writeln!(out, "\n--- answer ---")?;
                    self.in_reasoning = false;
                }
                write!(out, "{text}")?;
            }
            UiEvent::ReasoningDelta { text } => {
                // `hidden` suppresses the trail in the view; the trace is still
                // recorded by the engine (P-10).
                if self.reasoning_view != ReasoningView::Hidden {
                    if !self.in_reasoning {
                        writeln!(out, "--- reasoning ---")?;
                        self.in_reasoning = true;
                    }
                    write!(out, "{text}")?;
                }
            }
            UiEvent::AssistantDone => {
                self.in_reasoning = false;
                writeln!(out)?;
            }
            UiEvent::ToolStarted {
                tool,
                summary,
                explanation,
                ..
            } => {
                writeln!(out, "\n> {tool}: {summary}")?;
                // The model's caption (T-9), indented under the call. ASCII
                // lead so degraded mode carries it without a glyph or colour
                // (Design §4.5/§7); absent → nothing.
                if let Some(explanation) = explanation {
                    writeln!(out, "    - {explanation}")?;
                }
            }
            UiEvent::ToolFinished {
                call_id: _,
                ok,
                summary,
                preview,
                untrusted,
            } => {
                let tag = if *ok { "ok" } else { "FAILED" };
                writeln!(out, "  [{tag}] {summary}")?;
                // Untrusted web content (T-14, Design §4.10): labeled as fetched
                // web data with visible source URLs, ASCII-only — the "untrusted
                // web content" meaning must survive without colour (Design §7).
                if *untrusted {
                    writeln!(out, "    [web]")?;
                }
                for line in preview.lines() {
                    writeln!(out, "    {line}")?;
                }
            }
            UiEvent::FileModified { path, adds, dels } => {
                writeln!(out, "  ~ {path} (+{adds} -{dels})")?;
            }
            UiEvent::FileDiff { unified, .. } => {
                // The +/- prefixes carry the change without colour (Design §7).
                for line in unified.lines() {
                    writeln!(out, "  {line}")?;
                }
            }
            UiEvent::PermissionRequest { rendering, .. } => {
                self.render_permission(rendering, out)?
            }
            UiEvent::AskUserRequest {
                question, options, ..
            } => self.render_ask(question, options, out)?,
            UiEvent::LoopHalted { reason } => self.render_loop_halt(reason, out)?,
            UiEvent::CompletionGateHalted { failing, attempts } => {
                self.render_completion_gate(failing, *attempts, out)?
            }
            UiEvent::HarnessError { what, why, next } => {
                writeln!(out, "\nerror: {what}")?;
                writeln!(out, "  why:  {why}")?;
                writeln!(out, "  next: {next}")?;
            }
            UiEvent::Retrying {
                attempt,
                max_attempts,
                delay_ms,
                reason,
            } => {
                writeln!(
                    out,
                    "  ... retrying ({attempt}/{max_attempts}) in {delay_ms}ms - {reason}"
                )?;
            }
            UiEvent::SessionMeta {
                title,
                provider,
                model,
                project_root,
                ..
            } => {
                writeln!(
                    out,
                    "session: {title}  [{provider}/{model}]  {project_root}"
                )?;
            }
            UiEvent::SandboxStatus { status } => {
                let summary = match status {
                    SandboxStatus::Confined { backend } => backend.clone(),
                    SandboxStatus::Partial { backend, missing } => {
                        format!("{backend} (partial: {missing})")
                    }
                    SandboxStatus::Unavailable { reason } => format!("unavailable ({reason})"),
                };
                writeln!(out, "sandbox: {summary}")?;
            }
            UiEvent::ModeChanged { mode } => {
                let word = match mode {
                    Mode::Normal => "normal",
                    Mode::AutoAcceptEdits => "auto-accept edits",
                    Mode::Auto => "auto",
                };
                writeln!(out, "mode: {word}")?;
            }
            UiEvent::Notice { message } => {
                // Harness-voice notice: the degraded-sandbox explanation, a
                // saved permission grant, a refused mode switch (Design §8.2).
                for line in message.lines() {
                    writeln!(out, "note: {line}")?;
                }
            }
            // Memory inspector, degraded form (FR-6, Design §4.9, §7): the
            // grouped list printed inline, read-only, ASCII — inspection is not
            // rich-only. `/memory` requests this; unsolicited it never fires.
            UiEvent::MemoryEntries { user, project } => {
                self.render_memory_entries(user, project, out)?;
            }
            UiEvent::MemoryBody { scope, name, body } => {
                self.render_memory_body(*scope, name, body, out)?;
            }
            // Skills inspector body, degraded form (FR-7, §4.9): read-only, ASCII
            // — "what could this skill tell the model to do" before it ever runs.
            UiEvent::SkillBody {
                name,
                origin,
                body,
                resources,
            } => {
                self.render_skill_body(name, *origin, body, resources, out)?;
            }
            UiEvent::CompactionStatus { .. }
            | UiEvent::ContextUsage { .. }
            | UiEvent::CostEstimate { .. }
            | UiEvent::SessionUsage { .. } => {}
            UiEvent::TaskListUpdated { items } => {
                // ASCII checklist — the plain mode has no sidebar, so the
                // inline block is the only surface (Design §7).
                writeln!(out)?;
                for item in items {
                    let mark = match item.status {
                        emberly_core::TaskStatus::Pending => "[ ]",
                        emberly_core::TaskStatus::InProgress => "[~]",
                        emberly_core::TaskStatus::Done => "[x]",
                    };
                    writeln!(out, "  {mark} {}", item.text)?;
                }
            }
            // Unknown future events are ignored (non_exhaustive).
            _ => {}
        }
        Ok(())
    }

    /// The permission prompt in degraded form (Design §5, §7): full content
    /// never truncated, an OUTSIDE-PROJECT banner in capitals, deny as the
    /// default (Enter), and a distinct, deliberate approve key.
    fn render_permission(
        &self,
        rendering: &PermissionRendering,
        out: &mut impl Write,
    ) -> io::Result<()> {
        use crate::strings::permission as p;
        writeln!(out)?;
        if rendering.outside_root {
            // Capitalised banner carries the meaning colour would in rich mode
            // (Design §7); shared verbatim with the TUI for parity.
            writeln!(out, "!! {} !!", p::OUTSIDE_ROOT_BANNER)?;
        }
        writeln!(out, "{}: {}", p::HEADING, rendering.summary)?;
        writeln!(out, "  {}: {}", p::WHY_LABEL, rendering.reason)?;
        if !rendering.affected_paths.is_empty() {
            writeln!(
                out,
                "  {}: {}",
                p::PATHS_LABEL,
                rendering.affected_paths.join(", ")
            )?;
        }
        writeln!(out, "--- details ---")?;
        writeln!(out, "{}", rendering.detail.trim_end())?;
        writeln!(out, "---------------")?;
        writeln!(
            out,
            "Allow?  [y] {}   [s] {}   [Enter] {}",
            p::ALLOW_ONCE,
            p::ALLOW_SESSION,
            p::DENY
        )?;
        Ok(())
    }

    /// The `ask_user` question prompt in degraded form (T-8, Design §5.1, §7):
    /// calm — **no** capitals safety banner (this is not a safety prompt) — the
    /// question, numbered options if any, and a plain instruction. There is no
    /// unsafe default: an empty line declines, a number picks an option, any
    /// other text is a free-form answer.
    fn render_ask(
        &self,
        question: &str,
        options: &[String],
        out: &mut impl Write,
    ) -> io::Result<()> {
        use crate::strings::ask_user as a;
        writeln!(out)?;
        writeln!(out, "{}: {}", a::HEADING, question)?;
        for (i, opt) in options.iter().enumerate() {
            writeln!(out, "  {}. {}", i + 1, opt)?;
        }
        if options.is_empty() {
            writeln!(out, "{}", a::DECLINE_HINT)?;
        } else {
            writeln!(out, "answer, or an option number; {}", a::DECLINE_HINT)?;
        }
        Ok(())
    }

    /// The loop-halt surface in degraded form (S-5, Design §8.5, §7): the
    /// harness voice, calm — what happened, then the choices. No alarm styling.
    fn render_loop_halt(&self, reason: &str, out: &mut impl Write) -> io::Result<()> {
        use crate::strings::loop_halt as s;
        writeln!(out)?;
        writeln!(out, "{}", s::HEADING)?;
        writeln!(out, "  {reason}")?;
        writeln!(out, "{}", s::LINE_PROMPT)?;
        Ok(())
    }

    /// The completion-gate halt surface in degraded form (S-6, Design §8.7,
    /// §7): the harness voice, calm — the failing checks, the attempt count,
    /// then the choices. No alarm styling; "finish anyway" is offered as an
    /// override, never as though the checks passed.
    fn render_completion_gate(
        &self,
        failing: &[CheckResult],
        attempts: usize,
        out: &mut impl Write,
    ) -> io::Result<()> {
        use crate::strings::completion_gate as s;
        writeln!(out)?;
        writeln!(out, "{}", s::HEADING)?;
        writeln!(
            out,
            "  {attempts} attempt{} —",
            if attempts == 1 { "" } else { "s" }
        )?;
        for check in failing {
            writeln!(
                out,
                "    {}: {}",
                check.name,
                check.reason.lines().next().unwrap_or("")
            )?;
        }
        writeln!(out, "{}", s::LINE_PROMPT)?;
        Ok(())
    }

    /// The memory list in degraded form (FR-6, Design §4.9, §7): grouped by
    /// scope with an ASCII header per group (origin is how the user reads
    /// trust), `name - description` per entry. The project group is simply
    /// absent on an untrusted root (FR-1). Read-only inline text.
    fn render_memory_entries(
        &self,
        user: &[EntrySummary],
        project: &[EntrySummary],
        out: &mut impl Write,
    ) -> io::Result<()> {
        use crate::strings::memory as m;
        writeln!(out)?;
        if user.is_empty() && project.is_empty() {
            writeln!(out, "{}", m::EMPTY)?;
            return Ok(());
        }
        for (entries, header) in [(user, m::HEADER_USER), (project, m::HEADER_PROJECT)] {
            if entries.is_empty() {
                continue;
            }
            writeln!(out, "{header}:")?;
            for e in entries {
                if e.description.is_empty() {
                    writeln!(out, "  {}", e.name)?;
                } else {
                    writeln!(out, "  {} - {}", e.name, e.description)?;
                }
            }
        }
        Ok(())
    }

    /// A single memory entry's body in degraded form (FR-6, §4.6). Read-only
    /// inline text bracketed by ASCII rules; the body is fetched on demand
    /// (progressive disclosure — never pinned).
    fn render_memory_body(
        &self,
        scope: MemoryScope,
        name: &str,
        body: &str,
        out: &mut impl Write,
    ) -> io::Result<()> {
        use crate::strings::memory as m;
        let s = match scope {
            MemoryScope::User => m::SCOPE_USER,
            MemoryScope::Project => m::SCOPE_PROJECT,
        };
        writeln!(out, "\n--- memory: {name} ({s}) ---")?;
        if body.trim().is_empty() {
            writeln!(out, "{}", m::EMPTY_BODY)?;
        } else {
            for line in body.lines() {
                writeln!(out, "{line}")?;
            }
        }
        writeln!(out, "---")?;
        Ok(())
    }

    /// A skill's instruction body in degraded form (FR-7, §4.9): read-only
    /// inline text with the origin and any bundled resource paths — "what could
    /// this skill tell the model to do" before it ever runs. Displaying the body
    /// runs no bundled script (FR-7).
    fn render_skill_body(
        &self,
        name: &str,
        origin: SkillOrigin,
        body: &str,
        resources: &[String],
        out: &mut impl Write,
    ) -> io::Result<()> {
        use crate::strings::skills as s;
        let o = match origin {
            SkillOrigin::User => s::ORIGIN_USER,
            SkillOrigin::Project => s::ORIGIN_PROJECT,
        };
        writeln!(out, "\n--- skill: {name} ({o}) ---")?;
        if body.trim().is_empty() {
            writeln!(out, "{}", s::EMPTY_BODY)?;
        } else {
            for line in body.lines() {
                writeln!(out, "{line}")?;
            }
        }
        if !resources.is_empty() {
            writeln!(out, "{}", s::RESOURCES_HEADER)?;
            for r in resources {
                writeln!(out, "  {r}")?;
            }
        }
        writeln!(out, "---")?;
        Ok(())
    }
}

/// The available skills in degraded form (FR-7, Design §4.9, §7): `name -
/// description (origin)` per line, read-only inline text. Free function (not a
/// [`LineRenderer`] method) because the plain frontend holds the catalog in the
/// run loop, not the renderer — the skills list is standing state, not an event.
fn render_skill_list(skills: &[SkillMeta], out: &mut impl Write) -> io::Result<()> {
    use crate::strings::skills as s;
    writeln!(out)?;
    if skills.is_empty() {
        writeln!(out, "{}", s::EMPTY)?;
        return Ok(());
    }
    for skill in skills {
        let origin = match skill.origin {
            SkillOrigin::User => s::ORIGIN_USER,
            SkillOrigin::Project => s::ORIGIN_PROJECT,
        };
        if skill.description.is_empty() {
            writeln!(out, "  {} ({origin})", skill.name)?;
        } else {
            writeln!(out, "  {} - {} ({origin})", skill.name, skill.description)?;
        }
    }
    Ok(())
}

/// Resolve a memory entry name to its scope from the last-listed entries (user
/// first, then project). `None` when the name is unknown — the plain frontend
/// asks the user to `/memory` first so the list is current.
fn resolve_memory_scope(
    name: &str,
    user: &[EntrySummary],
    project: &[EntrySummary],
) -> Option<MemoryScope> {
    if user.iter().any(|e| e.name == name) {
        Some(MemoryScope::User)
    } else if project.iter().any(|e| e.name == name) {
        Some(MemoryScope::Project)
    } else {
        None
    }
}

/// The display label for a memory scope (parity with the rich inspector).
fn scope_label(scope: MemoryScope) -> &'static str {
    match scope {
        MemoryScope::User => crate::strings::memory::SCOPE_USER,
        MemoryScope::Project => crate::strings::memory::SCOPE_PROJECT,
    }
}

impl Default for LineRenderer {
    fn default() -> Self {
        Self::new(ReasoningView::default())
    }
}

/// The next auto-accept tier in the cycle (normal → auto-accept-edits → auto →
/// normal). The engine still gates the auto tiers on confinement (Tech Spec
/// §6.6); this only proposes the next one.
#[must_use]
pub fn next_mode(current: Mode) -> Mode {
    match current {
        Mode::Normal => Mode::AutoAcceptEdits,
        Mode::AutoAcceptEdits => Mode::Auto,
        Mode::Auto => Mode::Normal,
    }
}

/// Interpret a permission answer line. Deny is the safe default: only an
/// explicit, deliberate key allows (Design §5 — Enter/anything else denies).
#[must_use]
pub fn parse_permission_answer(line: &str) -> PermissionDecision {
    match line.trim().to_lowercase().as_str() {
        "y" | "yes" => PermissionDecision::AllowOnce,
        "s" | "session" => PermissionDecision::AllowForSession,
        _ => PermissionDecision::Deny,
    }
}

/// Interpret an `ask_user` answer line (T-8, Design §5.1). No unsafe default:
/// an empty line declines; a bare 1-based number selects an offered option;
/// anything else is a free-form answer.
#[must_use]
pub fn parse_ask_answer(line: &str, options: &[String]) -> AskAnswer {
    let text = line.trim();
    if text.is_empty() {
        return AskAnswer::Declined;
    }
    if let Ok(n) = text.parse::<usize>() {
        if let Some(opt) = n.checked_sub(1).and_then(|i| options.get(i)) {
            return AskAnswer::Answered(opt.clone());
        }
    }
    AskAnswer::Answered(text.to_string())
}

/// Interpret a loop-halt answer line (S-5, Design §8.5). `keep`/`go`/`resume`/
/// `1` → resume; `stop`/`2` → stop; an empty line → stop (never keep spending
/// unattended); anything else is a steer message handed back to the model.
#[must_use]
pub fn parse_loop_resolution(line: &str) -> LoopResolution {
    let text = line.trim();
    match text.to_lowercase().as_str() {
        "keep" | "go" | "resume" | "keep going" | "1" => LoopResolution::Resume,
        "stop" | "2" | "" => LoopResolution::Stop,
        _ => LoopResolution::Steer(text.to_string()),
    }
}

/// Interpret a completion-gate halt answer line (S-6, Design §8.7). `keep`/
/// `go`/`resume`/`1` → resume; `stop`/`2` → stop; an empty line → stop (never
/// keep spending unattended); `finish`/`f`/`4` → the explicit override; anything
/// else is a steer message handed back to the model.
#[must_use]
pub fn parse_gate_resolution(line: &str) -> GateResolution {
    let text = line.trim();
    match text.to_lowercase().as_str() {
        "keep" | "go" | "resume" | "keep going" | "1" => GateResolution::Resume,
        "stop" | "2" | "" => GateResolution::Stop,
        "finish" | "f" | "finish anyway" | "4" => GateResolution::Finish,
        _ => GateResolution::Steer(text.to_string()),
    }
}

/// A decision prompt awaiting the next stdin line. Only one is ever open at a
/// time (the engine serializes tool calls), but keeping them in one enum makes
/// it impossible for a permission answer and a question answer to cross wires.
enum Pending {
    Permission(PermissionId),
    Ask {
        id: AskId,
        options: Vec<String>,
    },
    Loop,
    /// The completion-gate halt (S-6, Design §8.7): the next line is a
    /// resume/steer/stop/finish answer.
    CompletionGate,
    /// An inline delete-confirm for the memory inspector (FR-6, §4.9): the next
    /// line confirms (`y`) or cancels. Delete is destructive, so it never
    /// happens on a single command — parity with the rich overlay's y/N step.
    MemoryDelete {
        scope: MemoryScope,
        name: String,
    },
}

/// Run the line-mode frontend: render events to stdout, forward stdin lines to
/// the engine as commands. Returns when either channel closes.
///
/// While a permission prompt is open, the next input line is its answer; a
/// `/cancel` line cancels the current turn; anything else is a user message.
pub async fn run(
    ports: FrontendPorts,
    sessions_dir: PathBuf,
    config_template: String,
    reasoning_view: ReasoningView,
) -> io::Result<()> {
    // `.agents/` is the parent of the sessions dir; `/config` and `/prompt`
    // resolve their targets under it (C-5).
    let agents_dir = sessions_dir
        .parent()
        .map_or(sessions_dir.clone(), Path::to_path_buf);
    let mut renderer = LineRenderer::new(reasoning_view);
    let mut stdout = io::stdout();
    let mut events_rx = ports.events_rx;
    // Held in an Option so stdin EOF can drop it, signaling the engine to
    // finish; we keep draining events until it closes its side.
    let mut commands_tx = Some(ports.commands_tx);

    let (lines_tx, mut lines_rx) = mpsc::channel::<String>(16);
    tokio::spawn(async move {
        let mut reader = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if lines_tx.send(line).await.is_err() {
                break;
            }
        }
    });

    let mut pending: Option<Pending> = None;
    // Track the current tier so `/mode` can cycle it (the engine gates the auto
    // tiers on confinement and echoes a ModeChanged / Notice back).
    let mut mode = Mode::default();
    // Standing inspector state (FR-6/FR-7): the skill catalog (from
    // `SkillsAvailable`, so `/skills` prints without a round-trip) and the last
    // memory listing (so `/memory <name>` and `/memory delete <name>` resolve a
    // name to its scope). The engine owns the store; the frontend never reads it.
    let mut skills: Vec<SkillMeta> = Vec::new();
    let mut mem_user: Vec<EntrySummary> = Vec::new();
    let mut mem_project: Vec<EntrySummary> = Vec::new();
    let mut stdin_open = true;
    loop {
        tokio::select! {
            event = events_rx.recv() => match event {
                Some(event) => {
                    renderer.render(&event, &mut stdout)?;
                    stdout.flush()?;
                    if let UiEvent::ModeChanged { mode: changed } = &event {
                        mode = *changed;
                    }
                    match event {
                        UiEvent::PermissionRequest { id, .. } => {
                            pending = Some(Pending::Permission(id));
                        }
                        UiEvent::AskUserRequest { id, options, .. } => {
                            pending = Some(Pending::Ask { id, options });
                        }
                        UiEvent::LoopHalted { .. } => {
                            pending = Some(Pending::Loop);
                        }
                        UiEvent::CompletionGateHalted { .. } => {
                            pending = Some(Pending::CompletionGate);
                        }
                        // Keep the standing inspector state current. The catalog
                        // is rendered on demand by `/skills` (not printed here);
                        // the memory list was already printed by `render` above,
                        // and is retained so name-based commands can resolve it.
                        UiEvent::SkillsAvailable { skills: s } => {
                            skills = s;
                        }
                        UiEvent::MemoryEntries { user, project } => {
                            mem_user = user;
                            mem_project = project;
                        }
                        _ => {}
                    }
                }
                None => break, // engine finished and closed its events
            },
            line = lines_rx.recv(), if stdin_open => match (line, commands_tx.as_ref()) {
                (Some(line), Some(tx)) => {
                    if let Some(p) = pending.take() {
                        match p {
                            Pending::Permission(id) => {
                                let _ = tx.send(Command::PermissionAnswer { id, decision: parse_permission_answer(&line) }).await;
                            }
                            Pending::Ask { id, options } => {
                                let _ = tx.send(Command::AskUserAnswer { id, answer: parse_ask_answer(&line, &options) }).await;
                            }
                            Pending::Loop => {
                                let _ = tx.send(Command::ResolveLoop { resolution: parse_loop_resolution(&line) }).await;
                            }
                            Pending::CompletionGate => {
                                let _ = tx.send(Command::ResolveCompletionGate { resolution: parse_gate_resolution(&line) }).await;
                            }
                            Pending::MemoryDelete { scope, name } => {
                                // Destructive: only an explicit `y` deletes; the
                                // harness performs the write (FR-6), then we
                                // re-list so the updated set prints.
                                if matches!(line.trim().to_lowercase().as_str(), "y" | "yes") {
                                    let _ = tx.send(Command::MemoryMutate {
                                        op: MemoryOp::Remove, scope, name: name.clone(),
                                        description: None, type_: None, body: None,
                                    }).await;
                                    let _ = tx.send(Command::MemoryList).await;
                                    println!("deleted {name}");
                                } else {
                                    println!("cancelled");
                                }
                            }
                        }
                    } else if line.trim() == "/cancel" {
                        let _ = tx.send(Command::Cancel).await;
                    } else if line.trim() == "/mode" {
                        let _ = tx.send(Command::SetMode { mode: next_mode(mode) }).await;
                    } else if let Some(args) = line
                        .trim()
                        .strip_prefix("/model")
                        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
                    {
                        // Degraded mode has no picker, so `/model` needs a
                        // profile argument (C-6); the engine validates it.
                        let mut parts = args.split_whitespace();
                        if let Some(profile) = parts.next() {
                            let model = parts.next().map(str::to_string);
                            let _ = tx.send(Command::SwitchModel { profile: profile.to_string(), model }).await;
                        } else {
                            println!("usage: /model <profile> [model]");
                        }
                    } else if line.trim() == "/reload" {
                        let _ = tx.send(Command::ReloadConfig).await;
                    } else if line.trim() == "/config" {
                        // Line mode does not launch $EDITOR (stdin is the line
                        // reader / often a pipe): seed + point at the file, then
                        // the user edits it and runs /reload (C-5, degraded §7).
                        match crate::edit::config_target(&agents_dir, &config_template) {
                            Ok((path, existed)) => println!(
                                "{} {} — edit it, then /reload to apply",
                                if existed { "editing" } else { "created" },
                                path.display()
                            ),
                            Err(e) => println!("could not prepare config: {e}"),
                        }
                    } else if let Some(rest) = line
                        .trim()
                        .strip_prefix("/prompt")
                        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
                    {
                        match crate::edit::prompt_target(&agents_dir, rest.trim()) {
                            Ok((path, existed)) => println!(
                                "{} {} — edit it, then /reload to apply",
                                if existed { "editing" } else { "created" },
                                path.display()
                            ),
                            Err(msg) => println!("{msg}"),
                        }
                    } else if let Some(rest) = line
                        .trim()
                        .strip_prefix("/effort")
                        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
                    {
                        // Degraded mode has no picker, so `/effort` needs a level
                        // argument (P-9); the engine drops it if the model has no
                        // control.
                        match emberly_core::Effort::parse(rest.trim()) {
                            Some(effort) => {
                                let _ = tx.send(Command::SetEffort { effort }).await;
                            }
                            None => println!("usage: /effort <low|medium|high|max>"),
                        }
                    } else if let Some(rest) = line
                        .trim()
                        .strip_prefix("/memory")
                        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
                    {
                        // Full inspection inline (FR-6, §4.9 — not rich-only):
                        // list, view a body, or a confirmed delete. Edit is a
                        // rich-TUI affordance (the plain frontend deliberately
                        // never launches $EDITOR — its stdin is the line reader,
                        // like /config/prompt), so it is not offered here.
                        let rest = rest.trim();
                        if rest.is_empty() {
                            let _ = tx.send(Command::MemoryList).await;
                        } else if let Some(name) = rest.strip_prefix("delete ").map(str::trim) {
                            match resolve_memory_scope(name, &mem_user, &mem_project) {
                                Some(scope) => {
                                    println!(
                                        "delete {name} ({})?  [y] confirm, anything else cancels",
                                        scope_label(scope)
                                    );
                                    pending = Some(Pending::MemoryDelete { scope, name: name.to_string() });
                                }
                                None => println!("no memory entry named '{name}' — run /memory to list"),
                            }
                        } else if rest == "delete" {
                            println!("usage: /memory delete <name>");
                        } else {
                            match resolve_memory_scope(rest, &mem_user, &mem_project) {
                                Some(scope) => {
                                    let _ = tx.send(Command::MemoryView { scope, name: rest.to_string() }).await;
                                }
                                None => println!("no memory entry named '{rest}' — run /memory to list"),
                            }
                        }
                    } else if let Some(rest) = line
                        .trim()
                        .strip_prefix("/skills")
                        .filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))
                    {
                        // The catalog is standing state, so listing needs no
                        // round-trip; a named skill's body is fetched read-only
                        // (FR-7 — inspectable before it ever runs).
                        let rest = rest.trim();
                        if rest.is_empty() {
                            let _ = render_skill_list(&skills, &mut stdout);
                            let _ = stdout.flush();
                        } else {
                            let _ = tx.send(Command::InspectSkill { name: rest.to_string() }).await;
                        }
                    } else if !line.trim().is_empty() {
                        let _ = tx.send(Command::UserInput { text: line }).await;
                    }
                }
                _ => {
                    // stdin closed: drop the command sender so the engine
                    // finishes the current turn and closes its events. We keep
                    // draining events until it does (no forced cancel — an
                    // in-flight reply should still be shown).
                    commands_tx = None;
                    stdin_open = false;
                }
            },
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use emberly_core::{PermissionRendering, ToolCallId};

    fn render_to_string(event: &UiEvent) -> String {
        render_with(ReasoningView::Collapsed, &[event])
    }

    /// Render a sequence of events through one renderer (state carries across
    /// events, e.g. the reasoning→answer transition).
    fn render_with(view: ReasoningView, events: &[&UiEvent]) -> String {
        let mut renderer = LineRenderer::new(view);
        let mut buf: Vec<u8> = Vec::new();
        for event in events {
            let ok = renderer.render(event, &mut buf).is_ok();
            assert!(ok, "render should not fail writing to a Vec");
        }
        String::from_utf8(buf).unwrap_or_default()
    }

    #[test]
    fn assistant_delta_streams_without_newline() {
        assert_eq!(
            render_to_string(&UiEvent::AssistantDelta { text: "hi".into() }),
            "hi"
        );
        assert_eq!(render_to_string(&UiEvent::AssistantDone), "\n");
    }

    #[test]
    fn reasoning_renders_a_labeled_block_then_the_answer() {
        let out = render_with(
            ReasoningView::Collapsed,
            &[
                &UiEvent::ReasoningDelta {
                    text: "let me think".into(),
                },
                &UiEvent::AssistantDelta {
                    text: "answer".into(),
                },
                &UiEvent::AssistantDone,
            ],
        );
        assert!(out.contains("--- reasoning ---"), "labeled block: {out:?}");
        assert!(out.contains("let me think"));
        assert!(out.contains("--- answer ---"), "answer separated: {out:?}");
        assert!(out.contains("answer"));
    }

    #[test]
    fn hidden_view_suppresses_reasoning_in_plain_mode() {
        let out = render_with(
            ReasoningView::Hidden,
            &[
                &UiEvent::ReasoningDelta {
                    text: "secret".into(),
                },
                &UiEvent::AssistantDelta {
                    text: "answer".into(),
                },
            ],
        );
        assert!(!out.contains("reasoning"), "no trail when hidden: {out:?}");
        assert!(!out.contains("secret"));
        assert!(out.contains("answer"));
    }

    #[test]
    fn file_diff_shows_plus_minus_prefixes() {
        let out = render_to_string(&UiEvent::FileDiff {
            path: "a.rs".into(),
            unified: "--- a/a.rs\n+++ b/a.rs\n-old\n+new".into(),
        });
        assert!(out.contains("-old"), "deletions carry a - prefix");
        assert!(out.contains("+new"), "additions carry a + prefix");
    }

    #[test]
    fn tool_call_explanation_renders_indented_under_the_call() {
        let out = render_to_string(&UiEvent::ToolStarted {
            call_id: ToolCallId::new("c"),
            tool: "bash".into(),
            summary: "run: sed -i s/debug/info/ log.conf".into(),
            explanation: Some("raise the log level to info".into()),
        });
        assert!(
            out.contains("run: sed"),
            "the call stays the headline: {out:?}"
        );
        assert!(
            out.contains("- raise the log level to info"),
            "caption indented under the call, ASCII lead (Design §4.5/§7): {out:?}"
        );
    }

    #[test]
    fn no_explanation_renders_no_caption_line() {
        let out = render_to_string(&UiEvent::ToolStarted {
            call_id: ToolCallId::new("c"),
            tool: "read_file".into(),
            summary: "read src/main.rs".into(),
            explanation: None,
        });
        // Only the call line — no dangling indented caption (no placeholder).
        assert_eq!(out.trim(), "> read_file: read src/main.rs");
    }

    #[test]
    fn ask_user_prompt_is_calm_and_numbers_options() {
        let out = render_to_string(&UiEvent::AskUserRequest {
            id: AskId(1),
            question: "which environment?".into(),
            options: vec!["dev".into(), "prod".into()],
        });
        assert!(out.contains("QUESTION"), "calm heading: {out:?}");
        assert!(out.contains("which environment?"));
        assert!(
            out.contains("1. dev") && out.contains("2. prod"),
            "numbered options"
        );
        // Not a safety prompt — no capitals banner (Design §5.1).
        assert!(!out.contains("!!"));
        assert!(!out.contains("OUTSIDE"));
    }

    #[test]
    fn loop_halt_renders_harness_voice_no_alarm() {
        let out = render_to_string(&UiEvent::LoopHalted {
            reason: "read nope.txt repeatedly".into(),
        });
        assert!(out.contains("Stopped"), "harness heading: {out:?}");
        assert!(out.contains("read nope.txt repeatedly"));
        assert!(
            out.contains("keep going") && out.contains("stop"),
            "choices offered"
        );
        // Not a safety prompt.
        assert!(!out.contains("!!"));
    }

    #[test]
    fn parse_loop_resolution_maps_keep_stop_empty_and_steer() {
        assert_eq!(parse_loop_resolution("keep"), LoopResolution::Resume);
        assert_eq!(parse_loop_resolution("1"), LoopResolution::Resume);
        assert_eq!(parse_loop_resolution("stop"), LoopResolution::Stop);
        // Empty stops — never keep spending unattended.
        assert_eq!(parse_loop_resolution("   "), LoopResolution::Stop);
        // Anything else is a steer message handed back to the model.
        assert_eq!(
            parse_loop_resolution("focus on the parser"),
            LoopResolution::Steer("focus on the parser".into())
        );
    }

    #[test]
    fn completion_gate_renders_harness_voice_no_alarm() {
        let out = render_to_string(&UiEvent::CompletionGateHalted {
            failing: vec![CheckResult {
                name: "tests".into(),
                passed: false,
                reason: "2 failed".into(),
            }],
            attempts: 3,
        });
        assert!(out.contains("Stopped"), "harness heading: {out:?}");
        assert!(out.contains("3 attempts"));
        assert!(out.contains("tests"));
        assert!(out.contains("2 failed"));
        assert!(
            out.contains("keep going") && out.contains("stop") && out.contains("finish anyway"),
            "all four choices offered (Design §8.7): {out:?}"
        );
        // Not a safety prompt, and never presented as though the checks passed.
        assert!(!out.contains("!!"));
        assert!(!out.contains("checks passed"));
    }

    #[test]
    fn parse_gate_resolution_maps_keep_stop_finish_empty_and_steer() {
        assert_eq!(parse_gate_resolution("keep"), GateResolution::Resume);
        assert_eq!(parse_gate_resolution("1"), GateResolution::Resume);
        assert_eq!(parse_gate_resolution("stop"), GateResolution::Stop);
        // Empty stops — never keep spending unattended.
        assert_eq!(parse_gate_resolution("   "), GateResolution::Stop);
        assert_eq!(parse_gate_resolution("finish"), GateResolution::Finish);
        assert_eq!(parse_gate_resolution("f"), GateResolution::Finish);
        // Anything else is a steer message handed back to the model.
        assert_eq!(
            parse_gate_resolution("fix the failing test"),
            GateResolution::Steer("fix the failing test".into())
        );
    }

    #[test]
    fn parse_ask_answer_handles_number_text_and_empty() {
        let options = vec!["dev".to_string(), "prod".to_string()];
        assert_eq!(
            parse_ask_answer("2", &options),
            AskAnswer::Answered("prod".into()),
            "a bare number picks that option"
        );
        assert_eq!(
            parse_ask_answer("staging", &options),
            AskAnswer::Answered("staging".into()),
            "free text passes through"
        );
        assert_eq!(
            parse_ask_answer("   ", &options),
            AskAnswer::Declined,
            "an empty line declines — no unsafe default"
        );
        // An out-of-range number is treated as free text, not a panic.
        assert_eq!(
            parse_ask_answer("9", &options),
            AskAnswer::Answered("9".into())
        );
    }

    #[test]
    fn degraded_output_has_no_ansi_escapes() {
        // Degraded mode is colourless and append-only: no ANSI/cursor control
        // ever reaches the stream (Design §7).
        let events = [
            UiEvent::AssistantDelta {
                text: "สวัสดี".into(),
            },
            UiEvent::AskUserRequest {
                id: AskId(1),
                question: "ตกลงไหม?".into(),
                options: vec!["ใช่".into(), "ไม่".into()],
            },
            UiEvent::LoopHalted {
                reason: "no progress".into(),
            },
            UiEvent::ToolStarted {
                call_id: ToolCallId::new("c"),
                tool: "bash".into(),
                summary: "run: ls".into(),
                // Exercise the caption path in the no-ANSI sweep too.
                explanation: Some("list the working tree".into()),
            },
            UiEvent::ToolFinished {
                call_id: ToolCallId::new("c"),
                ok: true,
                summary: "exit 0".into(),
                preview: "total 0".into(),
                untrusted: false,
            },
            UiEvent::FileDiff {
                path: "a".into(),
                unified: "-x\n+y".into(),
            },
            UiEvent::Retrying {
                attempt: 1,
                max_attempts: 3,
                delay_ms: 500,
                reason: "429".into(),
            },
            UiEvent::TaskListUpdated {
                items: vec![emberly_core::TaskItem {
                    text: "task".into(),
                    status: emberly_core::TaskStatus::InProgress,
                }],
            },
            // Inspector events must be ASCII/no-ANSI in degraded mode too
            // (FR-6/FR-7, Design §7) — inspection is not rich-only.
            UiEvent::MemoryEntries {
                user: vec![EntrySummary {
                    name: "หน่วยความจำ".into(),
                    description: "ทดสอบ".into(),
                    type_: None,
                    scope: MemoryScope::User,
                }],
                project: vec![],
            },
            UiEvent::MemoryBody {
                scope: MemoryScope::User,
                name: "note".into(),
                body: "จดจำสิ่งนี้".into(),
            },
            UiEvent::SkillBody {
                name: "pdf-fill".into(),
                origin: SkillOrigin::User,
                body: "ขั้นตอนที่ 1".into(),
                resources: vec!["/abs/template.txt".into()],
            },
            // read_image reference line (P-11, Design §4.8): ASCII-only in
            // degraded mode, no ANSI. No-vision failure also ASCII-only.
            UiEvent::ToolFinished {
                call_id: ToolCallId::new("img"),
                ok: true,
                summary: "read image mockup.png \u{00b7} 1200\u{00d7}800 \u{00b7} PNG".into(),
                preview: String::new(),
                untrusted: false,
            },
            UiEvent::ToolFinished {
                call_id: ToolCallId::new("img2"),
                ok: false,
                summary: "no vision".into(),
                preview: String::new(),
                untrusted: false,
            },
        ];
        for e in events {
            let s = render_to_string(&e);
            assert!(!s.contains('\u{1b}'), "no ANSI escape in {s:?}");
        }
    }

    #[test]
    fn tool_finished_shows_status() {
        let ok = render_to_string(&UiEvent::ToolFinished {
            call_id: ToolCallId::new("c1"),
            ok: true,
            summary: "wrote a.txt".into(),
            preview: "done".into(),
            untrusted: false,
        });
        assert!(ok.contains("[ok]"));
        assert!(ok.contains("done"), "preview shown in degraded mode");
        let failed = render_to_string(&UiEvent::ToolFinished {
            call_id: ToolCallId::new("c1"),
            ok: false,
            summary: "denied".into(),
            preview: String::new(),
            untrusted: false,
        });
        assert!(failed.contains("[FAILED]"));
    }

    #[test]
    fn permission_prompt_is_full_and_deny_default() {
        let rendering = PermissionRendering {
            tool: "bash".into(),
            summary: "run: rm -rf build".into(),
            detail: "rm -rf build".into(),
            affected_paths: vec![],
            outside_root: false,
            reason: "bash requires your approval".into(),
        };
        let out = render_to_string(&UiEvent::PermissionRequest {
            id: PermissionId(1),
            rendering,
        });
        assert!(out.contains("PERMISSION REQUIRED"));
        assert!(out.contains("rm -rf build"), "full command must be shown");
        assert!(out.contains("[Enter] DENY"), "deny is the default");
    }

    #[test]
    fn outside_root_prompt_has_loud_banner() {
        let rendering = PermissionRendering {
            tool: "write_file".into(),
            summary: "write /etc/x".into(),
            detail: "…".into(),
            affected_paths: vec!["/etc/x".into()],
            outside_root: true,
            reason: "this action affects files OUTSIDE the project root".into(),
        };
        let out = render_to_string(&UiEvent::PermissionRequest {
            id: PermissionId(2),
            rendering,
        });
        assert!(out.contains("OUTSIDE YOUR PROJECT"));
    }

    #[test]
    fn answer_parsing_defaults_to_deny() {
        assert_eq!(parse_permission_answer("y"), PermissionDecision::AllowOnce);
        assert_eq!(
            parse_permission_answer("YES"),
            PermissionDecision::AllowOnce
        );
        assert_eq!(
            parse_permission_answer("s"),
            PermissionDecision::AllowForSession
        );
        assert_eq!(parse_permission_answer(""), PermissionDecision::Deny);
        assert_eq!(parse_permission_answer("no"), PermissionDecision::Deny);
        assert_eq!(
            parse_permission_answer("anything"),
            PermissionDecision::Deny
        );
    }

    #[test]
    fn task_list_renders_ascii_checklist_in_plain_mode() {
        // The plain frontend has no sidebar, so TaskListUpdated renders as an
        // inline ASCII checklist (Design §7). No ANSI, markers carry status.
        let out = render_to_string(&UiEvent::TaskListUpdated {
            items: vec![
                emberly_core::TaskItem {
                    text: "first".into(),
                    status: emberly_core::TaskStatus::Done,
                },
                emberly_core::TaskItem {
                    text: "second".into(),
                    status: emberly_core::TaskStatus::InProgress,
                },
                emberly_core::TaskItem {
                    text: "third".into(),
                    status: emberly_core::TaskStatus::Pending,
                },
            ],
        });
        assert!(out.contains("[x] first"), "done item: {out:?}");
        assert!(out.contains("[~] second"), "in-progress item: {out:?}");
        assert!(out.contains("[ ] third"), "pending item: {out:?}");
        assert!(!out.contains('\u{1b}'), "no ANSI in degraded mode");
    }

    #[test]
    fn untrusted_web_results_labeled_in_degraded_mode() {
        // Design §4.10: untrusted web content must be visibly labeled as fetched
        // web data in degraded mode — ASCII-only, no ANSI, visible source URLs
        // so the "untrusted web content" meaning survives without colour.
        let out = render_to_string(&UiEvent::ToolFinished {
            call_id: ToolCallId::new("ws"),
            ok: true,
            summary: "searched: \"rust async\" (2 results)".into(),
            preview:
                "Search results for: rust async\n1. Tokio\n   https://tokio.rs\n   Async runtime"
                    .into(),
            untrusted: true,
        });
        assert!(out.contains("[ok]"), "status shown");
        assert!(
            out.contains("[web]"),
            "untrusted label visible in degraded mode"
        );
        assert!(out.contains("https://tokio.rs"), "source URL visible");
        assert!(!out.contains('\u{1b}'), "no ANSI in degraded mode");
    }

    #[test]
    fn trusted_results_have_no_web_label() {
        let out = render_to_string(&UiEvent::ToolFinished {
            call_id: ToolCallId::new("bash"),
            ok: true,
            summary: "exit 0".into(),
            preview: "hello world".into(),
            untrusted: false,
        });
        assert!(
            !out.contains("[web]"),
            "ordinary results have no [web] label"
        );
    }

    // ---- inspectors, degraded parity (FR-6/FR-7, Design §4.9, §7) ---------

    fn mem_sum(name: &str, desc: &str, scope: emberly_core::MemoryScope) -> EntrySummary {
        EntrySummary {
            name: name.into(),
            description: desc.into(),
            type_: None,
            scope,
        }
    }

    #[test]
    fn memory_list_renders_grouped_ascii_inline() {
        let out = render_to_string(&UiEvent::MemoryEntries {
            user: vec![mem_sum("Build", "how to build", MemoryScope::User)],
            project: vec![mem_sum("Conventions", "team style", MemoryScope::Project)],
        });
        assert!(out.contains("user memory:"), "user group header: {out:?}");
        assert!(out.contains("Build - how to build"));
        assert!(out.contains("project memory:"), "project group header");
        assert!(out.contains("Conventions - team style"));
        assert!(!out.contains('\u{1b}'), "no ANSI in degraded mode");
    }

    #[test]
    fn memory_list_empty_renders_calm_message() {
        let out = render_to_string(&UiEvent::MemoryEntries {
            user: vec![],
            project: vec![],
        });
        assert!(out.contains("no stored memory"), "empty message: {out:?}");
    }

    #[test]
    fn memory_list_untrusted_root_shows_only_user_group() {
        // The project group is simply absent when it is empty (FR-1) — the
        // untrusted-root shape, driven by the engine returning no project rows.
        let out = render_to_string(&UiEvent::MemoryEntries {
            user: vec![mem_sum("Build", "b", MemoryScope::User)],
            project: vec![],
        });
        assert!(out.contains("user memory:"));
        assert!(
            !out.contains("project memory"),
            "no project group when empty: {out:?}"
        );
    }

    #[test]
    fn memory_body_renders_inline_read_only() {
        let out = render_to_string(&UiEvent::MemoryBody {
            scope: MemoryScope::User,
            name: "Build".into(),
            body: "cargo build".into(),
        });
        assert!(
            out.contains("memory: Build (user)"),
            "header with origin: {out:?}"
        );
        assert!(out.contains("cargo build"));
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn skill_body_renders_inline_with_resources() {
        let out = render_to_string(&UiEvent::SkillBody {
            name: "pdf-fill".into(),
            origin: SkillOrigin::User,
            body: "Step 1: open the template.".into(),
            resources: vec!["/abs/template.txt".into()],
        });
        assert!(
            out.contains("skill: pdf-fill (user)"),
            "header with origin: {out:?}"
        );
        assert!(out.contains("Step 1: open the template."));
        assert!(out.contains("bundled files:"));
        assert!(out.contains("template.txt"), "resource path listed");
        assert!(!out.contains('\u{1b}'));
    }

    #[test]
    fn skill_list_renders_names_origin_and_empty() {
        let skills = vec![
            SkillMeta {
                name: "pdf-fill".into(),
                description: "Fill PDF forms".into(),
                origin: SkillOrigin::User,
            },
            SkillMeta {
                name: "linter".into(),
                description: String::new(),
                origin: SkillOrigin::Project,
            },
        ];
        let mut buf: Vec<u8> = Vec::new();
        assert!(render_skill_list(&skills, &mut buf).is_ok());
        let out = String::from_utf8(buf).unwrap_or_default();
        assert!(out.contains("pdf-fill - Fill PDF forms (user)"), "{out:?}");
        assert!(out.contains("linter (project)"), "no-desc skill + origin");
        assert!(!out.contains('\u{1b}'));

        let mut empty: Vec<u8> = Vec::new();
        assert!(render_skill_list(&[], &mut empty).is_ok());
        assert!(String::from_utf8(empty)
            .unwrap_or_default()
            .contains("no skills available"));
    }

    #[test]
    fn resolve_memory_scope_prefers_user_then_project() {
        let user = vec![mem_sum("Shared", "", MemoryScope::User)];
        let project = vec![
            mem_sum("Shared", "", MemoryScope::Project),
            mem_sum("ProjOnly", "", MemoryScope::Project),
        ];
        assert_eq!(
            resolve_memory_scope("Shared", &user, &project),
            Some(MemoryScope::User)
        );
        assert_eq!(
            resolve_memory_scope("ProjOnly", &user, &project),
            Some(MemoryScope::Project)
        );
        assert_eq!(resolve_memory_scope("Nope", &user, &project), None);
    }
}
