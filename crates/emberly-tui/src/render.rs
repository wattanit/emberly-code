//! Frame rendering (Design §3, Tech Spec §9). Pure over `&App` and a ratatui
//! `Frame` — no engine access, no mutation — so the layout is driven entirely
//! by the view-model.
//!
//! Layout: a conversation pane (with the input box beneath it) on the left, a
//! collapsible right sidebar, and a one-line status bar across the bottom.
//! Below [`COLLAPSE_BELOW`] columns the sidebar hides and its critical figures
//! (context %, mode) live only on the status bar (Design §3.2) — everything the
//! sidebar shows is also reachable by command, so nothing is sidebar-exclusive.

use emberly_core::{Mode, SandboxStatus};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, ConvItem};
use crate::text;
use crate::theme::Theme;
use crate::{strings, strings::markers};

/// Sidebar width in columns when shown.
const SIDEBAR_WIDTH: u16 = 32;
/// Below this total width the sidebar auto-collapses (Design §3.2).
const COLLAPSE_BELOW: u16 = 100;
/// Input box grows with content up to this many text rows, then scrolls.
const MAX_INPUT_ROWS: usize = 6;
/// The prompt gutter (`"› "`) reserved on every input row.
const GUTTER: u16 = 2;

/// Draw one full frame.
pub fn frame(f: &mut Frame, app: &App) {
    let area = f.area();
    let sidebar_shown = app.sidebar_visible && area.width >= COLLAPSE_BELOW;

    // Bottom status bar spans the full width; the body sits above it.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    let (body, status) = (outer[0], outer[1]);

    let (main, sidebar) = if sidebar_shown {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(SIDEBAR_WIDTH)])
            .split(body);
        (cols[0], Some(cols[1]))
    } else {
        (body, None)
    };

    // Within the main column: conversation over the input box.
    let input_rows = app.editor.line_count().clamp(1, MAX_INPUT_ROWS);
    let input_height = u16::try_from(input_rows).unwrap_or(1).saturating_add(2);
    let main_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(input_height)])
        .split(main);

    render_conversation(f, app, main_rows[0]);
    render_input(f, app, main_rows[1]);
    if let Some(area) = sidebar {
        render_sidebar(f, app, area);
    }
    render_status(f, app, status, sidebar_shown);
}

// ---- conversation --------------------------------------------------------

fn render_conversation(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;

    if app.pending_permission.is_some() {
        render_permission(f, app, area);
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.chrome())
        .title(Span::styled(
            format!(" {} ", strings::brand::NAME),
            theme.accent(),
        ));
    let inner = block.inner(area);
    let width = usize::from(inner.width);
    let height = usize::from(inner.height);

    // Pre-wrap the whole conversation to the pane width (Thai-safe, via the
    // text engine) so scroll math counts real display rows.
    let lines = conversation_lines(app, width);
    let total = lines.len();
    let max_scroll = total.saturating_sub(height);
    let scroll = app.scroll.min(max_scroll);
    let end = total - scroll;
    let start = end.saturating_sub(height);
    let visible: Vec<Line> = lines[start..end].to_vec();

    // A dim hint when scrolled up, so it is obvious there is newer content.
    let block = if scroll > 0 {
        block.title(Line::from(Span::styled(" ↓ more ", theme.chrome())).right_aligned())
    } else {
        block
    };
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(visible), inner);
}

/// Flatten the conversation into styled, pre-wrapped display rows.
fn conversation_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let mut out: Vec<Line<'static>> = Vec::new();
    let w = width.max(1);

    for item in &app.conversation {
        match item {
            ConvItem::User(text) => {
                push_wrapped(
                    &mut out,
                    text,
                    w.saturating_sub(2),
                    Span::styled(format!("{} ", markers::USER_PROMPT), theme.accent()),
                    "  ",
                    theme.primary(),
                );
            }
            ConvItem::Assistant(text) => {
                for logical in text.split('\n') {
                    for row in text::wrap(logical, w) {
                        out.push(Line::from(Span::styled(row, theme.primary())));
                    }
                }
            }
            ConvItem::Tool { summary, done, .. } => {
                // No "tool:" prefix — the summary is already a verb phrase
                // ("run: cargo test", "read src/main.rs"), so a "bash: bash"
                // style redundancy can't occur.
                let (mark, style) = match done {
                    None => (markers::RUNNING, theme.chrome()),
                    Some(true) => (markers::OK, theme.success()),
                    Some(false) => (markers::FAILED, theme.error()),
                };
                push_wrapped(
                    &mut out,
                    summary,
                    w.saturating_sub(mark.len() + 3),
                    Span::styled(format!("[{mark}] "), style),
                    "    ",
                    theme.chrome(),
                );
            }
            ConvItem::Notice(text) => {
                push_wrapped(
                    &mut out,
                    text,
                    w.saturating_sub(2),
                    Span::styled(format!("{} ", markers::NOTICE), theme.chrome()),
                    "  ",
                    theme.chrome(),
                );
            }
        }
    }
    out
}

/// Wrap `text` to `width`, styling the first row with `head` and continuation
/// rows with a plain `indent`, all body spans using `body_style`.
fn push_wrapped(
    out: &mut Vec<Line<'static>>,
    text: &str,
    width: usize,
    head: Span<'static>,
    indent: &'static str,
    body_style: ratatui::style::Style,
) {
    let rows = text::wrap(text, width.max(1));
    for (i, row) in rows.into_iter().enumerate() {
        let lead = if i == 0 {
            head.clone()
        } else {
            Span::raw(indent)
        };
        out.push(Line::from(vec![lead, Span::styled(row, body_style)]));
    }
}

// ---- sidebar -------------------------------------------------------------

fn render_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(theme.chrome());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let w = usize::from(inner.width).saturating_sub(1);
    let mut lines: Vec<Line> = Vec::new();

    // Wordmark + version (Design §1.1): ember `emberly`, dimmed `code` + version.
    lines.push(Line::from(vec![
        Span::styled(strings::brand::NAME, theme.accent()),
        Span::styled(
            format!(" {} v{}", strings::brand::SUFFIX, env!("CARGO_PKG_VERSION")),
            theme.chrome(),
        ),
    ]));
    lines.push(Line::from(""));

    // Session title (auto-generated/renamable later) + root.
    let title = if app.session.title.is_empty() {
        Span::styled("untitled session", theme.chrome())
    } else {
        Span::styled(fit(&app.session.title, w), theme.primary())
    };
    lines.push(Line::from(title));
    lines.push(labeled(
        theme,
        strings::status::ROOT_LABEL,
        &abbrev_home(&app.session.project_root),
        w,
    ));
    lines.push(Line::from(""));

    // Model block: model, context %, cost estimate, sandbox status.
    let model = if app.session.provider.is_empty() {
        app.session.model.clone()
    } else {
        format!("{}/{}", app.session.provider, app.session.model)
    };
    lines.push(labeled(theme, strings::status::MODEL_LABEL, &model, w));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} ", strings::status::CONTEXT_ABBR),
            theme.chrome(),
        ),
        Span::styled(
            format!("{}%", app.context_pct),
            context_style(theme, app.context_pct),
        ),
    ]));
    if app.cost_known {
        lines.push(Line::from(vec![
            Span::styled("cost ", theme.chrome()),
            Span::styled(format!("${:.4} ", app.cost_usd), theme.primary()),
            Span::styled(strings::status::COST_ESTIMATE_SUFFIX, theme.chrome()),
        ]));
    }
    lines.push(sandbox_line(theme, app.sandbox.as_ref(), w));
    lines.push(Line::from(""));

    // Modified files (Design §3.1): path + add/remove counts.
    lines.push(Line::from(Span::styled(
        strings::status::MODIFIED_FILES_TITLE,
        theme.chrome(),
    )));
    if app.modified_files.is_empty() {
        lines.push(Line::from(Span::styled("  —", theme.chrome())));
    } else {
        for file in &app.modified_files {
            let counts = format!(" +{} -{}", file.adds, file.dels);
            let path_w = w.saturating_sub(text::width(&counts));
            lines.push(Line::from(vec![
                Span::styled(fit(&file.path, path_w), theme.primary()),
                Span::styled(format!(" +{}", file.adds), theme.diff_add()),
                Span::styled(format!(" -{}", file.dels), theme.diff_del()),
            ]));
        }
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// A `label value` sidebar line, value truncated to fit.
fn labeled(theme: &Theme, label: &str, value: &str, width: usize) -> Line<'static> {
    let avail = width.saturating_sub(label.len() + 1);
    Line::from(vec![
        Span::styled(format!("{label} "), theme.chrome()),
        Span::styled(fit(value, avail), theme.primary()),
    ])
}

/// The sandbox status line: dimmed when confined, warning styling with a short
/// reason when degraded, `—` when not yet reported (Design §3.1, §8.2).
fn sandbox_line(theme: &Theme, status: Option<&SandboxStatus>, width: usize) -> Line<'static> {
    let label = Span::styled(
        format!("{} ", strings::status::SANDBOX_LABEL),
        theme.chrome(),
    );
    let value = match status {
        None => Span::styled(strings::status::SANDBOX_UNKNOWN, theme.chrome()),
        Some(SandboxStatus::Confined { backend }) => {
            Span::styled(fit(backend, width), theme.chrome())
        }
        Some(SandboxStatus::Partial { backend, missing }) => Span::styled(
            fit(&format!("{backend}: {missing}"), width),
            theme.warning(),
        ),
        Some(SandboxStatus::Unavailable { reason }) => {
            Span::styled(fit(reason, width), theme.warning())
        }
    };
    Line::from(vec![label, value])
}

fn context_style(theme: &Theme, pct: u8) -> ratatui::style::Style {
    if pct >= 90 {
        theme.error()
    } else if pct >= 75 {
        theme.warning()
    } else {
        theme.primary()
    }
}

// ---- input & status ------------------------------------------------------

fn render_input(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.chrome());
    let inner = block.inner(area);
    f.render_widget(block, area);

    if inner.width <= GUTTER || inner.height == 0 {
        return;
    }
    let text_width = usize::from(inner.width - GUTTER);
    let rows = usize::from(inner.height);

    let (cursor_row, cursor_col) = app.editor.cursor_row_col();
    let lines: Vec<&str> = app.editor.lines().collect();

    let top = cursor_row.saturating_sub(rows.saturating_sub(1));
    let h_scroll = cursor_col.saturating_sub(text_width.saturating_sub(1));

    for (screen_row, line_idx) in (top..top + rows).enumerate() {
        let Some(line) = lines.get(line_idx) else {
            break;
        };
        let start_col = if line_idx == cursor_row { h_scroll } else { 0 };
        let visible = text::slice_cols(line, start_col, text_width);
        let y = inner.y + u16::try_from(screen_row).unwrap_or(0);
        let gutter = if line_idx == 0 {
            Span::styled(format!("{} ", markers::USER_PROMPT), theme.accent())
        } else {
            Span::raw("  ")
        };
        let row_area = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                gutter,
                Span::styled(visible, theme.primary()),
            ])),
            row_area,
        );
    }

    if cursor_row >= top && cursor_row < top + rows {
        let screen_row = u16::try_from(cursor_row - top).unwrap_or(0);
        let col_in_view = cursor_col.saturating_sub(h_scroll);
        let x = inner.x + GUTTER + u16::try_from(col_in_view).unwrap_or(0);
        f.set_cursor_position((x.min(inner.x + inner.width - 1), inner.y + screen_row));
    }
}

fn render_status(f: &mut Frame, app: &App, area: Rect, sidebar_shown: bool) {
    let theme = &app.theme;
    let hints = if app.pending_permission.is_some() {
        strings::hints::PERMISSION
    } else {
        strings::hints::NORMAL
    };
    // Context % and mode always appear here; when the sidebar is collapsed this
    // is their only home (Design §3.2).
    let _ = sidebar_shown;
    let status = format!(
        " {mode}  {ctx} {pct}%  {hints}",
        mode = mode_name(app.mode),
        ctx = strings::status::CONTEXT_ABBR,
        pct = app.context_pct,
    );
    f.render_widget(Paragraph::new(status).style(theme.chrome()), area);
}

// ---- permission prompt (interim; full screen is group 7) -----------------

fn render_permission(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let Some((_, rendering)) = &app.pending_permission else {
        return;
    };
    let mut lines = Vec::new();
    if rendering.outside_root {
        lines.push(Line::from(Span::styled(
            strings::permission::OUTSIDE_ROOT_BANNER,
            theme.safety_band(),
        )));
    }
    lines.push(Line::from(vec![
        Span::styled(strings::permission::HEADING, theme.warning()),
        Span::styled(format!(": {}", rendering.summary), theme.primary()),
    ]));
    lines.push(Line::from(Span::styled(
        format!("{}: {}", strings::permission::WHY_LABEL, rendering.reason),
        theme.chrome(),
    )));
    if !rendering.affected_paths.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(
                "{}: {}",
                strings::permission::PATHS_LABEL,
                rendering.affected_paths.join(", ")
            ),
            theme.chrome(),
        )));
    }
    lines.push(Line::from(""));
    for line in rendering.detail.lines() {
        lines.push(Line::from(Span::styled(line.to_string(), theme.primary())));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!("[y] {}   ", strings::permission::ALLOW_ONCE),
            theme.success(),
        ),
        Span::styled(
            format!("[s] {}   ", strings::permission::ALLOW_SESSION),
            theme.success(),
        ),
        Span::styled(
            format!("[Enter] {}", strings::permission::DENY),
            theme.error(),
        ),
    ]));
    let prompt = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.dim_accent())
                .title(format!(" {} ", strings::permission::TITLE)),
        )
        .wrap(Wrap { trim: false });
    f.render_widget(prompt, area);
}

// ---- small helpers -------------------------------------------------------

/// Terse, lower-case mode name (Design §6.2).
fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => strings::mode::NORMAL,
        Mode::AutoAcceptEdits => strings::mode::AUTO_ACCEPT_EDITS,
        Mode::Auto => strings::mode::AUTO,
    }
}

/// Abbreviate a leading home directory with `~` (Design §3.1).
fn abbrev_home(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            if let Some(rest) = path.strip_prefix(&home) {
                return format!("~{rest}");
            }
        }
    }
    path.to_string()
}

/// Truncate `s` to at most `width` columns, appending `…` if it was cut.
/// Cluster-aware (never splits a grapheme).
fn fit(s: &str, width: usize) -> String {
    if text::width(s) <= width {
        return s.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let head = text::slice_cols(s, 0, width.saturating_sub(1));
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::SessionInfo;

    #[test]
    fn fit_truncates_with_ellipsis() {
        assert_eq!(fit("hello", 10), "hello");
        assert_eq!(fit("hello world", 5), "hell…");
    }

    #[test]
    fn fit_is_cluster_aware() {
        // "ที่กก" is 3 columns (a stacked cluster + two consonants); fitting to
        // 2 keeps the whole first cluster plus the ellipsis, never a split mark.
        assert_eq!(fit("ที่กก", 2), "ที่…");
    }

    #[test]
    fn abbrev_home_uses_tilde() {
        // Independent of the real HOME: only the prefix logic is exercised.
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() {
            let p = format!("{home}/proj");
            assert_eq!(abbrev_home(&p), "~/proj");
        }
    }

    #[test]
    fn conversation_wraps_and_counts_rows() {
        let mut app = App::new(SessionInfo::default());
        app.apply_event(emberly_core::UiEvent::AssistantDelta {
            text: "aaaa bbbb cccc".into(),
        });
        // Width 6 forces three rows.
        let lines = conversation_lines(&app, 6);
        assert_eq!(lines.len(), 3);
    }
}
