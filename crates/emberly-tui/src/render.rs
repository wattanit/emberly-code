//! Frame rendering (Design §3, Tech Spec §9). Pure over `&App` and a ratatui
//! `Frame` — no engine access, no mutation — so the layout is driven entirely
//! by the view-model.
//!
//! Layout: a conversation pane (with the input box beneath it) on the left, a
//! collapsible right sidebar, and a one-line status bar across the bottom.
//! Below [`COLLAPSE_BELOW`] columns the sidebar hides and its critical figures
//! (context %, mode) live only on the status bar (Design §3.2) — everything the
//! sidebar shows is also reachable by command, so nothing is sidebar-exclusive.

use emberly_core::{EntrySummary, Mode, SandboxStatus, SkillMeta, SkillOrigin, TaskStatus};
use ratatui::layout::{Constraint, Direction, Flex, Layout, Rect};
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::{App, ChoiceRow, ConvItem, Overlay, OverlayContent, SessionRow};
use crate::hit::{ClickTarget, HitMap};
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
/// Inline diffs show at most this many rows before pointing at the overlay.
const INLINE_DIFF_CAP: usize = 20;

/// Draw one full frame, populating `hit` with the click regions of the topmost
/// interactive layer (Design §3.4). `hit` is a fresh map each frame; the modal
/// renderers ([`render_overlay`], [`render_palette`]) clear it before pushing
/// their own regions, so it always reflects the layer that actually owns input
/// — a click can never fall through a modal to the pane behind it.
pub fn frame(f: &mut Frame, app: &App, hit: &mut HitMap) {
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

    // A pending permission prompt or question prompt takes over the whole main
    // area — no input box is shown, so nothing can be typed into a decision
    // (Design §5, §5.1). The permission prompt wins if somehow both are set.
    if app.pending_permission.is_some() {
        render_permission(f, app, main);
    } else if app.pending_ask.is_some() {
        render_ask(f, app, main);
    } else if app.pending_loop_halt.is_some() {
        render_loop_halt(f, app, main);
    } else {
        // Conversation over the input box.
        let input_rows = app.editor.line_count().clamp(1, MAX_INPUT_ROWS);
        let input_height = u16::try_from(input_rows).unwrap_or(1).saturating_add(2);
        let main_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(input_height)])
            .split(main);
        render_conversation(f, app, main_rows[0], hit);
        render_input(f, app, main_rows[1]);
    }
    if let Some(area) = sidebar {
        // The sidebar's click regions are live only on the base layer — not
        // while a permission/ask/loop prompt owns input (Design §3.4/§5). An
        // overlay or palette clears the map afterward, covering those.
        let base_active = app.pending_permission.is_none()
            && app.pending_ask.is_none()
            && app.pending_loop_halt.is_none();
        render_sidebar(f, app, area, hit, base_active);
    }
    render_status(f, app, status, sidebar_shown);

    // Overlays draw last, on top of everything (Design §4.2).
    if let Some(overlay) = app.active_overlay() {
        render_overlay(f, app, overlay, area, hit);
    }
    // The command palette sits above overlays when open (Design §3.3).
    if app.palette.is_some() {
        render_palette(f, app, area, hit);
    }
}

// ---- command palette -----------------------------------------------------

/// Draw the command palette: a query line over a filtered, selectable list.
fn render_palette(f: &mut Frame, app: &App, screen: Rect, hit: &mut HitMap) {
    let theme = &app.theme;
    // The palette is the topmost modal (Design §3.3): it owns the hit-map, so
    // clear any regions the layers behind it pushed — no click-through.
    hit.clear();
    let Some(palette) = &app.palette else {
        return;
    };
    let area = centered(screen, 60, 60);
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.dim_accent())
        .title(Span::styled(" commands ", theme.accent()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 2 || inner.width == 0 {
        return;
    }

    // Query line with the ember prompt marker.
    let query_area = Rect { height: 1, ..inner };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!("{} ", markers::USER_PROMPT), theme.accent()),
            Span::styled(palette.query.clone(), theme.primary()),
        ])),
        query_area,
    );

    let matched = crate::commands::matches(&palette.query);
    let list_h = usize::from(inner.height - 1);
    let selected = palette.selected.min(matched.len().saturating_sub(1));
    let top = selected.saturating_sub(list_h.saturating_sub(1));

    let mut lines: Vec<Line> = Vec::new();
    for (row, &ci) in matched.iter().enumerate().skip(top).take(list_h) {
        let spec = &crate::commands::COMMANDS[ci];
        let (marker, name_style) = if row == selected {
            ("› ", theme.accent())
        } else {
            ("  ", theme.primary())
        };
        let key = spec.key.map(|k| format!("  [{k}]")).unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(marker, theme.accent()),
            Span::styled(format!("/{:<9}", spec.name), name_style),
            Span::styled(spec.desc.to_string(), theme.chrome()),
            Span::styled(key, theme.chrome()),
        ]));
        // Clickable region for this row (Design §3.4): `row` is the index into
        // the filtered match list — exactly what `PaletteState::selected` holds
        // — so a click maps to "select this row + palette Enter".
        let y = inner.y + 1 + u16::try_from(row - top).unwrap_or(0);
        hit.push(
            Rect {
                x: inner.x,
                y,
                width: inner.width,
                height: 1,
            },
            ClickTarget::PaletteRow(row),
        );
    }
    if matched.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no matching command",
            theme.chrome(),
        )));
    }
    let list_area = Rect {
        y: inner.y + 1,
        height: inner.height - 1,
        ..inner
    };
    f.render_widget(Paragraph::new(lines), list_area);
}

// ---- overlay -------------------------------------------------------------

/// Which selectable list an overlay row belongs to — selects the `ClickTarget`
/// variant for a clicked overlay row (Design §3.4).
#[derive(Clone, Copy)]
enum RowKind {
    Choice,
    Session,
    Memory,
    Skill,
}

impl RowKind {
    fn target(self, row: usize) -> ClickTarget {
        match self {
            RowKind::Choice => ClickTarget::ChoiceRow(row),
            RowKind::Session => ClickTarget::SessionRow(row),
            RowKind::Memory => ClickTarget::MemoryRow(row),
            RowKind::Skill => ClickTarget::SkillRow(row),
        }
    }
}

/// Draw the active overlay as a centered, scrollable pane over the screen.
fn render_overlay(f: &mut Frame, app: &App, overlay: &Overlay, screen: Rect, hit: &mut HitMap) {
    let theme = &app.theme;
    // An overlay is modal over the conversation/sidebar (Design §4.2): it owns
    // the hit-map. Clear regions from the layers behind so a click cannot fall
    // through to them; interactive overlays push their own rows below. (If the
    // palette is also open it renders after this and clears again — palette on
    // top.)
    hit.clear();
    // Brief ease-in: the overlay expands from ~70% to its full 82% over a frame
    // or two (Design §6.4). Settled overlays render at full size.
    let p = app.overlay_ease_progress();
    let pct = 70 + (12.0 * p) as u16;
    let area = centered(screen, pct, pct);
    f.render_widget(Clear, area); // clear whatever is behind it

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.dim_accent())
        .title(Span::styled(format!(" {} ", overlay.title), theme.accent()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 2 || inner.width == 0 {
        return;
    }

    // One row is reserved at the bottom for the dismiss/scroll hint.
    let body_h = usize::from(inner.height - 1);
    let body_w = usize::from(inner.width);

    // The picker keeps its selection in view (forced_scroll); read-only panes
    // scroll freely, so their forced_scroll is None. `row_of_line`/`row_kind`
    // drive click hit-testing (Design §3.4) — empty/None for read-only panes.
    #[allow(clippy::type_complexity)]
    let (lines, forced_scroll, hint_base, row_of_line, row_kind): (
        Vec<Line>,
        Option<usize>,
        String,
        Vec<Option<usize>>,
        Option<RowKind>,
    ) = match &overlay.content {
        OverlayContent::Diff(unified) => (
            crate::diffview::render_unified(unified, theme),
            None,
            " Esc close · ↑↓ PgUp/PgDn scroll".to_string(),
            Vec::new(),
            None,
        ),
        OverlayContent::Text(body) => (
            body.split('\n')
                .flat_map(|l| text::wrap(l, body_w))
                .map(|row| Line::from(Span::styled(row, theme.primary())))
                .collect(),
            None,
            " Esc close · ↑↓ PgUp/PgDn scroll".to_string(),
            Vec::new(),
            None,
        ),
        OverlayContent::Sessions { rows, selected } => {
            let (lines, sel_line, map) = session_picker_lines(rows, *selected, theme);
            (
                lines,
                Some(sel_line),
                " Enter resume · ↑↓ move · Esc close".to_string(),
                map,
                Some(RowKind::Session),
            )
        }
        OverlayContent::Choices { rows, selected, .. } => {
            let (lines, sel_line, map) = choice_picker_lines(rows, *selected, theme);
            (
                lines,
                Some(sel_line),
                " Enter select · ↑↓ move · Esc close".to_string(),
                map,
                Some(RowKind::Choice),
            )
        }
        OverlayContent::MemoryEntries {
            user,
            project,
            selected,
            confirm_delete,
        } => {
            let (lines, sel_line, map) = memory_inspector_lines(user, project, *selected, theme);
            let hint = if *confirm_delete {
                let name = user
                    .iter()
                    .chain(project.iter())
                    .nth(*selected)
                    .map_or("", |e| e.name.as_str());
                format!(
                    "{}{}{}",
                    strings::memory::CONFIRM_PREFIX,
                    name,
                    strings::memory::CONFIRM_SUFFIX
                )
            } else {
                strings::memory::HINT.to_string()
            };
            (lines, Some(sel_line), hint, map, Some(RowKind::Memory))
        }
        OverlayContent::SkillList { skills, selected } => {
            let (lines, sel_line, map) = skill_list_lines(skills, *selected, theme);
            (
                lines,
                Some(sel_line),
                strings::skills::HINT.to_string(),
                map,
                Some(RowKind::Skill),
            )
        }
    };

    let total = lines.len();
    let max_scroll = total.saturating_sub(body_h);
    // A picker scroll keeps the selected row a couple of lines below the top;
    // everything else uses the overlay's own scroll offset.
    let scroll = forced_scroll
        .map_or(overlay.scroll, |sel| sel.saturating_sub(2))
        .min(max_scroll);
    let end = (scroll + body_h).min(total);
    let visible: Vec<Line> = lines[scroll..end].to_vec();

    let body_area = Rect {
        height: inner.height - 1,
        ..inner
    };
    let visible_len = visible.len();
    f.render_widget(Paragraph::new(visible), body_area);

    // Clickable rows for a selectable-list overlay (Design §3.4). Each visible
    // line maps to a logical row via `row_of_line` (`None` for headers/spacers);
    // a click on a row-line maps to "select this row + picker Enter". Read-only
    // panes (Diff/Text) have `row_kind == None` and push nothing (the clear
    // above still blocks click-through).
    if let Some(kind) = row_kind {
        for offset in 0..visible_len {
            let Some(Some(row)) = row_of_line.get(scroll + offset).copied() else {
                continue; // a header/spacer line — not selectable
            };
            let y = body_area.y + u16::try_from(offset).unwrap_or(0);
            hit.push(
                Rect {
                    x: body_area.x,
                    y,
                    width: body_area.width,
                    height: 1,
                },
                kind.target(row),
            );
        }
    }

    let more = if scroll < max_scroll {
        "  ↓ more"
    } else {
        ""
    };
    let hint = format!("{hint_base}{more}");
    let hint_area = Rect {
        y: inner.y + inner.height - 1,
        height: 1,
        ..inner
    };
    f.render_widget(Paragraph::new(hint).style(theme.chrome()), hint_area);
}

/// The return of a selectable-list builder: the rendered `lines`, the selected
/// row's line index (to scroll it into view), and `row_of_line` — the logical
/// row each line maps to for click hit-testing (`None` for headers/spacers/empty
/// messages), Design §3.4. `row_of_line.len() == lines.len()`.
type PickerLines = (Vec<Line<'static>>, usize, Vec<Option<usize>>);

/// Render the session picker as selectable rows (title + metadata), returning
/// the lines, the line index of the selected row (so the pane can scroll it into
/// view), and a per-line `row_of_line` map (which logical row each line belongs
/// to, `None` for spacers) for click hit-testing (Design §3.4). The selected row
/// is marked and accented.
fn session_picker_lines(
    rows: &[SessionRow],
    selected: usize,
    theme: &crate::theme::Theme,
) -> PickerLines {
    let mut lines: Vec<Line> = Vec::new();
    let mut row_of_line: Vec<Option<usize>> = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "No saved sessions yet.".to_string(),
            theme.chrome(),
        )));
        row_of_line.push(None);
        return (lines, 0, row_of_line);
    }
    let mut sel_line = 0;
    for (i, row) in rows.iter().enumerate() {
        if i == selected {
            sel_line = lines.len();
        }
        let marker = if i == selected { "▶ " } else { "  " };
        let title_style = if i == selected {
            theme.strong()
        } else {
            theme.primary()
        };
        let mut title_spans = vec![
            Span::styled(marker.to_string(), theme.accent()),
            Span::styled(row.title.clone(), title_style),
        ];
        if row.current {
            title_spans.push(Span::styled("  (current)".to_string(), theme.success()));
        }
        lines.push(Line::from(title_spans));
        row_of_line.push(Some(i));
        // A short id prefix — enough to recognise, matching `emberly resume`.
        let short = row.id.to_string();
        let short = short.get(..8).unwrap_or(&short);
        lines.push(Line::from(Span::styled(
            format!("    {short} · {}", row.subtitle),
            theme.chrome(),
        )));
        row_of_line.push(Some(i)); // clicking the metadata line selects the row too
        lines.push(Line::from(String::new()));
        row_of_line.push(None); // spacer
    }
    (lines, sel_line, row_of_line)
}

/// Render a generic choice picker (`/model`, C-6) as selectable rows, returning
/// the lines, the selected line index (to scroll it into view), and the
/// `row_of_line` map (one row per line here) for click hit-testing. The active
/// choice is marked; the selected one is accented.
fn choice_picker_lines(
    rows: &[ChoiceRow],
    selected: usize,
    theme: &crate::theme::Theme,
) -> PickerLines {
    let mut lines: Vec<Line> = Vec::new();
    let mut row_of_line: Vec<Option<usize>> = Vec::new();
    let mut sel_line = 0;
    for (i, row) in rows.iter().enumerate() {
        if i == selected {
            sel_line = lines.len();
        }
        let marker = if i == selected { "▶ " } else { "  " };
        let style = if i == selected {
            theme.strong()
        } else {
            theme.primary()
        };
        let mut spans = vec![
            Span::styled(marker.to_string(), theme.accent()),
            Span::styled(row.label.clone(), style),
        ];
        if row.current {
            spans.push(Span::styled("  (current)".to_string(), theme.success()));
        }
        lines.push(Line::from(spans));
        row_of_line.push(Some(i));
    }
    (lines, sel_line, row_of_line)
}

/// Render the memory inspector (`/memory`, FR-6, Design §4.9): entries grouped
/// by scope, each `name — description`, with a dimmed scope header per group.
/// Origin (the header) is how the user reads trust; the selected entry is
/// marked and accented. Returns the lines and the selected entry's line index
/// (so the pane scrolls it into view). `selected` indexes the flattened list
/// (user entries, then project entries).
fn memory_inspector_lines(
    user: &[EntrySummary],
    project: &[EntrySummary],
    selected: usize,
    theme: &crate::theme::Theme,
) -> PickerLines {
    let mut lines: Vec<Line> = Vec::new();
    let mut row_of_line: Vec<Option<usize>> = Vec::new();
    if user.is_empty() && project.is_empty() {
        lines.push(Line::from(Span::styled(
            strings::memory::EMPTY.to_string(),
            theme.chrome(),
        )));
        row_of_line.push(None);
        return (lines, 0, row_of_line);
    }
    let mut sel_line = 0;
    // A running index over the flattened entry list, matched against `selected`.
    let mut gi = 0usize;
    for (entries, header) in [
        (user, strings::memory::HEADER_USER),
        (project, strings::memory::HEADER_PROJECT),
    ] {
        if entries.is_empty() {
            continue;
        }
        lines.push(Line::from(Span::styled(header.to_string(), theme.chrome())));
        row_of_line.push(None); // scope header
        for entry in entries {
            if gi == selected {
                sel_line = lines.len();
            }
            let marker = if gi == selected { "▶ " } else { "  " };
            let name_style = if gi == selected {
                theme.strong()
            } else {
                theme.primary()
            };
            let mut spans = vec![
                Span::styled(marker.to_string(), theme.accent()),
                Span::styled(entry.name.clone(), name_style),
            ];
            if !entry.description.is_empty() {
                spans.push(Span::styled(
                    format!(" — {}", entry.description),
                    theme.chrome(),
                ));
            }
            lines.push(Line::from(spans));
            row_of_line.push(Some(gi));
            gi += 1;
        }
        lines.push(Line::from(String::new()));
        row_of_line.push(None); // spacer
    }
    (lines, sel_line, row_of_line)
}

/// Render the skills inspector (`/skills`, FR-7, Design §4.9): the available
/// skills as `name — description` rows with a dimmed `(origin)` suffix (origin
/// is how the user reads trust). The selected row is marked and accented.
/// Returns the lines and the selected row's line index (to scroll it into
/// view).
fn skill_list_lines(
    skills: &[SkillMeta],
    selected: usize,
    theme: &crate::theme::Theme,
) -> PickerLines {
    let mut lines: Vec<Line> = Vec::new();
    let mut row_of_line: Vec<Option<usize>> = Vec::new();
    if skills.is_empty() {
        lines.push(Line::from(Span::styled(
            strings::skills::EMPTY.to_string(),
            theme.chrome(),
        )));
        row_of_line.push(None);
        return (lines, 0, row_of_line);
    }
    let mut sel_line = 0;
    for (i, skill) in skills.iter().enumerate() {
        if i == selected {
            sel_line = lines.len();
        }
        let marker = if i == selected { "▶ " } else { "  " };
        let name_style = if i == selected {
            theme.strong()
        } else {
            theme.primary()
        };
        let origin = match skill.origin {
            SkillOrigin::User => strings::skills::ORIGIN_USER,
            SkillOrigin::Project => strings::skills::ORIGIN_PROJECT,
        };
        let mut spans = vec![
            Span::styled(marker.to_string(), theme.accent()),
            Span::styled(skill.name.clone(), name_style),
        ];
        if !skill.description.is_empty() {
            spans.push(Span::styled(
                format!(" — {}", skill.description),
                theme.chrome(),
            ));
        }
        spans.push(Span::styled(format!("  ({origin})"), theme.chrome()));
        lines.push(Line::from(spans));
        row_of_line.push(Some(i));
    }
    (lines, sel_line, row_of_line)
}

/// A rectangle centered in `area` at the given width/height percentages.
fn centered(area: Rect, pct_w: u16, pct_h: u16) -> Rect {
    let [h] = Layout::horizontal([Constraint::Percentage(pct_w)])
        .flex(Flex::Center)
        .areas(area);
    let [v] = Layout::vertical([Constraint::Percentage(pct_h)])
        .flex(Flex::Center)
        .areas(h);
    v
}

// ---- conversation --------------------------------------------------------

fn render_conversation(f: &mut Frame, app: &App, area: Rect, hit: &mut HitMap) {
    let theme = &app.theme;

    // No pane title — the wordmark lives in the sidebar. The conversation is a
    // plain bordered transcript of both sides, top to bottom.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.chrome());
    let inner = block.inner(area);
    let width = usize::from(inner.width);
    let height = usize::from(inner.height);

    // Pre-wrap the whole conversation to the pane width (Thai-safe, via the
    // text engine) so scroll math counts real display rows. `reasoning_line`
    // comes back as the line index of the *most recent* reasoning header — the
    // one Ctrl+R toggles — so a click on it can mirror that key (Design §3.4).
    let mut reasoning_line: Option<usize> = None;
    let lines = conversation_lines(app, width, &mut reasoning_line);
    let total = lines.len();
    let max_scroll = total.saturating_sub(height);
    let scroll = app.scroll.min(max_scroll);
    let end = total - scroll;
    let start = end.saturating_sub(height);
    let visible: Vec<Line> = lines[start..end].to_vec();

    // The collapsed/expanded reasoning trail is clickable when it is on screen
    // (Design §3.4): a click toggles it exactly as Ctrl+R does. The conversation
    // is pre-wrapped (one line == one screen row), so the screen row is simply
    // the line's offset from the top of the visible window.
    if let Some(idx) = reasoning_line {
        if idx >= start && idx < end {
            let y = inner.y + u16::try_from(idx - start).unwrap_or(0);
            hit.push(
                Rect {
                    x: inner.x,
                    y,
                    width: inner.width,
                    height: 1,
                },
                ClickTarget::ReasoningToggle,
            );
        }
    }

    // A dim hint when scrolled up, so it is obvious there is newer content.
    let block = if scroll > 0 {
        block.title(Line::from(Span::styled(" ↓ more ", theme.chrome())).right_aligned())
    } else {
        block
    };
    f.render_widget(block, area);
    f.render_widget(Paragraph::new(visible), inner);
}

/// Flatten the conversation into styled, pre-wrapped display rows. `reasoning_line`
/// is set to the line index of the **most recent** reasoning header (for click
/// hit-testing, Design §3.4) — it is overwritten on each reasoning item so it
/// ends pointing at the last one, matching `toggle_reasoning`/Ctrl+R.
fn conversation_lines(
    app: &App,
    width: usize,
    reasoning_line: &mut Option<usize>,
) -> Vec<Line<'static>> {
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
                // First-party markdown pass: fenced code (highlighted), bold,
                // inline code, lists, headings; everything else plain (§4.1).
                out.extend(crate::markdown::render_message(text, theme, w));
            }
            ConvItem::Reasoning { text, expanded } => {
                // A quiet register, one visual step below the answer (Design
                // §4.4): a dim summary line, and — when expanded — the reasoning
                // in chrome colour so it never reads as the conclusion.
                let lines = text.lines().count().max(1);
                let (mark, label) = if *expanded {
                    (markers::REASONING_EXPANDED, "reasoning".to_string())
                } else {
                    (
                        markers::REASONING_COLLAPSED,
                        format!("reasoning ({lines} lines)"),
                    )
                };
                // Record this header's line index; the last one wins, matching
                // Ctrl+R's "toggle the most recent reasoning" (Design §3.4).
                *reasoning_line = Some(out.len());
                out.push(Line::from(vec![Span::styled(
                    format!("{mark} {label}"),
                    theme.chrome(),
                )]));
                if *expanded {
                    for row in text
                        .split('\n')
                        .flat_map(|l| text::wrap(l, w.saturating_sub(2)))
                    {
                        out.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(row, theme.chrome()),
                        ]));
                    }
                }
            }
            ConvItem::Tool {
                summary,
                explanation,
                done,
                result,
                preview,
                untrusted,
                ..
            } => {
                // Header line: mark + the descriptive summary (a verb phrase
                // like "run: cargo test"), then the result status when done.
                let (mark, style) = match done {
                    None => (markers::RUNNING, theme.chrome()),
                    Some(true) => (markers::OK, theme.success()),
                    Some(false) => (markers::FAILED, theme.error()),
                };
                let mut spans = vec![
                    Span::styled(format!("[{mark}] "), style),
                    Span::styled(summary.clone(), theme.primary()),
                ];
                if let Some(result) = result {
                    spans.push(Span::styled(format!(" — {result}"), theme.chrome()));
                }
                out.push(Line::from(spans));

                // The model's caption (T-9, Design §4.5): a single dim line
                // directly under the call, led by a marker so it reads as an
                // annotation, not tool output. Absent → nothing (no placeholder).
                // Dim + marker + position carry it — never styled as a result or
                // error, never meaning-by-colour (Design §4.5/§7).
                if let Some(explanation) = explanation {
                    let lead = format!("    {} ", markers::EXPLANATION);
                    for (i, row) in text::wrap(explanation, w.saturating_sub(6))
                        .into_iter()
                        .enumerate()
                    {
                        let prefix = if i == 0 {
                            lead.clone()
                        } else {
                            "      ".into()
                        };
                        out.push(Line::from(vec![
                            Span::raw(prefix),
                            Span::styled(row, theme.chrome()),
                        ]));
                    }
                }

                // Result preview: a few indented, dimmed lines of the output so
                // the user sees what the tool produced (Design §6.1).
                if let Some(preview) = preview {
                    if *untrusted {
                        // Untrusted web content (T-14, Design §4.10): rendered as
                        // fetched web data with a visible label and source URLs —
                        // never in harness or assistant voice, so a hostile snippet
                        // reads visibly as quoted web text, not as something the
                        // agent is telling the user to do.
                        out.push(Line::from(vec![
                            Span::raw("    "),
                            Span::styled(
                                format!("{} untrusted web content", markers::WEB),
                                theme.chrome(),
                            ),
                        ]));
                    }
                    let style = if *untrusted {
                        theme.chrome()
                    } else if *done == Some(false) {
                        theme.error()
                    } else {
                        theme.chrome()
                    };
                    for row in preview
                        .split('\n')
                        .flat_map(|l| text::wrap(l, w.saturating_sub(4)))
                    {
                        out.push(Line::from(vec![
                            Span::raw("    "),
                            Span::styled(row, style),
                        ]));
                    }
                }
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
            ConvItem::Diff { unified } => {
                // Inline diff on execute (Design §4.2), capped — the full diff
                // is one Ctrl+O away in the overlay.
                let (rows, hidden) =
                    crate::diffview::render_unified_capped(unified, theme, INLINE_DIFF_CAP);
                out.extend(rows);
                if hidden > 0 {
                    out.push(Line::from(Span::styled(
                        format!("  … {hidden} more lines — Ctrl+O to view"),
                        theme.chrome(),
                    )));
                }
            }
            ConvItem::TaskList { items } => {
                // Inline checklist block (T-11, Design §4.7). A completed list
                // settles to an all-✓ block rather than vanishing.
                for item in items {
                    let (glyph, style) = match item.status {
                        TaskStatus::Pending => (markers::TASK_PENDING, theme.chrome()),
                        TaskStatus::InProgress => (markers::TASK_IN_PROGRESS, theme.accent()),
                        TaskStatus::Done => (markers::TASK_DONE, theme.success()),
                    };
                    for (i, row) in text::wrap(&item.text, w.saturating_sub(4))
                        .into_iter()
                        .enumerate()
                    {
                        if i == 0 {
                            out.push(Line::from(vec![
                                Span::raw("  "),
                                Span::styled(format!("{glyph} "), style),
                                Span::styled(row, theme.primary()),
                            ]));
                        } else {
                            out.push(Line::from(vec![
                                Span::raw("    "),
                                Span::styled(row, theme.primary()),
                            ]));
                        }
                    }
                }
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

fn render_sidebar(f: &mut Frame, app: &App, area: Rect, hit: &mut HitMap, interactive: bool) {
    let theme = &app.theme;
    let block = Block::default()
        .borders(Borders::LEFT)
        .border_style(theme.chrome());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let w = usize::from(inner.width).saturating_sub(1);
    let mut lines: Vec<Line> = Vec::new();
    // Line-index ranges of the clickable sections, filled as they are built.
    // The sidebar renders **without wrap** (each logical line is one screen row),
    // so a range `[start, end)` maps directly to screen rows for hit-testing.
    let mut modified_range: Option<(usize, usize)> = None;
    let mut memory_range: Option<(usize, usize)> = None;
    let mut skills_range: Option<(usize, usize)> = None;

    // Wordmark + version (Design §1.1): ember `emberly`, dimmed `code` + version.
    // While the model is working, the wordmark breathes — the ember glowing
    // (Design §6.4). Ambient only; it carries no information.
    let wordmark_style = if app.is_working() {
        ratatui::style::Style::default()
            .fg(glow(theme.palette().accent, glow_pct(app.anim_frame())))
            .add_modifier(ratatui::style::Modifier::BOLD)
    } else {
        theme.accent()
    };
    lines.push(Line::from(vec![
        Span::styled(strings::brand::NAME, wordmark_style),
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
    // Effort line: shown only when the active model exposes a control (P-9,
    // Design §3.1); hidden otherwise so it never implies a knob that does
    // nothing.
    if let Some(effort) = app.effort {
        lines.push(labeled(
            theme,
            strings::status::EFFORT_LABEL,
            effort.as_str(),
            w,
        ));
    }
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
    // Cumulative session tokens (in + out), always available (Design §3.1).
    let usage = &app.session_usage;
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} ", strings::status::TOKENS_LABEL),
            theme.chrome(),
        ),
        Span::styled(compact_count(usage.input + usage.output), theme.primary()),
        Span::styled(
            format!(
                "  ({}{} {}{})",
                compact_count(usage.input),
                strings::status::TOKENS_IN,
                compact_count(usage.output),
                strings::status::TOKENS_OUT
            ),
            theme.chrome(),
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
    let modified_start = lines.len();
    lines.push(Line::from(Span::styled(
        strings::status::MODIFIED_FILES_TITLE,
        theme.chrome(),
    )));
    if app.modified_files.is_empty() {
        lines.push(Line::from(Span::styled("  —", theme.chrome())));
    } else {
        // Clickable → open the diff (Ctrl+O twin); range set after the rows.
        let last = app.modified_files.len() - 1;
        for (i, file) in app.modified_files.iter().enumerate() {
            let counts = format!(" +{} -{}", file.adds, file.dels);
            let path_w = w.saturating_sub(text::width(&counts));
            // The newest entry briefly settles in on the accent (Design §6.4).
            let path_style = if i == last && app.sidebar_settling() {
                theme.accent()
            } else {
                theme.primary()
            };
            lines.push(Line::from(vec![
                Span::styled(fit(&file.path, path_w), path_style),
                Span::styled(format!(" +{}", file.adds), theme.diff_add()),
                Span::styled(format!(" -{}", file.dels), theme.diff_del()),
            ]));
        }
        modified_range = Some((modified_start, lines.len()));
    }

    // Task list (T-11, Design §3.1/§4.7): one line per item with a status
    // glyph. Not shown when empty (Design §3.1). The single in-progress item
    // is lightly ember-accented; color is never the sole signal (Design §7).
    if !app.tasks.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("Tasks", theme.chrome())));
        for item in &app.tasks {
            let (glyph, style) = match item.status {
                TaskStatus::Pending => (markers::TASK_PENDING, theme.chrome()),
                TaskStatus::InProgress => (markers::TASK_IN_PROGRESS, theme.accent()),
                TaskStatus::Done => (markers::TASK_DONE, theme.success()),
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{glyph} "), style),
                Span::styled(fit(&item.text, w.saturating_sub(2)), theme.primary()),
            ]));
        }
    }

    // Memory counts (T-13, FR-6, Design §4.9): per-scope entry counts, not
    // bodies. An empty store shows no section (Design §3.1).
    if app.memory_user > 0 || app.memory_project > 0 {
        lines.push(Line::from(""));
        let start = lines.len();
        lines.push(Line::from(Span::styled("Memory", theme.chrome())));
        lines.push(Line::from(Span::styled(
            format!("user {} · project {}", app.memory_user, app.memory_project),
            theme.primary(),
        )));
        memory_range = Some((start, lines.len())); // clickable → open /memory
    }

    // Skills catalog (T-15, FR-7, Design §4.9): name, description, and origin
    // per skill. Origin (user vs project) is how the user reads trust. An empty
    // catalog shows no section (Design §3.1). No color-only signal (Design §7).
    if !app.skills.is_empty() {
        lines.push(Line::from(""));
        let start = lines.len();
        lines.push(Line::from(Span::styled("Skills", theme.chrome())));
        for skill in &app.skills {
            let origin = match skill.origin {
                emberly_core::SkillOrigin::User => "user",
                emberly_core::SkillOrigin::Project => "project",
            };
            let desc = if skill.description.is_empty() {
                format!("{} ({})", skill.name, origin)
            } else {
                format!("{} — {} ({})", skill.name, skill.description, origin)
            };
            // `fit` so the row stays one screen line (the sidebar no longer
            // wraps — see below); the full text is in the `/skills` inspector.
            lines.push(Line::from(Span::styled(fit(&desc, w), theme.primary())));
        }
        skills_range = Some((start, lines.len())); // clickable → open /skills
    }

    // Render **without wrap** so each logical line is exactly one screen row
    // (ratatui truncates overflow) — this is what makes the sidebar's click
    // regions reliable (Design §3.4): line index `i` sits at screen row
    // `inner.y + i`. Long values are already `fit`-truncated above.
    f.render_widget(Paragraph::new(lines), inner);

    // Register the clickable sidebar sections (Design §3.4), each a parity-safe
    // shortcut for an existing action. Only on the base layer (`interactive`);
    // a modal on top clears/owns the map. A section is clickable only for the
    // rows actually drawn (line index < inner.height).
    if interactive {
        let mut push_section = |range: Option<(usize, usize)>, target: ClickTarget| {
            let Some((start, end)) = range else { return };
            let vis_end = end.min(usize::from(inner.height));
            if start >= vis_end {
                return; // scrolled/clipped off the drawn area
            }
            let y = inner.y + u16::try_from(start).unwrap_or(0);
            let height = u16::try_from(vis_end - start).unwrap_or(0);
            hit.push(
                Rect {
                    x: inner.x,
                    y,
                    width: inner.width,
                    height,
                },
                target,
            );
        };
        push_section(modified_range, ClickTarget::OpenDiff);
        push_section(memory_range, ClickTarget::OpenMemoryInspector);
        push_section(skills_range, ClickTarget::OpenSkillsInspector);
    }
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
    } else if app.pending_ask.is_some() {
        strings::ask_user::HINT
    } else if let Some(halt) = &app.pending_loop_halt {
        if halt.steering {
            strings::loop_halt::STEER_HINT
        } else {
            strings::loop_halt::HINT
        }
    } else {
        strings::hints::NORMAL
    };
    // Context % lives in the sidebar; show it on the status bar only when the
    // sidebar is collapsed, so it is never duplicated (Design §3.2). Mode is
    // status-bar-only, so it always appears here.
    let ctx = if sidebar_shown {
        String::new()
    } else {
        format!("{} {}%  ", strings::status::CONTEXT_ABBR, app.context_pct)
    };

    let mut spans: Vec<Span> = Vec::new();
    // The ember-pulse spinner + verb (+ elapsed after 5s) while the model works
    // (Design §6.3). Absent at idle, during a permission prompt, and it does not
    // fire for transient effects (overlay ease / sidebar settle).
    if app.is_working() {
        let verb = app.spinner_verb();
        let elapsed = app
            .spinner_elapsed()
            .map(|s| format!(" {s}s"))
            .unwrap_or_default();
        // The spinner breathes in lockstep with the wordmark glow (Design
        // §6.4): a single steady glyph whose accent brightness rises and falls
        // on the same raised-cosine cycle — one ember pulse, not two.
        let glyph_style = ratatui::style::Style::default()
            .fg(glow(theme.palette().accent, glow_pct(app.anim_frame())))
            .add_modifier(ratatui::style::Modifier::BOLD);
        spans.push(Span::styled(format!(" {} ", app.spinner_glyph()), glyph_style));
        spans.push(Span::styled(format!("{verb}{elapsed}  "), theme.chrome()));
    } else {
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(
        format!("{}  {ctx}{hints}", mode_name(app.mode)),
        theme.chrome(),
    ));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---- permission prompt — the most important screen (Design §5) -----------

/// Render the permission prompt: a pinned header (loud safety band for
/// outside-root, heading, why line, affected paths) over the **full,
/// scrollable content** (the command, or the diff rendered with diff colours),
/// over a pinned footer of choices. Deny is the default and the meaning of
/// Enter/Esc; allow (`y`/`s`) is deliberate. Nothing auto-scrolls, nothing is
/// truncated to fit, and no timer approves — see `App::on_permission_key`.
fn render_permission(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let Some((_, r)) = &app.pending_permission else {
        return;
    };

    // Outside-root escalation uses the reserved safety styling on the border
    // and a loud banner — impossible to mistake for a routine prompt (§5).
    let border = if r.outside_root {
        theme.error()
    } else {
        theme.dim_accent()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border)
        .title(Span::styled(
            format!(" {} ", strings::permission::TITLE),
            theme.accent(),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 || inner.width == 0 {
        return;
    }
    let width = usize::from(inner.width);

    // Pinned header.
    let mut header: Vec<Line> = Vec::new();
    if r.outside_root {
        header.push(Line::from(Span::styled(
            format!(" {} ", strings::permission::OUTSIDE_ROOT_BANNER),
            theme.safety_band(),
        )));
    }
    header.push(Line::from(vec![
        Span::styled(strings::permission::HEADING, theme.warning()),
        Span::styled(format!(": {}", r.summary), theme.strong()),
    ]));
    header.push(Line::from(Span::styled(
        format!("{}: {}", strings::permission::WHY_LABEL, r.reason),
        theme.chrome(),
    )));
    if !r.affected_paths.is_empty() {
        header.push(Line::from(Span::styled(
            format!(
                "{}: {}",
                strings::permission::PATHS_LABEL,
                r.affected_paths.join(", ")
            ),
            theme.chrome(),
        )));
    }
    header.push(Line::from("")); // divider blank

    // Body: the full content. Edit/write details are unified diffs; render them
    // with diff colours. Anything else (a command) is plain, wrapped text.
    let body: Vec<Line> = if is_unified_diff(&r.detail) {
        crate::diffview::render_unified(&r.detail, theme)
    } else {
        r.detail
            .split('\n')
            .flat_map(|l| text::wrap(l, width))
            .map(|row| Line::from(Span::styled(row, theme.primary())))
            .collect()
    };

    // Layout: header (fixed), body (scrollable, fills the middle), footer
    // (fixed, 2 rows). The footer's approve hint notes any content below the
    // fold, so approving always acknowledges there is more to see (§5).
    let header_h = u16::try_from(header.len()).unwrap_or(0);
    let footer_h = 2u16;
    let body_h = inner.height.saturating_sub(header_h + footer_h).max(1);
    let body_rows = usize::from(body_h);

    let total = body.len();
    let max_scroll = total.saturating_sub(body_rows);
    let scroll = app.permission_scroll.min(max_scroll);
    let end = (scroll + body_rows).min(total);
    let visible: Vec<Line> = body.get(scroll..end).unwrap_or(&[]).to_vec();
    let hidden_below = max_scroll - scroll;

    let header_area = Rect {
        height: header_h,
        ..inner
    };
    let body_area = Rect {
        y: inner.y + header_h,
        height: body_h,
        ..inner
    };
    let footer_area = Rect {
        y: inner.y + inner.height - footer_h,
        height: footer_h,
        ..inner
    };

    f.render_widget(Paragraph::new(header), header_area);
    f.render_widget(Paragraph::new(visible), body_area);
    f.render_widget(
        Paragraph::new(footer_lines(theme, hidden_below)),
        footer_area,
    );
}

// ---- question prompt — the model asking your opinion (Design §5.1) --------

/// Render the `ask_user` question prompt (T-8). **Calm and neutral** — the same
/// routine styling as a non-outside-root permission prompt (`dim_accent`
/// border), **never** the reserved safety band (Design §5.1/§2). The question,
/// any options as a selectable list, and a free-text answer that is always
/// available. There is no default selection and Enter never auto-answers — see
/// `App::on_ask_key`.
fn render_ask(f: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let Some(p) = &app.pending_ask else {
        return;
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.dim_accent())
        .title(Span::styled(
            format!(" {} ", strings::ask_user::TITLE),
            theme.accent(),
        ));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 || inner.width == 0 {
        return;
    }
    let width = usize::from(inner.width);

    let mut lines: Vec<Line> = Vec::new();
    // The question, wrapped — the first row carries the emphasis.
    for (i, row) in text::wrap(&p.question, width).into_iter().enumerate() {
        let style = if i == 0 {
            theme.strong()
        } else {
            theme.primary()
        };
        lines.push(Line::from(Span::styled(row, style)));
    }
    lines.push(Line::from(""));

    // Options as a selectable list. No highlight until the user moves to one
    // (no default selection — Design §5.1).
    for (i, opt) in p.options.iter().enumerate() {
        let selected = p.selected == Some(i);
        let (marker, style) = if selected {
            (markers::USER_PROMPT, theme.accent())
        } else {
            (" ", theme.primary())
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{marker} "), theme.accent()),
            Span::styled(format!("{}. {}", i + 1, opt), style),
        ]));
    }
    if !p.options.is_empty() {
        lines.push(Line::from(""));
    }

    // The free-text answer field, always available, with a caret.
    lines.push(Line::from(Span::styled(
        strings::ask_user::ANSWER_LABEL,
        theme.chrome(),
    )));
    lines.push(Line::from(vec![
        Span::styled(format!("{} ", markers::USER_PROMPT), theme.accent()),
        Span::styled(p.editor.text().to_string(), theme.primary()),
        Span::styled("▏", theme.accent()),
    ]));

    let footer_h = 1u16;
    let body_h = inner.height.saturating_sub(footer_h).max(1);
    let body_area = Rect {
        height: body_h,
        ..inner
    };
    let footer_area = Rect {
        y: inner.y + inner.height - footer_h,
        height: footer_h,
        ..inner
    };
    f.render_widget(Paragraph::new(lines), body_area);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            strings::ask_user::HINT,
            theme.chrome(),
        ))),
        footer_area,
    );
}

// ---- loop-halt surface — the harness stepping in (Design §8.5) ------------

/// Render the loop-halt surface (S-5). The **harness voice**, calm and
/// out-of-band — not model output, and deliberately distinct from the question
/// prompt (that is the model asking; this is the harness stepping in when the
/// model stopped progressing). No alarm styling, never the safety band; the
/// user always chooses keep-going / stop / say-something.
fn render_loop_halt(f: &mut Frame, app: &App, area: Rect) {
    use strings::loop_halt as s;
    let theme = &app.theme;
    let Some(halt) = &app.pending_loop_halt else {
        return;
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.chrome())
        .title(Span::styled(format!(" {} ", s::TITLE), theme.warning()));
    let inner = block.inner(area);
    f.render_widget(block, area);
    if inner.height < 3 || inner.width == 0 {
        return;
    }
    let width = usize::from(inner.width);

    let mut lines: Vec<Line> = Vec::new();
    // The calm harness line, then the specific reason.
    lines.push(Line::from(Span::styled(s::HEADING, theme.strong())));
    for row in text::wrap(&halt.reason, width) {
        lines.push(Line::from(Span::styled(row, theme.chrome())));
    }
    lines.push(Line::from(""));

    if halt.steering {
        // The steer field (hand a message back to the model).
        lines.push(Line::from(Span::styled(s::STEER_LABEL, theme.chrome())));
        lines.push(Line::from(vec![
            Span::styled(format!("{} ", markers::USER_PROMPT), theme.accent()),
            Span::styled(halt.editor.text().to_string(), theme.primary()),
            Span::styled("▏", theme.accent()),
        ]));
    } else {
        // The three choices.
        for (key, label) in [
            ("g", s::KEEP_GOING),
            ("s", s::STOP),
            ("t", s::SAY_SOMETHING),
        ] {
            lines.push(Line::from(vec![
                Span::styled(format!("  [{key}] "), theme.accent()),
                Span::styled(label, theme.primary()),
            ]));
        }
    }

    let footer_h = 1u16;
    let body_h = inner.height.saturating_sub(footer_h).max(1);
    let body_area = Rect {
        height: body_h,
        ..inner
    };
    let footer_area = Rect {
        y: inner.y + inner.height - footer_h,
        height: footer_h,
        ..inner
    };
    let hint = if halt.steering {
        s::STEER_HINT
    } else {
        s::HINT
    };
    f.render_widget(Paragraph::new(lines), body_area);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(hint, theme.chrome()))),
        footer_area,
    );
}

/// The pinned footer: a scroll notice (when content remains below) and the
/// choice line with deny as the default.
fn footer_lines(theme: &Theme, hidden_below: usize) -> Vec<Line<'static>> {
    let notice = if hidden_below > 0 {
        Line::from(Span::styled(
            format!(
                "↓ {hidden_below} more — {}",
                strings::permission::MORE_BELOW
            ),
            theme.warning(),
        ))
    } else {
        Line::from(Span::styled("— end of content —", theme.chrome()))
    };
    let choices = Line::from(vec![
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
    ]);
    vec![notice, choices]
}

/// Heuristic: a unified diff begins with a `--- ` file header.
fn is_unified_diff(detail: &str) -> bool {
    detail.starts_with("--- ") || detail.starts_with("---\n")
}

// ---- small helpers -------------------------------------------------------

/// Format a token count compactly: `950`, `12.3k`, `1.2M`.
fn compact_count(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    }
}

/// A brightness percentage that rises and falls as the ember "breathes" while
/// the model works (Design §6.4). One full cycle takes `PERIOD` frames (~3s at
/// 12fps) — an unhurried breath. Brightness follows a raised-cosine ease
/// (smooth and slow at both the dim and bright turning points, unlike a
/// triangle wave's abrupt apex) and swings 55%..=100% so the pulse is
/// perceptible but never flashes.
fn glow_pct(frame: usize) -> u16 {
    const PERIOD: usize = 36; // ~3s at 12fps
    const MIN: f32 = 55.0;
    const MAX: f32 = 100.0;
    let t = (frame % PERIOD) as f32 / PERIOD as f32; // 0.0..1.0
                                                     // (1 - cos) / 2 eases 0 → 1 → 0 across the cycle, flattening at both ends.
    let eased = (1.0 - (t * std::f32::consts::TAU).cos()) / 2.0;
    (MIN + (MAX - MIN) * eased).round() as u16
}

/// Scale an RGB colour's brightness by `pct` percent (non-RGB colours pass
/// through unchanged).
fn glow(color: Color, pct: u16) -> Color {
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(scale(r, pct), scale(g, pct), scale(b, pct)),
        other => other,
    }
}

fn scale(channel: u8, pct: u16) -> u8 {
    u8::try_from((u16::from(channel) * pct / 100).min(255)).unwrap_or(channel)
}

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
    use emberly_core::{PermissionId, PermissionRendering, UiEvent};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    #[test]
    fn glow_breathes_between_dim_and_full_over_a_smooth_cycle() {
        // Dimmest at the start of the cycle, brightest at the midpoint (~1.5s in).
        assert_eq!(glow_pct(0), 55);
        assert_eq!(glow_pct(18), 100);
        // Every frame stays inside the perceptible-but-never-flashing band.
        for frame in 0..200 {
            let pct = glow_pct(frame);
            assert!((55..=100).contains(&pct), "frame {frame} → {pct}%");
        }
        // The ease is gentle near the trough: the first step barely moves,
        // where a linear triangle would have jumped ~6%.
        assert!(
            glow_pct(1) - glow_pct(0) <= 1,
            "eased start, not a linear ramp"
        );
    }

    /// Render a full frame to an off-screen buffer and flatten it to text (one
    /// screen row per line), for asserting what actually appears on screen.
    fn draw(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("backend");
        let mut hit = HitMap::new();
        term.draw(|f| frame(f, app, &mut hit)).expect("draw");
        let buf = term.backend().buffer();
        let width = usize::from(buf.area.width);
        buf.content
            .chunks(width)
            .map(|chunk| {
                chunk
                    .iter()
                    .map(ratatui::buffer::Cell::symbol)
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Render a full frame and return the click hit-map it built, so tests can
    /// assert which screen positions resolve to which targets (Design §3.4).
    fn hit_map_of(app: &App, w: u16, h: u16) -> HitMap {
        let mut term = Terminal::new(TestBackend::new(w, h)).expect("backend");
        let mut hit = HitMap::new();
        term.draw(|f| frame(f, app, &mut hit)).expect("draw");
        hit
    }

    #[test]
    fn palette_rows_populate_the_hit_map() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.palette = Some(crate::app::PaletteState::default());
        let hit = hit_map_of(&app, 100, 24);
        // The first two filtered rows are clickable at consecutive screen rows.
        let y0 = (0..24).find(|&y| hit.hit(50, y) == Some(ClickTarget::PaletteRow(0)));
        let y1 = (0..24).find(|&y| hit.hit(50, y) == Some(ClickTarget::PaletteRow(1)));
        assert!(y0.is_some(), "palette row 0 is clickable");
        assert_eq!(
            y1,
            y0.map(|y| y + 1),
            "consecutive rows occupy adjacent screen rows"
        );
    }

    #[test]
    fn choice_picker_rows_populate_the_hit_map() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.overlays.push(Overlay {
            title: "permission mode".into(),
            content: OverlayContent::Choices {
                kind: crate::app::ChoiceKind::Mode,
                rows: vec![
                    ChoiceRow {
                        label: "normal".into(),
                        current: true,
                    },
                    ChoiceRow {
                        label: "auto-accept edits".into(),
                        current: false,
                    },
                ],
                selected: 0,
            },
            scroll: 0,
        });
        let hit = hit_map_of(&app, 100, 24);
        let y0 = (0..24).find(|&y| hit.hit(50, y) == Some(ClickTarget::ChoiceRow(0)));
        let y1 = (0..24).find(|&y| hit.hit(50, y) == Some(ClickTarget::ChoiceRow(1)));
        assert!(y0.is_some(), "choice row 0 is clickable");
        assert_eq!(y1, y0.map(|y| y + 1), "choice rows are adjacent");
    }

    #[test]
    fn memory_inspector_rows_populate_the_hit_map() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.apply_event(emberly_core::UiEvent::MemoryEntries {
            user: vec![EntrySummary {
                name: "Alpha".into(),
                description: "a".into(),
                type_: None,
                scope: emberly_core::MemoryScope::User,
            }],
            project: vec![EntrySummary {
                name: "Proj".into(),
                description: "p".into(),
                type_: None,
                scope: emberly_core::MemoryScope::Project,
            }],
        });
        let hit = hit_map_of(&app, 100, 24);
        // The two entries (flattened user-then-project) are clickable; the scope
        // header lines resolve to nothing.
        let r0 = (0..24).find_map(|y| match hit.hit(50, y) {
            Some(ClickTarget::MemoryRow(r)) => Some(r),
            _ => None,
        });
        let rows: Vec<usize> = (0..24)
            .filter_map(|y| match hit.hit(50, y) {
                Some(ClickTarget::MemoryRow(r)) => Some(r),
                _ => None,
            })
            .collect();
        assert_eq!(r0, Some(0), "the first entry is clickable");
        assert!(rows.contains(&1), "the project entry (row 1) is clickable");
    }

    #[test]
    fn reasoning_trail_is_clickable() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.apply_event(emberly_core::UiEvent::ReasoningDelta {
            text: "why".into(),
        });
        app.apply_event(emberly_core::UiEvent::AssistantDelta { text: "a".into() });
        let hit = hit_map_of(&app, 100, 24);
        let found = (0..24).any(|y| {
            (0..100).any(|x| hit.hit(x, y) == Some(ClickTarget::ReasoningToggle))
        });
        assert!(found, "the collapsed reasoning line is clickable");
    }

    #[test]
    fn sidebar_sections_populate_the_hit_map() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.modified_files.push(crate::app::ModifiedFile {
            path: "src/x.rs".into(),
            adds: 3,
            dels: 1,
        });
        app.memory_user = 2;
        app.memory_project = 1;
        app.skills.push(SkillMeta {
            name: "review".into(),
            description: "d".into(),
            origin: SkillOrigin::User,
        });
        // Wide enough for the sidebar to show (>= COLLAPSE_BELOW).
        let hit = hit_map_of(&app, 120, 40);
        let has = |t: ClickTarget| (0..40).any(|y| (0..120).any(|x| hit.hit(x, y) == Some(t)));
        assert!(has(ClickTarget::OpenDiff), "modified files → open diff");
        assert!(has(ClickTarget::OpenMemoryInspector), "Memory → inspector");
        assert!(has(ClickTarget::OpenSkillsInspector), "Skills → inspector");
    }

    #[test]
    fn sidebar_is_not_clickable_behind_a_permission_prompt() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.memory_user = 1;
        pending(&mut app, false, "rm -rf build"); // a permission prompt owns input
        let hit = hit_map_of(&app, 120, 40);
        assert!(
            (0..40).all(|y| (0..120)
                .all(|x| hit.hit(x, y) != Some(ClickTarget::OpenMemoryInspector))),
            "the sidebar is inert while a permission decision is pending (§5)"
        );
    }

    #[test]
    fn a_read_only_overlay_blocks_click_through() {
        // A Text overlay is read-only: it clears the hit-map (no click-through
        // to the conversation behind) and pushes no rows.
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.open_text_overlay("t", "some body text");
        let hit = hit_map_of(&app, 100, 24);
        assert!(
            (0..24).all(|y| (0..100).all(|x| hit.hit(x, y).is_none())),
            "nothing under a read-only overlay is clickable"
        );
    }

    fn pending(app: &mut App, outside_root: bool, detail: &str) {
        app.apply_event(UiEvent::PermissionRequest {
            id: PermissionId(1),
            rendering: PermissionRendering {
                tool: "bash".into(),
                summary: "run: rm -rf build".into(),
                detail: detail.into(),
                affected_paths: vec!["/etc/x".into()],
                outside_root,
                reason: "bash requires approval".into(),
            },
        });
    }

    #[test]
    fn permission_prompt_shows_full_content_and_deny_default() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        pending(&mut app, false, "rm -rf build");
        let screen = draw(&app, 100, 24);
        assert!(screen.contains("PERMISSION REQUIRED"));
        assert!(screen.contains("rm -rf build"), "full command shown");
        assert!(screen.contains("DENY"), "deny is offered as the default");
        assert!(screen.contains("allow once"));
        // No input box while deciding — nothing can be typed into the choice.
        assert!(!screen.contains("Enter send"));
    }

    #[test]
    fn outside_root_prompt_is_loud() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        pending(&mut app, true, "rm -rf /etc/x");
        let screen = draw(&app, 100, 24);
        assert!(
            screen.contains("OUTSIDE YOUR PROJECT"),
            "loud banner for outside-root escalation"
        );
    }

    #[test]
    fn long_content_reports_more_below() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        let long: String = (0..80).map(|i| format!("line {i}\n")).collect();
        pending(&mut app, false, &long);
        // A short screen forces the content to overflow the prompt body.
        let screen = draw(&app, 100, 12);
        assert!(
            screen.contains("more"),
            "approve hint indicates content below the fold (Design §5)"
        );
    }

    #[test]
    fn tool_call_shows_what_and_result_and_output() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        let id = emberly_core::ToolCallId::new("c1");
        app.apply_event(UiEvent::ToolStarted {
            call_id: id.clone(),
            tool: "bash".into(),
            summary: "run: ls -la".into(),
            explanation: None,
        });
        app.apply_event(UiEvent::ToolFinished {
            call_id: id,
            ok: true,
            summary: "exit 0".into(),
            preview: "total 8\nsrc\nCargo.toml".into(),
            untrusted: false,
        });
        let screen = draw(&app, 100, 24);
        assert!(screen.contains("run: ls -la"), "shows what the tool did");
        assert!(screen.contains("exit 0"), "shows the result status");
        assert!(
            screen.contains("Cargo.toml"),
            "shows a preview of the output"
        );
    }

    #[test]
    fn tool_call_explanation_renders_as_a_dim_caption() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.apply_event(UiEvent::ToolStarted {
            call_id: emberly_core::ToolCallId::new("c1"),
            tool: "bash".into(),
            summary: "run: sed -i s/debug/info/ log.conf".into(),
            explanation: Some("raise the log level to info".into()),
        });
        let screen = draw(&app, 100, 24);
        assert!(
            screen.contains("raise the log level to info"),
            "caption shown under the call"
        );
        assert!(
            screen.contains(crate::strings::markers::EXPLANATION),
            "caption led by the annotation marker — meaning without colour (Design §4.5)"
        );
    }

    #[test]
    fn no_explanation_shows_no_caption() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.apply_event(UiEvent::ToolStarted {
            call_id: emberly_core::ToolCallId::new("c1"),
            tool: "read_file".into(),
            summary: "read src/main.rs".into(),
            explanation: None,
        });
        let screen = draw(&app, 100, 24);
        assert!(
            !screen.contains(crate::strings::markers::EXPLANATION),
            "no caption marker when the model gave none — no placeholder (Design §4.5)"
        );
    }

    #[test]
    fn question_prompt_is_calm_not_a_permission_prompt() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.apply_event(UiEvent::AskUserRequest {
            id: emberly_core::AskId(1),
            question: "which environment?".into(),
            options: vec!["dev".into(), "prod".into()],
        });
        let screen = draw(&app, 100, 24);
        assert!(screen.contains("which environment?"), "question shown");
        assert!(
            screen.contains("dev") && screen.contains("prod"),
            "options shown"
        );
        assert!(screen.contains("question"), "calm title");
        // It must NOT borrow the permission prompt's loud safety vocabulary
        // (Design §5.1/§2 — the safety band stays rare).
        assert!(!screen.contains("PERMISSION REQUIRED"));
        assert!(!screen.contains("OUTSIDE YOUR PROJECT"));
    }

    #[test]
    fn loop_halt_is_harness_voice_not_a_prompt() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.apply_event(UiEvent::LoopHalted {
            reason: "read_file nope.txt three times".into(),
        });
        let screen = draw(&app, 100, 24);
        assert!(screen.contains("Stopped"), "harness voice heading");
        assert!(
            screen.contains("read_file nope.txt three times"),
            "the reason"
        );
        assert!(
            screen.contains("keep going") && screen.contains("stop here"),
            "the choices"
        );
        // Neither a permission prompt nor a question prompt.
        assert!(!screen.contains("PERMISSION REQUIRED"));
        assert!(!screen.contains("OUTSIDE YOUR PROJECT"));
    }

    #[test]
    fn thai_content_renders_in_the_conversation() {
        // A stacked-mark Thai word must survive into the rendered buffer intact
        // (grapheme-correct wrap, §2.1).
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.apply_event(UiEvent::AssistantDelta {
            text: "สวัสดี ที่".into(),
        });
        app.apply_event(UiEvent::AssistantDone);
        let screen = draw(&app, 120, 20);
        assert!(screen.contains("ที่"), "stacked Thai cluster rendered");
        assert!(screen.contains("สวัสดี"), "Thai word rendered");
    }

    #[test]
    fn sidebar_hides_below_the_collapse_threshold() {
        let app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        // Sidebar-only chrome present when wide, absent when narrow.
        assert!(draw(&app, 120, 20).contains("modified files"));
        assert!(!draw(&app, 80, 20).contains("modified files"));
    }

    #[test]
    fn sidebar_shows_session_token_total() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.apply_event(UiEvent::SessionUsage {
            usage: emberly_core::TokenUsage {
                input: 12_000,
                output: 3_400,
            },
        });
        let screen = draw(&app, 120, 20);
        assert!(screen.contains("tokens"), "token label in the sidebar");
        assert!(screen.contains("15.4k"), "total = input + output, compact");
    }

    #[test]
    fn compact_count_formats() {
        assert_eq!(compact_count(950), "950");
        assert_eq!(compact_count(15_400), "15.4k");
        assert_eq!(compact_count(1_200_000), "1.2M");
    }

    #[test]
    fn context_percent_only_on_status_bar_when_sidebar_hidden() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.apply_event(UiEvent::ContextUsage {
            pct: 42,
            tokens: 100,
        });
        // Wide: sidebar shown → ctx% is in the sidebar, not duplicated on the bar.
        let wide = draw(&app, 120, 20);
        let bar_wide = wide.lines().last().unwrap_or_default();
        assert!(
            !bar_wide.contains("ctx 42%"),
            "no ctx on bar when sidebar up"
        );
        assert!(wide.contains("42%"), "but present in the sidebar");
        // Narrow: sidebar collapsed → ctx% migrates to the status bar.
        let narrow = draw(&app, 80, 20);
        assert!(narrow.contains("ctx 42%"), "ctx on bar when collapsed");
    }

    #[test]
    fn edit_prompt_renders_its_diff() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        pending(
            &mut app,
            false,
            "--- a/foo.rs\n+++ b/foo.rs\n@@ -1 +1 @@\n-old\n+new",
        );
        let screen = draw(&app, 100, 24);
        assert!(screen.contains("-old"));
        assert!(screen.contains("+new"));
    }

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
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        app.apply_event(emberly_core::UiEvent::AssistantDelta {
            text: "aaaa bbbb cccc".into(),
        });
        // Width 6 forces three rows.
        let mut reasoning_line = None;
        let lines = conversation_lines(&app, 6, &mut reasoning_line);
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn untrusted_web_results_labeled_in_rich_mode() {
        // Design §4.10: untrusted web content is labeled as fetched web data —
        // never harness or assistant voice. The WEB marker + "untrusted web
        // content" label makes a hostile snippet read visibly as quoted web text.
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        let id = emberly_core::ToolCallId::new("ws");
        app.apply_event(UiEvent::ToolStarted {
            call_id: id.clone(),
            tool: "web_search".into(),
            summary: "search: rust async".into(),
            explanation: None,
        });
        app.apply_event(UiEvent::ToolFinished {
            call_id: id,
            ok: true,
            summary: "searched: \"rust async\" (1 results)".into(),
            preview: "1. Tokio\n   https://tokio.rs\n   Async runtime".into(),
            untrusted: true,
        });
        let screen = draw(&app, 100, 24);
        assert!(
            screen.contains("untrusted web content"),
            "untrusted label visible in rich mode"
        );
        assert!(
            screen.contains("https://tokio.rs"),
            "source URL visible"
        );
    }

    #[test]
    fn trusted_tool_results_have_no_untrusted_label() {
        let mut app = App::new(
            SessionInfo::default(),
            std::env::temp_dir(),
            Vec::new(),
            String::new(),
        );
        let id = emberly_core::ToolCallId::new("c1");
        app.apply_event(UiEvent::ToolStarted {
            call_id: id.clone(),
            tool: "bash".into(),
            summary: "run: echo hi".into(),
            explanation: None,
        });
        app.apply_event(UiEvent::ToolFinished {
            call_id: id,
            ok: true,
            summary: "exit 0".into(),
            preview: "hi".into(),
            untrusted: false,
        });
        let screen = draw(&app, 100, 24);
        assert!(
            !screen.contains("untrusted web content"),
            "ordinary tool results have no untrusted label"
        );
    }
}
