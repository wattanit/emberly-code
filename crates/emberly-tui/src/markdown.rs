//! A deliberately small, first-party markdown pass for assistant output
//! (Design §4.1). Full markdown parsers are not used for the chat flow — we
//! render a boring subset well and pass everything else through as plain,
//! unmangled text:
//!
//! - Fenced code blocks (```) with `syntect` syntax highlighting (fancy-regex
//!   backend, HC-2). **Only closed fences are highlighted**; an unclosed fence
//!   still streaming renders as plain text (Design §4.1 streaming note).
//! - **Bold**, `inline code`, `- `/`* `/`1. ` lists, and `#` headings rendered
//!   as bold with spacing. Tables, links, images, and nested exotica pass
//!   through verbatim.
//!
//! Output is a `Vec<Line<'static>>` already wrapped to the pane width via the
//! grapheme-correct [`crate::text`] engine, so each line is exactly one display
//! row (the conversation's scroll math depends on that). Highlighting maps
//! `syntect` colours onto ratatui, keeping our own background — code stays in
//! the neutral range and never competes with the ember accent.

use std::sync::OnceLock;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme as SynTheme, ThemeSet};
use syntect::parsing::SyntaxSet;

use crate::text;
use crate::theme::Theme;

/// Render an assistant message to styled, pre-wrapped display rows.
#[must_use]
pub fn render_message(text: &str, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let w = width.max(1);
    let mut out: Vec<Line<'static>> = Vec::new();

    // Fenced code is handled by scanning for ``` toggles; a block is only
    // highlighted once its closing fence is seen.
    let mut fence: Option<FenceState> = None;

    for raw in text.split('\n') {
        let trimmed = raw.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            match fence.take() {
                None => {
                    // Opening fence: remember the language token (if any).
                    fence = Some(FenceState {
                        lang: rest.trim().to_string(),
                        lines: Vec::new(),
                    });
                }
                Some(state) => {
                    // Closing fence: highlight the collected block.
                    out.extend(highlight_block(&state.lang, &state.lines, theme, w));
                }
            }
            continue;
        }

        if let Some(state) = fence.as_mut() {
            state.lines.push(raw.to_string());
            continue;
        }

        render_text_line(raw, theme, w, &mut out);
    }

    // An unclosed (still-streaming) fence renders as plain text, not highlighted.
    if let Some(state) = fence {
        for line in &state.lines {
            for row in text::wrap(line, w) {
                out.push(Line::from(Span::styled(row, theme.primary())));
            }
        }
    }

    out
}

struct FenceState {
    lang: String,
    lines: Vec<String>,
}

/// Render one non-code line: heading, list item, blank, or paragraph.
fn render_text_line(raw: &str, theme: &Theme, width: usize, out: &mut Vec<Line<'static>>) {
    if raw.trim().is_empty() {
        out.push(Line::from(""));
        return;
    }

    // Heading: `#`..`######` + space → bold text with a trailing blank line.
    if let Some((_level, rest)) = heading(raw) {
        for line in wrap_spans(&[(rest.to_string(), theme.strong())], width) {
            out.push(line);
        }
        return;
    }

    // List item: bullet (`-`/`*`/`+`) or ordered (`1.`).
    if let Some((marker, rest)) = list_item(raw) {
        let mut segs = vec![(marker, theme.chrome())];
        segs.extend(inline(rest, theme));
        out.extend(wrap_spans(&segs, width));
        return;
    }

    out.extend(wrap_spans(&inline(raw, theme), width));
}

/// Match a heading, returning (level, text) — `## Foo` → `(2, "Foo")`.
fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.bytes().take_while(|&b| b == b'#').count();
    if (1..=6).contains(&hashes) {
        let rest = &line[hashes..];
        if let Some(text) = rest.strip_prefix(' ') {
            return Some((hashes, text.trim_end()));
        }
    }
    None
}

/// Match a list item, returning (display marker, remaining text).
fn list_item(line: &str) -> Option<(String, &str)> {
    let trimmed = line.trim_start();
    for lead in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(lead) {
            return Some(("• ".to_string(), rest));
        }
    }
    // Ordered: digits then ". ".
    let digits = trimmed.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        if let Some(rest) = trimmed[digits..].strip_prefix(". ") {
            return Some((format!("{}. ", &trimmed[..digits]), rest));
        }
    }
    None
}

/// Parse inline **bold** and `code` into styled segments. Inside `code`, bold
/// markers are literal. Unmatched markers pass through as text.
fn inline(line: &str, theme: &Theme) -> Vec<(String, Style)> {
    let mut segs: Vec<(String, Style)> = Vec::new();
    let mut cur = String::new();
    let mut bold = false;
    let mut code = false;
    let bytes = line.as_bytes();
    let mut i = 0;

    let flush = |cur: &mut String, segs: &mut Vec<(String, Style)>, bold: bool, code: bool| {
        if !cur.is_empty() {
            let style = if code {
                theme.code_inline()
            } else if bold {
                theme.strong()
            } else {
                theme.primary()
            };
            segs.push((std::mem::take(cur), style));
        }
    };

    while i < bytes.len() {
        if !code && bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            flush(&mut cur, &mut segs, bold, code);
            bold = !bold;
            i += 2;
            continue;
        }
        if bytes[i] == b'`' {
            flush(&mut cur, &mut segs, bold, code);
            code = !code;
            i += 1;
            continue;
        }
        // Copy one whole UTF-8 char (indices land on char boundaries because
        // the matched markers are ASCII).
        let ch_len = utf8_len(bytes[i]);
        cur.push_str(&line[i..i + ch_len]);
        i += ch_len;
    }
    flush(&mut cur, &mut segs, bold, code);
    segs
}

fn utf8_len(first: u8) -> usize {
    match first {
        b if b < 0x80 => 1,
        b if b >> 5 == 0b110 => 2,
        b if b >> 4 == 0b1110 => 3,
        _ => 4,
    }
}

/// Word-aware wrap of styled segments to `max` columns, preserving each run's
/// style across line breaks (Thai-/CJK-correct via [`crate::text`]).
fn wrap_spans(segs: &[(String, Style)], max: usize) -> Vec<Line<'static>> {
    // Flatten to (grapheme, style) so wrapping never splits a cluster and each
    // grapheme keeps its style.
    let mut cells: Vec<(&str, Style)> = Vec::new();
    for (s, style) in segs {
        for g in text::clusters(s) {
            cells.push((g, *style));
        }
    }
    if cells.is_empty() {
        return vec![Line::from("")];
    }

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut cur: Vec<(&str, Style)> = Vec::new();
    let mut cur_w = 0usize;
    let mut last_space: Option<usize> = None;

    for (g, style) in cells {
        let gw = text::width(g);
        if cur_w + gw > max && !cur.is_empty() {
            match last_space {
                Some(sp) => {
                    let tail = cur.split_off(sp);
                    trim_trailing_spaces(&mut cur);
                    lines.push(coalesce(&cur));
                    cur = tail;
                    trim_leading_spaces(&mut cur);
                    cur_w = cur.iter().map(|(g, _)| text::width(g)).sum();
                    last_space = None;
                }
                None => {
                    lines.push(coalesce(&cur));
                    cur.clear();
                    cur_w = 0;
                }
            }
        }
        if g == " " {
            last_space = Some(cur.len());
        }
        cur.push((g, style));
        cur_w += gw;
    }
    lines.push(coalesce(&cur));
    lines
}

fn trim_trailing_spaces(cells: &mut Vec<(&str, Style)>) {
    while matches!(cells.last(), Some((g, _)) if *g == " ") {
        cells.pop();
    }
}

fn trim_leading_spaces(cells: &mut Vec<(&str, Style)>) {
    let mut n = 0;
    while matches!(cells.get(n), Some((g, _)) if *g == " ") {
        n += 1;
    }
    cells.drain(0..n);
}

/// Merge consecutive equally-styled graphemes into `Span`s → one `Line`.
fn coalesce(cells: &[(&str, Style)]) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut buf = String::new();
    let mut cur_style: Option<Style> = None;
    for (g, style) in cells {
        if cur_style != Some(*style) {
            if let Some(s) = cur_style.take() {
                spans.push(Span::styled(std::mem::take(&mut buf), s));
            }
            cur_style = Some(*style);
        }
        buf.push_str(g);
    }
    if let Some(s) = cur_style {
        spans.push(Span::styled(buf, s));
    }
    Line::from(spans)
}

// ---- syntax highlighting -------------------------------------------------

/// Bundled syntaxes + a neutral theme, loaded once. `load_defaults_newlines`
/// and the theme dumps are pure-Rust (HC-2); the fancy-regex engine is selected
/// by the `syntect` feature set (see `Cargo.toml`).
fn assets() -> &'static (SyntaxSet, SynTheme) {
    static ASSETS: OnceLock<(SyntaxSet, SynTheme)> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let themes = ThemeSet::load_defaults();
        // A muted base16 theme: blues/greens/greys, no orange to clash with the
        // ember accent. Fall back to the first theme if absent.
        let theme = themes
            .themes
            .get("base16-ocean.dark")
            .or_else(|| themes.themes.values().next())
            .cloned()
            .unwrap_or_default();
        (syntaxes, theme)
    })
}

/// Highlight a closed code block. Falls back to plain rendering when the
/// language is unknown or highlighting errors.
fn highlight_block(lang: &str, lines: &[String], ui: &Theme, width: usize) -> Vec<Line<'static>> {
    let (syntaxes, syn_theme) = assets();
    let syntax = (!lang.is_empty())
        .then(|| {
            syntaxes
                .find_syntax_by_token(lang)
                .or_else(|| syntaxes.find_syntax_by_extension(lang))
        })
        .flatten()
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());

    let mut highlighter = HighlightLines::new(syntax, syn_theme);
    let mut out = Vec::new();
    for line in lines {
        match highlighter.highlight_line(line, syntaxes) {
            Ok(ranges) if ui.has_color() => {
                let spans: Vec<Span<'static>> = ranges
                    .iter()
                    .map(|(style, piece)| {
                        Span::styled(
                            piece.trim_end_matches('\n').to_string(),
                            Style::default().fg(syntect_color(style.foreground)),
                        )
                    })
                    .collect();
                out.push(Line::from(spans));
            }
            // No colour (plain theme) or highlight error: render the source line
            // as-is. Code lines are not wrapped — a long line clips at the pane
            // edge rather than reflowing (reflowed code is unreadable).
            _ => out.push(Line::from(Span::styled(
                line.trim_end_matches('\n').to_string(),
                ui.primary(),
            ))),
        }
    }
    let _ = width; // code lines are not width-wrapped
    out
}

fn syntect_color(c: syntect::highlighting::Color) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain() -> Theme {
        // Colourless theme keeps assertions about text content simple.
        Theme::plain()
    }

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn bold_splits_into_segments() {
        let segs = inline("a **b** c", &plain());
        let texts: Vec<&str> = segs.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(texts, vec!["a ", "b", " c"]);
    }

    #[test]
    fn inline_code_is_literal() {
        // Bold markers inside code are not interpreted.
        let segs = inline("`**x**`", &plain());
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0].0, "**x**");
    }

    #[test]
    fn heading_becomes_bold_text() {
        let out = render_message("## Title", &plain(), 40);
        assert_eq!(out.len(), 1);
        assert_eq!(line_text(&out[0]), "Title");
    }

    #[test]
    fn list_item_gets_bullet() {
        let out = render_message("- item", &plain(), 40);
        assert_eq!(line_text(&out[0]), "• item");
    }

    #[test]
    fn plain_prose_passes_through() {
        let out = render_message("just text", &plain(), 40);
        assert_eq!(line_text(&out[0]), "just text");
    }

    #[test]
    fn wrapping_preserves_content_across_styles() {
        // "**hello** world foo" wrapped narrow: content is intact, split by word.
        let out = render_message("**hello** world foo", &plain(), 6);
        let joined: String = out.iter().map(line_text).collect::<Vec<_>>().join("|");
        assert_eq!(joined, "hello|world|foo");
    }

    #[test]
    fn closed_code_fence_highlights_each_line() {
        let src = "```rust\nfn main() {}\nlet x = 1;\n```";
        // Rich theme so highlighting runs; assert content is preserved.
        let out = render_message(src, &Theme::rich(), 80);
        assert_eq!(out.len(), 2, "two code lines, fences dropped");
        assert_eq!(line_text(&out[0]), "fn main() {}");
        assert_eq!(line_text(&out[1]), "let x = 1;");
    }

    #[test]
    fn open_fence_renders_plain_while_streaming() {
        // No closing fence yet: lines render, not dropped, not highlighted.
        let src = "```rust\nfn main() {}";
        let out = render_message(src, &Theme::rich(), 80);
        assert_eq!(out.len(), 1);
        assert_eq!(line_text(&out[0]), "fn main() {}");
    }
}
