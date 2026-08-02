//! The skills inspector (FR-7, T-15).
//!
//! Part of the `App` inherent impl, split out of one 2,300-line block.

use super::*;

impl App {
    /// `/skills` — open the skills inspector. The catalog is already cached
    /// (`SkillsAvailable`), so the list overlay opens immediately with no engine
    /// round-trip; only a selected skill's *body* is fetched on demand (§8.6).
    pub(super) fn open_skills_inspector(&mut self) -> Action {
        self.push_overlay(Overlay {
            title: crate::strings::skills::TITLE.into(),
            content: OverlayContent::SkillList {
                skills: self.skills.clone(),
                selected: 0,
            },
            scroll: 0,
        });
        Action::None
    }

    /// Apply a `SkillBody` reply: open the instruction body **read-only** on top
    /// of the list (§4.9 — inspectable before it ever runs). Bundled resource
    /// paths are appended so "what the skill bundles" is visible too. Fetching
    /// the body for display runs no bundled script (FR-7).
    pub(super) fn apply_skill_body(
        &mut self,
        name: &str,
        origin: SkillOrigin,
        body: String,
        resources: &[String],
    ) {
        let title = format!("{name} · {}", skill_origin_label(origin));
        let mut text = if body.trim().is_empty() {
            crate::strings::skills::EMPTY_BODY.to_string()
        } else {
            body
        };
        if !resources.is_empty() {
            text.push_str("\n\n");
            text.push_str(crate::strings::skills::RESOURCES_HEADER);
            text.push('\n');
            for r in resources {
                text.push_str(&format!("- {r}\n"));
            }
        }
        self.open_text_overlay(title, text);
    }

    pub(super) fn set_skill_selection(&mut self, next: usize) {
        if let Some(Overlay {
            content: OverlayContent::SkillList { selected, .. },
            ..
        }) = self.overlays.last_mut()
        {
            *selected = next;
        }
    }

    /// Keys for the skills inspector (FR-7, Design §4.9): ↑/↓ move, Enter fetches
    /// and shows the selected skill's body read-only, Esc/q dismiss. There is no
    /// edit or delete — skills are externally-authored folders (read-only here).
    pub(super) fn on_skills_inspector_key(&mut self, key: KeyEvent) -> Action {
        let (len, selected, chosen) = match self.overlays.last().map(|o| &o.content) {
            Some(OverlayContent::SkillList { skills, selected }) => {
                (skills.len(), *selected, skills.get(*selected).cloned())
            }
            _ => return Action::None,
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlays.pop();
                Action::None
            }
            KeyCode::Up => {
                self.set_skill_selection(selected.saturating_sub(1));
                Action::None
            }
            KeyCode::Down => {
                self.set_skill_selection((selected + 1).min(len.saturating_sub(1)));
                Action::None
            }
            KeyCode::Enter => match chosen {
                Some(skill) => Action::Command(Command::InspectSkill { name: skill.name }),
                None => Action::None,
            },
            _ => Action::None,
        }
    }
}
