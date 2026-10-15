//! Session instructions: per-project text that steers the live notes, and the
//! templates that fill it. One JSON file next to settings.json.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::store::{self, Loaded};
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Template {
    pub id: String,
    pub name: String,
    pub text: String,
    /// The sections to switch the shown tab to when applied (catalogue ids,
    /// catalogue order). Empty: leave the tab's sections as they are, as every
    /// template from before this field existed does.
    #[serde(default)]
    pub sections: Vec<String>,
}

/// One project's instructions. `current` steers the notes; `default` is what
/// "Reset to default" restores. Both start empty, so notes behave as before.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectInstructions {
    pub default: String,
    pub current: String,
    /// The template applied last, for the top-bar label.
    pub template_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct InstructionsFile {
    pub templates: Vec<Template>,
    /// Keyed by project root; "" is the session with no project open.
    pub projects: BTreeMap<String, ProjectInstructions>,
}

impl Default for InstructionsFile {
    fn default() -> Self {
        Self { templates: starters(), projects: BTreeMap::new() }
    }
}

/// What the UI loads for one project.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InstructionsView {
    pub templates: Vec<Template>,
    pub project: ProjectInstructions,
}

/// The templates a new install starts with, and what "Restore starter
/// templates" brings back. Each steers within its sections and the rules.
const STARTERS: [(&str, &str, &[&str], &str); 5] = [
    (
        "feature-spec",
        "Feature spec",
        &["intent", "conflicts", "gaps", "ambiguities", "questions"],
        "I'm writing a spec for a new feature that an AI agent will implement.
- Conflicts: features or code that already do part of this, integration points the spec skips, and data or state the project doesn't have yet.
- Gaps: acceptance criteria an agent could turn into tests, and non-goals.
- Questions: who this is for and what done looks like.",
    ),
    (
        "bugfix-spec",
        "Bug fix spec",
        &["intent", "conflicts", "gaps", "questions"],
        "This spec describes a bug and its fix. Treat the described cause as a hypothesis.
- Conflicts: code paths that contradict the described cause or the described current behaviour.
- Gaps: current behaviour, expected behaviour, what must not change, and the test that proves the fix.
- Questions: the evidence that would confirm or rule out the cause.",
    ),
    (
        "refactor-spec",
        "Refactor spec",
        &["conflicts", "gaps", "ambiguities", "questions"],
        "This spec describes a refactor or an architectural change.
- Conflicts: callers, contracts and invariants the change would break, including anything the project's docs say to preserve.
- Gaps: an order of steps that keeps the code working after each one, and how we'd know nothing regressed.
- Questions: what must not change.",
    ),
    (
        "handoff",
        "Hand-off pass",
        &["gaps", "ambiguities"],
        "This spec is nearly ready to hand to an agent. Hunt for anything it would have to guess: vague verbs, unnamed files, \"etc.\", implied defaults, missing error cases. Flag what it needs to start without asking.",
    ),
    (
        "devils-advocate",
        "Devil's advocate",
        &["conflicts", "questions"],
        "Be a skeptical reviewer.
- Conflicts: give the strongest objection to the plan rather than a list of small issues.
- Questions: ask what evidence would change my mind.",
    ),
];

pub fn starters() -> Vec<Template> {
    STARTERS
        .iter()
        .map(|(id, name, sections, text)| Template {
            id: (*id).into(),
            name: (*name).into(),
            text: (*text).into(),
            sections: sections.iter().map(|s| (*s).to_owned()).collect(),
        })
        .collect()
}

pub struct InstructionStore {
    path: PathBuf,
    /// Serializes load-modify-write, so concurrent saves never lose an update.
    lock: Mutex<()>,
}

/// What a load found, as the file to work from. A file that didn't parse and
/// couldn't be set aside as `.bad` is an error: the next save would overwrite it.
fn start_from(loaded: Loaded<InstructionsFile>) -> Result<InstructionsFile> {
    match loaded {
        Loaded::Parsed(file) => Ok(file),
        Loaded::Missing | Loaded::Corrupt { kept: Some(_) } => Ok(InstructionsFile::default()),
        Loaded::Corrupt { kept: None } => Err(Error::Precondition(
            "Couldn't read your saved instructions and couldn't keep a copy; free up instructions.json and try again."
                .into(),
        )),
    }
}

impl InstructionStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path, lock: Mutex::new(()) }
    }

    /// Never fails: a missing file gives the starters, and an unreadable one is
    /// kept as `.bad` first. Instructions must never block opening a project.
    pub fn load(&self) -> InstructionsFile {
        let _guard = self.lock.lock().expect("instructions lock");
        // A file that can't be read right now (locked, I/O error) is left exactly
        // where it is; the UI just shows the starters until the next load.
        self.read().unwrap_or_default()
    }

    pub fn view(&self, root: &str) -> InstructionsView {
        let file = self.load();
        InstructionsView { project: file.projects.get(root).cloned().unwrap_or_default(), templates: file.templates }
    }

    /// Stores one project's instructions; an entry with nothing set is removed.
    pub fn save_project(&self, root: &str, p: ProjectInstructions) -> Result<()> {
        self.update(|f| {
            if p == ProjectInstructions::default() {
                f.projects.remove(root);
            } else {
                f.projects.insert(root.to_owned(), p);
            }
        })
    }

    pub fn save_templates(&self, templates: Vec<Template>) -> Result<()> {
        self.update(|f| f.templates = templates)
    }

    /// Re-adds each starter whose id is missing, in starter order, at the end;
    /// never overwrites an edited one. Returns the merged list.
    pub fn restore_starters(&self) -> Result<Vec<Template>> {
        let mut merged = Vec::new();
        self.update(|f| {
            for s in starters() {
                if !f.templates.iter().any(|t| t.id == s.id) {
                    f.templates.push(s);
                }
            }
            merged = f.templates.clone();
        })?;
        Ok(merged)
    }

    fn update(&self, change: impl FnOnce(&mut InstructionsFile)) -> Result<()> {
        let _guard = self.lock.lock().expect("instructions lock");
        let mut file = self.read()?; // never write over a file we couldn't read
        change(&mut file);
        self.write(&file)
    }

    fn read(&self) -> Result<InstructionsFile> {
        // A file that doesn't parse is kept as `.bad`, `.bad.1`, … instead of
        // being overwritten by the next save (settings.json just resets; this
        // holds the user's writing). A file that can't be read right now is an
        // error, so `update` never writes over it.
        start_from(store::load_json(&self.path)?)
    }

    /// Atomic write: never leaves a half-written file behind.
    fn write(&self, file: &InstructionsFile) -> Result<()> {
        store::write_atomic(&self.path, &serde_json::to_vec_pretty(file)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, InstructionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = InstructionStore::new(dir.path().join("instructions.json"));
        (dir, store)
    }

    #[test]
    fn a_missing_file_starts_with_the_starters() {
        let (_dir, store) = store();
        let view = store.view("/p");
        let ids: Vec<_> = view.templates.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["feature-spec", "bugfix-spec", "refactor-spec", "handoff", "devils-advocate"]);
        assert_eq!(view.project, ProjectInstructions::default());
    }

    #[test]
    fn a_corrupt_file_is_kept_aside_not_overwritten() {
        let (dir, store) = store();
        std::fs::write(dir.path().join("instructions.json"), "{ my precious templates").unwrap();
        assert_eq!(store.load(), InstructionsFile::default());
        let bad = dir.path().join("instructions.json.bad");
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ my precious templates");
        store.save_project("/p", ProjectInstructions { current: "x".into(), ..Default::default() }).unwrap();
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "{ my precious templates", "later saves never touch the .bad copy");
        assert_eq!(store.view("/p").project.current, "x");
    }

    #[test]
    fn a_second_corruption_never_destroys_the_first_kept_copy() {
        let (dir, store) = store();
        let file = dir.path().join("instructions.json");
        std::fs::write(&file, "first").unwrap();
        store.load();
        std::fs::write(&file, "second").unwrap();
        store.load();
        assert_eq!(std::fs::read_to_string(dir.path().join("instructions.json.bad")).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(dir.path().join("instructions.json.bad.1")).unwrap(), "second");
    }

    #[test]
    fn a_file_that_cannot_be_read_is_left_alone_and_never_overwritten() {
        let (dir, store) = store();
        // A directory where the file should be: reading fails, but not with NotFound.
        std::fs::create_dir(dir.path().join("instructions.json")).unwrap();
        assert_eq!(store.load(), InstructionsFile::default(), "the UI still gets the starters");
        assert!(store.save_project("/p", ProjectInstructions { current: "x".into(), ..Default::default() }).is_err());
        assert!(dir.path().join("instructions.json").is_dir(), "nothing was renamed or replaced");
        assert!(!dir.path().join("instructions.json.bad").exists(), "a read failure is not corruption");
    }

    #[test]
    fn a_corrupt_file_that_could_not_be_kept_is_an_error_not_defaults() {
        let err = start_from(Loaded::Corrupt { kept: None }).unwrap_err();
        assert!(matches!(&err, Error::Precondition(m) if m.contains("couldn't keep a copy")), "{err}");
        assert_eq!(start_from(Loaded::Missing).unwrap(), InstructionsFile::default());
        assert_eq!(
            start_from(Loaded::Corrupt { kept: Some(PathBuf::from("instructions.json.bad")) }).unwrap(),
            InstructionsFile::default()
        );
    }

    #[test]
    fn deleting_every_template_sticks_until_restore() {
        let (_dir, store) = store();
        store.save_templates(vec![]).unwrap();
        assert!(store.load().templates.is_empty(), "an empty list is the user's choice, not a missing file");
        let mut edited = starters()[1].clone();
        edited.text = "my own bug hunt".into();
        store.save_templates(vec![edited.clone()]).unwrap();
        let restored = store.restore_starters().unwrap();
        assert_eq!(restored.len(), 5);
        assert_eq!(restored[0], edited, "an edited starter keeps its edits and its place");
        assert_eq!(restored.iter().filter(|t| t.id == "bugfix-spec").count(), 1);
        assert_eq!(store.load().templates, restored);
    }

    #[test]
    fn projects_round_trip_and_empty_ones_are_dropped() {
        let (dir, store) = store();
        let p = ProjectInstructions { default: "d".into(), current: "c".into(), template_id: Some("bug-hunt".into()) };
        store.save_project("/a", p.clone()).unwrap();
        store.save_project("", ProjectInstructions { current: "no project".into(), ..Default::default() }).unwrap();
        assert_eq!(store.view("/a").project, p);
        assert_eq!(store.view("").project.current, "no project");
        store.save_project("/a", ProjectInstructions::default()).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("instructions.json")).unwrap()).unwrap();
        assert!(json["projects"].get("/a").is_none(), "nothing set means no entry");
        assert_eq!(json["projects"][""], serde_json::json!({"default": "", "current": "no project", "templateId": null}));
    }

    #[test]
    fn concurrent_saves_never_lose_an_update() {
        let (_dir, store) = store();
        std::thread::scope(|s| {
            for i in 0..8 {
                let store = &store;
                s.spawn(move || {
                    let p = ProjectInstructions { current: format!("{i}"), ..Default::default() };
                    store.save_project(&format!("/p{i}"), p).unwrap();
                });
            }
        });
        assert_eq!(store.load().projects.len(), 8);
    }

    #[test]
    fn starters_pick_catalogue_sections_per_the_spec() {
        let by_id = |id: &str| starters().into_iter().find(|t| t.id == id).map(|t| t.sections).unwrap();
        assert_eq!(by_id("feature-spec"), ["intent", "conflicts", "gaps", "ambiguities", "questions"]);
        assert_eq!(by_id("bugfix-spec"), ["intent", "conflicts", "gaps", "questions"]);
        assert_eq!(by_id("refactor-spec"), ["conflicts", "gaps", "ambiguities", "questions"]);
        assert_eq!(by_id("handoff"), ["gaps", "ambiguities"], "stored in catalogue order");
        assert_eq!(by_id("devils-advocate"), ["conflicts", "questions"]);
        for t in starters() {
            assert_eq!(crate::sections::normalize(&t.sections), t.sections, "{}: catalogue ids in catalogue order", t.id);
        }
    }

    #[test]
    fn a_template_without_sections_leaves_the_tabs_alone() {
        let t: Template = serde_json::from_str(r#"{"id":"mine","name":"Mine","text":"x"}"#).unwrap();
        assert!(t.sections.is_empty());
    }

    #[test]
    fn starters_stay_within_the_notes_format() {
        for t in starters() {
            assert!(!t.text.contains('#'), "{}: headings belong to the prompt, not a template", t.id);
            assert!(t.text.chars().count() < 600, "{}: short enough for small local models", t.id);
        }
    }
}
