//! The single theme definition (Design §2). Colours are addressed by **role**
//! (accent, chrome, success, …), never by literal value at the call site, so
//! tuning the palette — or, later, loading a different one — is a value change
//! here, not a refactor across the UI. This is the whole "theming deferred, not
//! designed out" story: a future theme is a different [`Palette`] behind the
//! same roles.
//!
//! Two disciplines from the guideline are encoded structurally:
//! - **The ember accent is scarce.** Only the wordmark, focus/active marks, the
//!   spinner, and key highlights use [`Theme::accent`]; if a screen is more
//!   than ~5% accent, something is wrong (Design §2).
//! - **Safety styling is reserved.** [`Theme::safety_band`] is used by the
//!   outside-project-root permission band (Design §5) and *nothing else*, ever.
//!   Do not reuse it decoratively.
//!
//! The rich TUI always has colour (a `NO_COLOR`/`TERM=dumb`/non-tty environment
//! is routed to the line-mode frontend instead — see `frontend::detect`), but
//! [`Theme::plain`] exists for tests and future headless reuse: with colour off
//! every role collapses to the terminal default, and meaning must be carried by
//! text and layout, never colour alone (Design §7).

use ratatui::style::{Color, Modifier, Style};

/// The named colour values. One instance per theme; roles read from it. A
/// different theme (e.g. the light-terminal fallback) is a different `Palette`
/// with the same field set — nothing else changes.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Warm near-black reference background (the terminal's own background wins
    /// where we do not draw; this guides drawn surfaces).
    pub background: Color,
    /// Raised surface for prompts and overlays.
    pub raised: Color,
    /// The one warm identity accent (ember amber/copper).
    pub accent: Color,
    /// Dimmer accent for focus borders and subtle marks.
    pub dim_accent: Color,
    /// High-contrast neutral for primary content.
    pub primary: Color,
    /// Dimmed warm gray for chrome, labels, dividers, hints, metadata.
    pub chrome: Color,
    /// Semantic success / allow (also diff additions).
    pub success: Color,
    /// Semantic error / deny (also diff deletions).
    pub error: Color,
    /// Semantic warning / caution.
    pub warning: Color,
    /// Foreground of the reserved outside-root safety band.
    pub safety_fg: Color,
    /// Background of the reserved outside-root safety band.
    pub safety_bg: Color,
}

impl Palette {
    /// The v1 built-in warm-dark palette (Design §2 candidate values). Tuned by
    /// eye against the real interface later; changing a value here is the tune.
    #[must_use]
    pub const fn dark() -> Self {
        Self {
            background: Color::Rgb(0x16, 0x12, 0x0F),
            raised: Color::Rgb(0x22, 0x1C, 0x17),
            accent: Color::Rgb(0xE8, 0x83, 0x3A),
            dim_accent: Color::Rgb(0x9C, 0x5A, 0x2B),
            primary: Color::Rgb(0xED, 0xE6, 0xDB),
            chrome: Color::Rgb(0x85, 0x7C, 0x6F),
            success: Color::Rgb(0x8F, 0xB5, 0x73),
            error: Color::Rgb(0xD9, 0x6C, 0x5F),
            warning: Color::Rgb(0xDB, 0xA9, 0x4D),
            // A loud, unmistakable band: bright warm text on deep red. Reserved
            // for outside-root prompts only (Design §5).
            safety_fg: Color::Rgb(0xF6, 0xE9, 0xE6),
            safety_bg: Color::Rgb(0x6E, 0x1F, 0x1A),
        }
    }

    /// Light-terminal legibility fallback (Design §2). Same roles, values chosen
    /// to read on a light background: dark primary/chrome, the accent kept warm.
    #[must_use]
    pub const fn light() -> Self {
        Self {
            background: Color::Rgb(0xF7, 0xF2, 0xEA),
            raised: Color::Rgb(0xEC, 0xE3, 0xD6),
            accent: Color::Rgb(0xB5, 0x5E, 0x1E),
            dim_accent: Color::Rgb(0x8A, 0x4E, 0x22),
            primary: Color::Rgb(0x2A, 0x24, 0x1E),
            chrome: Color::Rgb(0x6B, 0x62, 0x55),
            success: Color::Rgb(0x4C, 0x7A, 0x2E),
            error: Color::Rgb(0xA8, 0x3A, 0x2E),
            warning: Color::Rgb(0x9A, 0x6E, 0x18),
            safety_fg: Color::Rgb(0xFB, 0xF3, 0xF1),
            safety_bg: Color::Rgb(0x8A, 0x24, 0x1E),
        }
    }
}

/// A theme: a palette plus whether colour is applied. Roles are `Style`
/// builders; with colour off they return the terminal default so text/layout
/// carry meaning alone (Design §7).
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    palette: Palette,
    color: bool,
}

impl Theme {
    /// The rich warm-dark theme with colour.
    #[must_use]
    pub const fn rich() -> Self {
        Self {
            palette: Palette::dark(),
            color: true,
        }
    }

    /// The light-terminal fallback with colour.
    #[must_use]
    pub const fn light() -> Self {
        Self {
            palette: Palette::light(),
            color: true,
        }
    }

    /// A colourless theme: every role is the terminal default. For tests and
    /// any future headless/degraded reuse.
    #[must_use]
    pub const fn plain() -> Self {
        Self {
            palette: Palette::dark(),
            color: false,
        }
    }

    /// The underlying palette, for callers needing a raw [`Color`].
    #[must_use]
    pub const fn palette(&self) -> Palette {
        self.palette
    }

    /// Whether colour is applied (false under [`Theme::plain`]). Callers that
    /// map external colours (syntax highlighting) check this to stay colourless
    /// in degraded/test contexts.
    #[must_use]
    pub const fn has_color(&self) -> bool {
        self.color
    }

    fn fg(&self, color: Color) -> Style {
        if self.color {
            Style::default().fg(color)
        } else {
            Style::default()
        }
    }

    /// Primary content text.
    #[must_use]
    pub fn primary(&self) -> Style {
        self.fg(self.palette.primary)
    }

    /// Chrome: labels, dividers, hints, metadata — dimmed.
    #[must_use]
    pub fn chrome(&self) -> Style {
        self.fg(self.palette.chrome).add_modifier(Modifier::DIM)
    }

    /// Bold primary text — markdown **strong** and headings (hierarchy through
    /// weight, not colour; Design §2, §4.1).
    #[must_use]
    pub fn strong(&self) -> Style {
        self.fg(self.palette.primary).add_modifier(Modifier::BOLD)
    }

    /// Inline `code`: primary text on the raised surface, a subtle chip that
    /// reads as code without competing with the accent (Design §4.1).
    #[must_use]
    pub fn code_inline(&self) -> Style {
        if self.color {
            Style::default()
                .fg(self.palette.primary)
                .bg(self.palette.raised)
        } else {
            Style::default()
        }
    }

    /// The scarce ember accent (wordmark, focus, spinner, key highlights).
    #[must_use]
    pub fn accent(&self) -> Style {
        self.fg(self.palette.accent).add_modifier(Modifier::BOLD)
    }

    /// Dimmer accent for focus borders and subtle marks.
    #[must_use]
    pub fn dim_accent(&self) -> Style {
        self.fg(self.palette.dim_accent)
    }

    /// Success / allow.
    #[must_use]
    pub fn success(&self) -> Style {
        self.fg(self.palette.success)
    }

    /// Error / deny.
    #[must_use]
    pub fn error(&self) -> Style {
        self.fg(self.palette.error)
    }

    /// Warning / caution.
    #[must_use]
    pub fn warning(&self) -> Style {
        self.fg(self.palette.warning)
    }

    /// Diff additions (green, universal convention).
    #[must_use]
    pub fn diff_add(&self) -> Style {
        self.fg(self.palette.success)
    }

    /// Diff deletions (red, universal convention).
    #[must_use]
    pub fn diff_del(&self) -> Style {
        self.fg(self.palette.error)
    }

    /// **Reserved.** The outside-project-root permission band (Design §5) and
    /// nothing else. A loud fg-on-bg band, bold, that cannot be mistaken for a
    /// routine prompt at a glance. Do not reuse decoratively.
    #[must_use]
    pub fn safety_band(&self) -> Style {
        if self.color {
            Style::default()
                .fg(self.palette.safety_fg)
                .bg(self.palette.safety_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            // Without colour the band's meaning is carried by the capitalised
            // banner text (Design §7); keep bold for weight where supported.
            Style::default().add_modifier(Modifier::BOLD)
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::rich()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_theme_applies_no_colour() {
        let t = Theme::plain();
        // No foreground/background colour on any ordinary role.
        assert_eq!(t.primary().fg, None);
        assert_eq!(t.accent().fg, None);
        assert_eq!(t.error().fg, None);
        assert_eq!(t.safety_band().fg, None);
        assert_eq!(t.safety_band().bg, None);
    }

    #[test]
    fn rich_theme_colours_roles() {
        let t = Theme::rich();
        assert_eq!(t.accent().fg, Some(Palette::dark().accent));
        assert_eq!(t.diff_add().fg, Some(Palette::dark().success));
        assert_eq!(t.diff_del().fg, Some(Palette::dark().error));
    }

    #[test]
    fn safety_band_is_a_distinct_bg_band_in_colour() {
        let t = Theme::rich();
        let band = t.safety_band();
        assert_eq!(band.bg, Some(Palette::dark().safety_bg));
        assert!(band.add_modifier.contains(Modifier::BOLD));
    }
}
