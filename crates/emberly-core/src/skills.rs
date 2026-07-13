//! The skill catalog (FR-7, T-15, Tech Spec §8.2). Pure over injected dir
//! paths so it is unit-testable against temp dirs — mirroring `MemoryStore`.
//!
//! A skill is a subdirectory `<name>/` containing `SKILL.md` (TOML `+++`
//! frontmatter with `name`, `description` + markdown instruction body) and
//! optional bundled resources/scripts. Two scopes: user-global
//! (`~/.config/emberly/skills/`) and project (`.agents/skills/`, loaded only
//! under a trusted root). On a name collision **project overrides user-global**
//! (Tech Spec §8.2); the shadow is surfaced via `emberly config show`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use emberly_tools::skills::{SkillInvocation, SkillMeta, SkillOrigin};

use crate::memory::split_frontmatter;

/// The metadata stored in a skill's `SKILL.md` TOML frontmatter (Tech Spec §8.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
}

/// A shadowed skill: the user-global variant is overridden by a project skill
/// of the same name. Surfaced via `emberly config show` (Tech Spec §8.2, §16).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowNotice {
    pub name: String,
    pub shadowed_origin: SkillOrigin,
}

/// The skill catalog. `project_dir` is `None` on an untrusted root (structural
/// trust-gating, Tech Spec §6.7) — project skills are neither cataloged nor
/// invocable. Mirrors `MemoryStore`.
pub struct SkillCatalog {
    user_dir: PathBuf,
    project_dir: Option<PathBuf>,
}

/// The resolved location of a discovered skill: its folder path and origin.
struct ResolvedSkill {
    folder: PathBuf,
    origin: SkillOrigin,
}

impl SkillCatalog {
    /// Build a catalog over two scope directories. `project_dir` is `None` on
    /// an untrusted root so project-scope skills are structurally rejected.
    #[must_use]
    pub fn new(user_dir: PathBuf, project_dir: Option<PathBuf>) -> Self {
        Self {
            user_dir,
            project_dir,
        }
    }

    /// Discover all skills and build the catalog with precedence. Returns the
    /// catalog (`Vec<SkillMeta>`) and any shadow notices (Tech Spec §8.2).
    ///
    /// Scans user-global then project (project only if `project_dir.is_some()`);
    /// on a collision **project overrides user-global** and a `ShadowNotice`
    /// is recorded for `config show`. A folder without a readable `SKILL.md`
    /// or valid frontmatter is skipped (warn, not a crash).
    #[must_use]
    pub fn discover(&self) -> (Vec<SkillMeta>, Vec<ShadowNotice>) {
        let mut skills: Vec<SkillMeta> = Vec::new();
        let mut shadows: Vec<ShadowNotice> = Vec::new();

        // Scan user-global first.
        for meta in scan_dir(&self.user_dir, SkillOrigin::User) {
            skills.push(meta);
        }

        // Scan project (only if trusted).
        if let Some(project_dir) = &self.project_dir {
            for meta in scan_dir(project_dir, SkillOrigin::Project) {
                // Check for collision — project overrides user-global.
                if let Some(existing) = skills.iter().find(|s| s.name == meta.name) {
                    if existing.origin == SkillOrigin::User {
                        shadows.push(ShadowNotice {
                            name: meta.name.clone(),
                            shadowed_origin: SkillOrigin::User,
                        });
                    }
                }
                // Remove any existing user-global entry with the same name.
                skills.retain(|s| s.name != meta.name);
                skills.push(meta);
            }
        }

        // Stable sort by name.
        skills.sort_by_key(|s| s.name.to_lowercase());
        (skills, shadows)
    }

    /// Resolve a skill's folder by name. Returns the folder path and origin, or
    /// `None` when the skill is not in the catalog.
    fn resolve(&self, name: &str) -> Option<ResolvedSkill> {
        // Project takes precedence — check it first.
        if let Some(project_dir) = &self.project_dir {
            let folder = project_dir.join(name);
            if folder.join("SKILL.md").is_file() {
                return Some(ResolvedSkill {
                    folder,
                    origin: SkillOrigin::Project,
                });
            }
        }
        // Fall back to user-global.
        let folder = self.user_dir.join(name);
        if folder.join("SKILL.md").is_file() {
            return Some(ResolvedSkill {
                folder,
                origin: SkillOrigin::User,
            });
        }
        None
    }

    /// Invoke a skill: load its `SKILL.md` body (post-frontmatter) and list
    /// bundled resource paths (Tech Spec §8.2). Returns `None` when the skill
    /// is not in the catalog. Resource paths are absolute — the model passes
    /// them to `read_file`/`bash`, which root-check; project resources are
    /// inside the root (no prompt), user-global resources are outside-root
    /// (prompt under HC-4 — FR-7 honesty clause).
    #[must_use]
    pub fn invoke(&self, name: &str) -> Option<SkillInvocation> {
        let resolved = self.resolve(name)?;
        let skill_md = resolved.folder.join("SKILL.md");
        let text = std::fs::read_to_string(&skill_md).ok()?;
        let (_, body) = split_frontmatter(&text);

        // List bundled resources: every file in the folder except SKILL.md.
        let mut resources: Vec<String> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&resolved.folder) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.file_name() != Some(std::ffi::OsStr::new("SKILL.md")) {
                    if let Some(s) = path.to_str() {
                        resources.push(s.to_string());
                    }
                }
            }
        }
        resources.sort();

        Some(SkillInvocation {
            body: body.trim().to_string(),
            resources,
            origin: resolved.origin,
        })
    }
}

/// Scan a single scope directory for skill folders. Each subdirectory
/// containing a readable `SKILL.md` with valid frontmatter becomes a
/// `SkillMeta`. A folder without `SKILL.md` or valid frontmatter is skipped.
fn scan_dir(dir: &Path, origin: SkillOrigin) -> Vec<SkillMeta> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out, // Missing dir is normal — empty catalog.
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        let text = match std::fs::read_to_string(&skill_md) {
            Ok(t) => t,
            Err(_) => continue, // No SKILL.md — skip, not a crash.
        };
        let (front, _) = split_frontmatter(&text);
        let front = match front {
            Some(f) => f,
            None => continue, // No frontmatter — skip.
        };
        let manifest: SkillManifest = match toml::from_str(front) {
            Ok(m) => m,
            Err(_) => continue, // Invalid frontmatter — skip.
        };
        out.push(SkillMeta {
            name: manifest.name,
            description: manifest.description,
            origin,
        });
    }
    out
}

/// Render the skill catalog as a pinned context block (Tech Spec §7). One line
/// per skill: `- <name> — <description> (<origin>)`. Empty catalog → empty
/// string (no section).
#[must_use]
pub fn render_catalog(skills: &[SkillMeta]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("## Available skills\n\n");
    for skill in skills {
        let origin = match skill.origin {
            SkillOrigin::User => "user",
            SkillOrigin::Project => "project",
        };
        if skill.description.is_empty() {
            out.push_str(&format!("- {} ({})\n", skill.name, origin));
        } else {
            out.push_str(&format!("- {} — {} ({})\n", skill.name, skill.description, origin));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("emberly-skill-test-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        dir
    }

    fn write_skill(dir: &Path, name: &str, description: &str, body: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();
        let content = format!("+++\nname = \"{name}\"\ndescription = \"{description}\"\n+++\n{body}");
        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    fn write_resource(dir: &Path, skill_name: &str, resource_name: &str, content: &str) {
        let skill_dir = dir.join(skill_name);
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join(resource_name), content).unwrap();
    }

    #[test]
    fn discover_finds_skill_with_frontmatter() {
        let dir = temp_dir();
        write_skill(&dir, "pdf-fill", "Fill PDF forms", "Instructions here.");

        let catalog = SkillCatalog::new(dir.clone(), None);
        let (skills, shadows) = catalog.discover();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "pdf-fill");
        assert_eq!(skills[0].description, "Fill PDF forms");
        assert_eq!(skills[0].origin, SkillOrigin::User);
        assert!(shadows.is_empty());
    }

    #[test]
    fn discover_skips_folder_without_skill_md() {
        let dir = temp_dir();
        fs::create_dir_all(dir.join("not-a-skill")).unwrap();
        write_skill(&dir, "real-skill", "A real skill", "Body.");

        let catalog = SkillCatalog::new(dir.clone(), None);
        let (skills, _) = catalog.discover();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "real-skill");
    }

    #[test]
    fn discover_skips_skill_without_frontmatter() {
        let dir = temp_dir();
        let skill_dir = dir.join("no-frontmatter");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "Just a body, no frontmatter.").unwrap();

        let catalog = SkillCatalog::new(dir.clone(), None);
        let (skills, _) = catalog.discover();

        assert!(skills.is_empty());
    }

    #[test]
    fn discover_skips_skill_with_invalid_frontmatter() {
        let dir = temp_dir();
        let skill_dir = dir.join("bad-frontmatter");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "+++\nnot valid toml = = =\n+++\nBody.").unwrap();

        let catalog = SkillCatalog::new(dir.clone(), None);
        let (skills, _) = catalog.discover();

        assert!(skills.is_empty());
    }

    #[test]
    fn discover_tolerates_missing_dir() {
        let dir = temp_dir();
        let _ = fs::remove_dir_all(&dir);

        let catalog = SkillCatalog::new(dir, None);
        let (skills, shadows) = catalog.discover();

        assert!(skills.is_empty());
        assert!(shadows.is_empty());
    }

    #[test]
    fn project_overrides_user_on_collision() {
        let user_dir = temp_dir();
        let project_dir = temp_dir();

        write_skill(&user_dir, "shared", "User version", "User body.");
        write_skill(&project_dir, "shared", "Project version", "Project body.");

        let catalog = SkillCatalog::new(user_dir, Some(project_dir));
        let (skills, shadows) = catalog.discover();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "shared");
        assert_eq!(skills[0].description, "Project version");
        assert_eq!(skills[0].origin, SkillOrigin::Project);
        assert_eq!(shadows.len(), 1);
        assert_eq!(shadows[0].name, "shared");
        assert_eq!(shadows[0].shadowed_origin, SkillOrigin::User);
    }

    #[test]
    fn project_absent_when_untrusted() {
        let user_dir = temp_dir();
        let project_dir = temp_dir();

        write_skill(&user_dir, "user-skill", "User skill", "Body.");
        write_skill(&project_dir, "project-skill", "Project skill", "Body.");

        // project_dir = None simulates untrusted root.
        let catalog = SkillCatalog::new(user_dir, None);
        let (skills, shadows) = catalog.discover();

        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "user-skill");
        assert!(shadows.is_empty());
    }

    #[test]
    fn invoke_returns_body_and_resources() {
        let dir = temp_dir();
        write_skill(&dir, "pdf-fill", "Fill PDF forms", "Step 1: do the thing.");
        write_resource(&dir, "pdf-fill", "template.txt", "Template content");

        let catalog = SkillCatalog::new(dir.clone(), None);
        let invocation = catalog.invoke("pdf-fill");

        assert!(invocation.is_some());
        let inv = invocation.unwrap();
        assert_eq!(inv.body, "Step 1: do the thing.");
        assert_eq!(inv.origin, SkillOrigin::User);
        assert_eq!(inv.resources.len(), 1);
        // User-global resources are absolute paths.
        assert!(inv.resources[0].ends_with("template.txt"));
    }

    #[test]
    fn invoke_returns_none_for_missing_skill() {
        let dir = temp_dir();
        let catalog = SkillCatalog::new(dir, None);
        assert!(catalog.invoke("nonexistent").is_none());
    }

    #[test]
    fn invoke_resolves_project_precedence() {
        let user_dir = temp_dir();
        let project_dir = temp_dir();

        write_skill(&user_dir, "shared", "User version", "User body.");
        write_skill(&project_dir, "shared", "Project version", "Project body.");

        let catalog = SkillCatalog::new(user_dir, Some(project_dir.clone()));
        let inv = catalog.invoke("shared").unwrap();

        assert_eq!(inv.body, "Project body.");
        assert_eq!(inv.origin, SkillOrigin::Project);
    }

    #[test]
    fn invoke_project_skill_untrusted_returns_none() {
        let user_dir = temp_dir();
        let project_dir = temp_dir();

        write_skill(&project_dir, "project-only", "Project skill", "Body.");

        // project_dir = None — project skill is invocable only under trust.
        let catalog = SkillCatalog::new(user_dir, None);
        assert!(catalog.invoke("project-only").is_none());
    }

    #[test]
    fn render_catalog_empty_returns_empty_string() {
        assert_eq!(render_catalog(&[]), "");
    }

    #[test]
    fn render_catalog_formats_skills() {
        let skills = vec![
            SkillMeta {
                name: "pdf-fill".into(),
                description: "Fill PDF forms".into(),
                origin: SkillOrigin::User,
            },
            SkillMeta {
                name: "linter".into(),
                description: "".into(),
                origin: SkillOrigin::Project,
            },
        ];
        let rendered = render_catalog(&skills);
        assert!(rendered.contains("## Available skills"));
        assert!(rendered.contains("- pdf-fill — Fill PDF forms (user)"));
        assert!(rendered.contains("- linter (project)"));
    }
}
