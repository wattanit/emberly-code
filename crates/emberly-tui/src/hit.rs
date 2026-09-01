//! Click hit-testing (Design §3.4). The layout is otherwise transient — it is
//! computed inside the draw call from `Layout::split` and no `Rect`s survive the
//! frame — so a click `(column, row)` has nothing to resolve against on its own.
//!
//! This module retains a [`HitMap`]: a per-frame list of `(Rect, ClickTarget)`
//! rebuilt by [`crate::render::frame`] from the *real* render geometry (one
//! source of truth, so the hit regions can never drift from what is drawn).
//! [`crate::app::App::on_click`] resolves a click against it.
//!
//! Every [`ClickTarget`] maps to an action the keyboard can already perform —
//! the mouse adds no new authority (the §3.4 invariant). Dispatch lives in
//! `App::on_click`, which reuses the same handlers the keys do.

use ratatui::layout::Rect;

/// A clickable target resolved from a screen position (Design §3.4). Each
/// variant has a keyboard twin; the click is a shortcut for "focus + Enter" on
/// it, never a capability the keyboard lacks. A new clickable surface adds a
/// variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickTarget {
    /// A command-palette row — index into the *filtered* match list (what
    /// `PaletteState::selected` indexes). Click = select it + palette Enter.
    PaletteRow(usize),
    /// A choice-picker row (model/effort/mode) — index into the picker's rows.
    /// Click = select it + picker Enter.
    ChoiceRow(usize),
    /// A session-picker row — index into the sessions list. Click = select it +
    /// picker Enter (resume).
    SessionRow(usize),
    /// A memory-inspector row — index into the flattened user-then-project
    /// entry list. Click = select it + inspector Enter (view).
    MemoryRow(usize),
    /// A skills-inspector row — index into the skills list. Click = select it +
    /// inspector Enter (read-only body view).
    SkillRow(usize),
    /// An Agents-inspector row — index into the alive-subagents list. Click =
    /// select it + inspector Enter (read-only activity view).
    AgentRow(usize),
    /// An MCP-inspector row — index into the connected-servers list. Click =
    /// select it + inspector Enter (read-only tool-list view).
    McpServerRow(usize),
    /// The collapsed/expanded reasoning-trail line in the conversation. Click =
    /// toggle it, exactly as Ctrl+R does (`toggle_reasoning`).
    ReasoningToggle,
    /// The sidebar's modified-files list. Click = open the diff overlay, exactly
    /// as Ctrl+O does (`open_last_diff`) — parity-safe (per-file open has no
    /// keyboard twin, so any click opens the same view the key does).
    OpenDiff,
    /// The sidebar's Memory section. Click = open the memory inspector, exactly
    /// as `/memory` does (palette-reachable, §3.3).
    OpenMemoryInspector,
    /// The sidebar's Skills section. Click = open the skills inspector, exactly
    /// as `/skills` does.
    OpenSkillsInspector,
    /// The sidebar's Agents section. Click = open the Agents inspector, exactly
    /// as `/agents` does.
    OpenAgentsInspector,
    /// The sidebar's MCP section. Click = open the MCP inspector, exactly as
    /// `/mcp` does.
    OpenMcpInspector,
    /// A permission-prompt affordance (Design §5). A click here dispatches the
    /// **same** decision the matching key does, via `on_permission_key` — never
    /// a new path. Only the affordance text is registered (the gaps between them
    /// are inert), and a click elsewhere on the prompt is inert: no click
    /// "approves whatever is focused," no hover-to-approve, no click-through.
    PermissionChoice(PermissionChoice),
}

/// The three permission-prompt affordances (Design §5), in footer order. Each
/// maps to the exact key `on_permission_key` handles: Allow→`y`, Session→`s`,
/// Deny→Enter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionChoice {
    /// Allow once (the `y` key).
    Allow,
    /// Allow for this session (the `s` key).
    Session,
    /// Deny — the safe default (Enter/Esc/`d`/`n`).
    Deny,
}

/// A per-frame map from screen rectangles to click targets, rebuilt every draw
/// from the live render geometry (Design §3.4). Regions are pushed in
/// back-to-front (draw) order; [`hit`](Self::hit) scans back-to-front so the
/// **topmost** region wins, honoring the same modal priority as `on_key`
/// (palette > overlay > permission > sidebar/conversation). Each modal renderer
/// [`clear`](Self::clear)s the map before pushing its own regions, so the map
/// always reflects the topmost interactive layer and a click can never fall
/// through a modal to the pane behind it.
#[derive(Debug, Default, Clone)]
pub struct HitMap {
    regions: Vec<(Rect, ClickTarget)>,
}

impl HitMap {
    /// An empty map (a fresh one is built each frame).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Drop all regions. A modal renderer calls this before pushing its own, so
    /// the topmost layer owns the map and clicks never fall through it.
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    /// Register a clickable region. Later pushes sit "on top" of earlier ones.
    pub fn push(&mut self, area: Rect, target: ClickTarget) {
        self.regions.push((area, target));
    }

    /// Resolve a screen position to the topmost target whose rectangle contains
    /// it, or `None` if the click landed on nothing interactive.
    #[must_use]
    pub fn hit(&self, col: u16, row: u16) -> Option<ClickTarget> {
        self.regions
            .iter()
            .rev()
            .find(|(r, _)| contains(*r, col, row))
            .map(|(_, t)| *t)
    }
}

/// Whether `(col, row)` falls inside `r`. First-party (not `Rect::contains`) so
/// the containment rule is explicit and version-independent.
fn contains(r: Rect, col: u16, row: u16) -> bool {
    col >= r.x
        && col < r.x.saturating_add(r.width)
        && row >= r.y
        && row < r.y.saturating_add(r.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect {
            x,
            y,
            width: w,
            height: h,
        }
    }

    #[test]
    fn hit_resolves_a_contained_point() {
        let mut m = HitMap::new();
        m.push(rect(0, 5, 10, 1), ClickTarget::PaletteRow(3));
        assert_eq!(m.hit(4, 5), Some(ClickTarget::PaletteRow(3)));
        // Boundaries: the rect is half-open on the far edges.
        assert_eq!(m.hit(0, 5), Some(ClickTarget::PaletteRow(3))); // top-left included
        assert_eq!(m.hit(10, 5), None); // x == x+width excluded
        assert_eq!(m.hit(9, 6), None); // y == y+height excluded
    }

    #[test]
    fn hit_returns_none_off_every_region() {
        let mut m = HitMap::new();
        m.push(rect(2, 2, 4, 1), ClickTarget::ChoiceRow(0));
        assert_eq!(m.hit(0, 0), None);
        assert_eq!(m.hit(6, 2), None);
    }

    #[test]
    fn topmost_region_wins() {
        let mut m = HitMap::new();
        // Two overlapping regions; the later push is "on top".
        m.push(rect(0, 0, 20, 20), ClickTarget::ChoiceRow(9));
        m.push(rect(5, 5, 2, 2), ClickTarget::PaletteRow(1));
        assert_eq!(m.hit(5, 5), Some(ClickTarget::PaletteRow(1)));
        assert_eq!(m.hit(1, 1), Some(ClickTarget::ChoiceRow(9)));
    }

    #[test]
    fn clear_drops_regions() {
        let mut m = HitMap::new();
        m.push(rect(0, 0, 4, 4), ClickTarget::PaletteRow(0));
        m.clear();
        assert_eq!(m.hit(1, 1), None);
    }
}
