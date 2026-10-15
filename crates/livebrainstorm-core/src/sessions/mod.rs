//! Tabs, windows and their saved analyses, kept under `<app_data>/sessions/`:
//! `workspace.json` lists each window's tabs and its active tab, and
//! `tabs/<id>.json` holds one tab's draft, title and last `HISTORY_LEN`
//! analyses. A closed tab is any tab file that no window lists, so there is
//! no separate closed list that could drift. Live analysis stays stateless:
//! the history is for the user to browse and is never sent to the model.

mod ids;
mod store;
mod windows;

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::jev::check::{CheckEvent, CheckResult};

pub use ids::{tab_id, valid_id, window_id};
pub use store::{Recorder, SessionStore};
pub use windows::WindowSummary;

/// Analyses kept per tab, newest first.
pub const HISTORY_LEN: usize = 5;
/// The most characters (Unicode scalars) of a draft's first line a tab title shows.
pub const TITLE_CHARS: usize = 48;

/// One analysis, as the notes pane showed it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Snapshot {
    /// Assigned by the store from the tab's `next_snapshot`.
    pub id: u64,
    /// Unix seconds.
    pub at: u64,
    /// The draft this analysis read.
    pub draft: String,
    /// The frontend's `analysisContextKey` for the run, opaque here. It lets
    /// the gate keep working after a restart.
    pub context: String,
    pub markdown: String,
    pub partial: bool,
    pub notices: Vec<String>,
    pub checks: Vec<CheckResult>,
    /// The claim count from the last `checking` event. Fewer checks than this,
    /// with `checks_stopped` false, means an edit cut the run off.
    pub checks_total: Option<usize>,
    /// The Conflicts count from the same event, so restored notes read "2 of 5
    /// conflicts checked" as they did live. The field keeps its old name: it
    /// is a file and IPC contract.
    pub checks_faults: Option<usize>,
    pub checks_stopped: bool,
}

/// One tab: `tabs/<id>.json`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Tab {
    pub id: String,
    /// `None`: no project; it analyses without code context.
    pub root: Option<String>,
    /// `None`: the sidebar shows the draft's first line.
    pub title: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub closed_at: Option<u64>,
    pub draft: String,
    /// The catalogue ids this tab's notes cover (`sections::CATALOGUE`), in
    /// catalogue order. Empty: the default set, as every tab from before
    /// this field existed.
    pub sections: Vec<String>,
    pub next_snapshot: u64,
    /// Newest first, at most `HISTORY_LEN`.
    pub history: Vec<Snapshot>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WindowEntry {
    pub id: String,
    pub tabs: Vec<String>,
    pub active: Option<String>,
}

/// `workspace.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Workspace {
    pub version: u32,
    pub next_window: u64,
    pub windows: Vec<WindowEntry>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self { version: 1, next_window: 1, windows: Vec::new() }
    }
}

/// What lists show; no draft body or history.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TabSummary {
    pub id: String,
    pub root: Option<String>,
    pub title: Option<String>,
    /// `first_line(draft)`.
    pub first_line: String,
    pub updated_at: u64,
    pub closed_at: Option<u64>,
}

/// One window's tabs, the full active tab, and the shared closed list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowView {
    pub window: String,
    /// This window's tabs, in stored order.
    pub tabs: Vec<TabSummary>,
    pub active: Option<Tab>,
    /// Every window's closed tabs, newest first.
    pub closed: Vec<TabSummary>,
    /// `workspace.json` didn't exist when this process first read it, and no
    /// import has run: the frontend imports the old build's drafts.
    pub fresh: bool,
    /// Files kept as `.bad` during this process.
    pub unreadable: Vec<String>,
}

/// A draft the old build kept in `localStorage`, keyed by project root.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyDraft {
    pub root: Option<String>,
    pub draft: String,
}

/// The draft's first non-empty line, trimmed, at most `TITLE_CHARS`
/// characters. Mirrors `firstLine` in `src/lib/tabTitle.ts`.
pub fn first_line(draft: &str) -> String {
    let line = draft.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    line.chars().take(TITLE_CHARS).collect::<String>().trim_end().to_owned()
}

impl Snapshot {
    /// Applies one fact-check event the way the notes pane does live.
    pub fn apply_check(&mut self, ev: &CheckEvent) {
        match ev {
            CheckEvent::Checking { count, total } => {
                self.checks_total = Some(*count);
                self.checks_faults = Some(*total);
            }
            CheckEvent::Result(result) => self.checks.push(result.clone()),
            // The fact-check failed: shown as a notice, and the rest were dropped.
            CheckEvent::Notice(message) => {
                self.notices.push(message.clone());
                self.checks_stopped = true;
            }
        }
    }
}

impl Tab {
    pub fn summary(&self) -> TabSummary {
        TabSummary {
            id: self.id.clone(),
            root: self.root.clone(),
            title: self.title.clone(),
            first_line: first_line(&self.draft),
            updated_at: self.updated_at,
            closed_at: self.closed_at,
        }
    }
}

impl Workspace {
    /// A first run: one empty window, `w-1`.
    pub fn first() -> Self {
        let mut ws = Self::default();
        ws.push_window();
        ws
    }

    /// Drops what a hand-edited or damaged file could hold that would break
    /// the rules: bad ids, a tab listed twice, an active tab the window
    /// doesn't list, and a `next_window` that would reuse an id.
    pub fn sanitized(mut self) -> Self {
        let mut seen = HashSet::new();
        let mut listed = HashSet::new();
        self.windows.retain(|w| w.id.starts_with("w-") && valid_id(&w.id) && seen.insert(w.id.clone()));
        for w in &mut self.windows {
            w.tabs.retain(|t| t.starts_with("t-") && valid_id(t) && listed.insert(t.clone()));
            if !w.active.as_ref().is_some_and(|a| w.tabs.contains(a)) {
                w.active = w.tabs.first().cloned();
            }
        }
        let highest = self.windows.iter().filter_map(|w| w.id.strip_prefix("w-")?.parse::<u64>().ok()).max().unwrap_or(0);
        self.next_window = self.next_window.max(highest.saturating_add(1));
        self
    }

    /// Adds an empty window, `w-<next_window>`, and returns its id.
    pub fn push_window(&mut self) -> String {
        let id = window_id(self.next_window);
        self.next_window = self.next_window.saturating_add(1);
        self.windows.push(WindowEntry { id: id.clone(), tabs: Vec::new(), active: None });
        id
    }

    pub fn window_mut(&mut self, id: &str) -> Option<&mut WindowEntry> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// The window that lists `tab`, if the tab is open.
    pub fn window_of(&self, tab: &str) -> Option<&str> {
        self.windows.iter().find(|w| w.tabs.iter().any(|t| t == tab)).map(|w| w.id.as_str())
    }
}

impl WindowEntry {
    /// Removes `tab`. If it was active, the tab that took its place (else the
    /// one before it) becomes active.
    pub fn remove_tab(&mut self, tab: &str) {
        let Some(pos) = self.tabs.iter().position(|t| t == tab) else { return };
        self.tabs.remove(pos);
        if self.active.as_deref() == Some(tab) {
            self.active = self.tabs.get(pos).or_else(|| self.tabs.last()).cloned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jev::check::Verdict;

    fn check() -> CheckResult {
        CheckResult {
            claim: "`x` is wrong".into(),
            verdict: Verdict::NotShown,
            confidence: Some(0.9),
            path: Some("src/x.rs".into()),
            line: Some(3),
        }
    }

    #[test]
    fn tabs_serialize_with_the_frontends_field_names() {
        let tab = Tab {
            id: "t-1".into(),
            root: Some("/p".into()),
            title: None,
            created_at: 1,
            updated_at: 2,
            closed_at: None,
            draft: "d".into(),
            sections: vec!["conflicts".into()],
            next_snapshot: 7,
            history: vec![Snapshot {
                id: 6,
                at: 3,
                draft: "d".into(),
                context: "/p|1|".into(),
                markdown: "## Intent".into(),
                partial: false,
                notices: vec![],
                checks: vec![check()],
                checks_total: Some(4),
                checks_faults: Some(5),
                checks_stopped: false,
            }],
        };
        let json = serde_json::to_value(&tab).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "id": "t-1", "root": "/p", "title": null, "createdAt": 1, "updatedAt": 2, "closedAt": null,
                "draft": "d", "sections": ["conflicts"], "nextSnapshot": 7,
                "history": [{
                    "id": 6, "at": 3, "draft": "d", "context": "/p|1|", "markdown": "## Intent", "partial": false,
                    "notices": [],
                    "checks": [{ "claim": "`x` is wrong", "verdict": "notShown", "confidence": 0.9, "path": "src/x.rs", "line": 3 }],
                    "checksTotal": 4, "checksFaults": 5, "checksStopped": false
                }]
            })
        );
        assert_eq!(serde_json::from_value::<Tab>(json).unwrap(), tab, "and reads back unchanged");
        let summary = serde_json::to_value(tab.summary()).unwrap();
        assert_eq!(summary["firstLine"], "d");
        assert_eq!(summary["closedAt"], serde_json::Value::Null);
    }

    #[test]
    fn a_tab_file_with_missing_fields_still_reads() {
        let tab: Tab = serde_json::from_str(r#"{"id":"t-1","draft":"x"}"#).unwrap();
        assert_eq!((tab.draft.as_str(), tab.next_snapshot, tab.history.len()), ("x", 0, 0));
        assert!(tab.sections.is_empty(), "a tab from the old build has the default set");
    }

    #[test]
    fn first_line_matches_the_frontend() {
        // The same strings as `firstLine` in src/lib/tabTitle.test.ts.
        assert_eq!(first_line(""), "");
        assert_eq!(first_line("\n  \n\t Retry failed checkout payments  \nsecond line"), "Retry failed checkout payments");
        assert_eq!(first_line("line one\r\nline two"), "line one");
        assert_eq!(first_line(&"a".repeat(60)), "a".repeat(48));
        assert_eq!(first_line(&format!("{} yz", "x".repeat(47))), "x".repeat(47));
        assert_eq!(first_line(&"😀".repeat(50)), "😀".repeat(48));
        // Unicode White_Space is trimmed (NEL, no-break and ideographic spaces); a BOM is not whitespace.
        assert_eq!(first_line("\u{85}\u{a0}Retry\u{2003}payments\u{3000}\nmore"), "Retry\u{2003}payments");
        assert_eq!(first_line("\u{feff}Retry"), "\u{feff}Retry");
    }

    #[test]
    fn checks_apply_as_the_notes_pane_shows_them() {
        let mut s = Snapshot::default();
        s.apply_check(&CheckEvent::Checking { count: 2, total: 3 });
        s.apply_check(&CheckEvent::Result(check()));
        assert_eq!((s.checks_total, s.checks_faults, s.checks.len(), s.checks_stopped), (Some(2), Some(3), 1, false));
        s.apply_check(&CheckEvent::Notice("Jev is unavailable.".into()));
        assert_eq!(s.notices, ["Jev is unavailable."]);
        assert!(s.checks_stopped, "a failed fact-check is checksStopped, as it is live");
    }

    #[test]
    fn a_damaged_workspace_is_sanitized() {
        let ws: Workspace = serde_json::from_str(
            r#"{"version":1,"nextWindow":1,"windows":[
                {"id":"w-1","tabs":["t-1","t-1","../x","t-2"],"active":"t-9"},
                {"id":"bad","tabs":["t-3"],"active":null},
                {"id":"w-4","tabs":["t-2"],"active":"t-2"},
                {"id":"w-1","tabs":[],"active":null}]}"#,
        )
        .unwrap();
        let ws = ws.sanitized();
        let windows: Vec<(&str, Vec<&str>, Option<&str>)> = ws
            .windows
            .iter()
            .map(|w| (w.id.as_str(), w.tabs.iter().map(String::as_str).collect(), w.active.as_deref()))
            .collect();
        assert_eq!(windows, [("w-1", vec!["t-1", "t-2"], Some("t-1")), ("w-4", vec![], None)]);
        assert_eq!(ws.next_window, 5, "a window id is never reused");
    }

    #[test]
    fn a_window_id_at_the_top_of_the_range_cannot_overflow() {
        let ws: Workspace = serde_json::from_str(
            r#"{"version":1,"nextWindow":1,"windows":[{"id":"w-18446744073709551615","tabs":[],"active":null}]}"#,
        )
        .unwrap();
        let mut ws = ws.sanitized();
        assert_eq!(ws.next_window, u64::MAX);
        ws.push_window(); // must not panic either
    }

    #[test]
    fn new_windows_count_up_from_the_first() {
        let mut ws = Workspace::first();
        assert_eq!(ws.windows.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(), ["w-1"]);
        assert_eq!(ws.push_window(), "w-2");
        assert_eq!(ws.next_window, 3);
        ws.window_mut("w-2").unwrap().tabs.push("t-a".into());
        assert_eq!(ws.window_of("t-a"), Some("w-2"));
        assert_eq!(ws.window_of("t-b"), None);
        assert!(ws.window_mut("w-9").is_none());
    }

    #[test]
    fn removing_the_active_tab_activates_its_neighbour() {
        let mut w = WindowEntry { id: "w-1".into(), tabs: vec!["t-a".into(), "t-b".into(), "t-c".into()], active: Some("t-b".into()) };
        w.remove_tab("t-b");
        assert_eq!(w.active.as_deref(), Some("t-c"), "the tab that took its place");
        w.remove_tab("t-c");
        assert_eq!(w.active.as_deref(), Some("t-a"), "at the end: the one before it");
        w.tabs.push("t-d".into());
        w.remove_tab("t-d");
        assert_eq!(w.active.as_deref(), Some("t-a"), "closing another tab leaves the active one");
        w.remove_tab("t-e");
        assert_eq!(w.tabs, ["t-a"], "a tab the window doesn't list changes nothing");
        w.remove_tab("t-a");
        assert!(w.tabs.is_empty() && w.active.is_none());
    }
}
