//! Window-level operations for the shell's runtime windows (Phase 3), built on
//! `SessionStore`'s public API.

use serde::Serialize;

use super::{SessionStore, TabSummary};
use crate::Result;

/// One window for the tab menu's "Move to …" items, named there by its active tab.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSummary {
    pub id: String,
    pub active: Option<TabSummary>,
    pub tabs: usize,
}

impl SessionStore {
    /// The windows to build at launch: every workspace window, in order, or one new
    /// empty window if the workspace lists none, so the app never starts without one.
    pub fn windows_to_open(&self) -> Result<Vec<String>> {
        let ids = self.window_ids();
        if !ids.is_empty() {
            return Ok(ids);
        }
        Ok(vec![self.add_window()?])
    }

    /// A window's close request. With another window open, its tabs go to Recently
    /// closed and it leaves the workspace: returns the closed tab ids. The last window is
    /// left as it is, because closing it quits and the next launch restores it: `None`.
    /// Decided from the workspace, not Tauri's window list, which still lists a window
    /// that is closing.
    pub fn close_window_unless_last(&self, window: &str) -> Result<Option<Vec<String>>> {
        if !self.window_ids().iter().any(|w| w != window) {
            return Ok(None);
        }
        self.close_window(window).map(Some)
    }

    /// Every window with its active tab, in workspace order.
    pub fn windows(&self) -> Result<Vec<WindowSummary>> {
        let mut out = Vec::new();
        for id in self.window_ids() {
            let view = self.window_view(&id)?;
            let active_id = view.active.map(|tab| tab.id);
            let active = view.tabs.iter().find(|t| Some(&t.id) == active_id.as_ref()).cloned();
            out.push(WindowSummary { tabs: view.tabs.len(), active, id });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("sessions"));
        (dir, store)
    }

    fn ids(tabs: &[TabSummary]) -> Vec<&str> {
        tabs.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn the_last_window_is_never_closed() {
        let (_dir, s) = store();
        let w1 = s.window_ids()[0].clone();
        let tab = s.create_tab(&w1, Some("/p".into())).unwrap();
        assert_eq!(s.close_window_unless_last(&w1).unwrap(), None);
        assert_eq!(s.window_ids(), [w1.as_str()]);
        assert_eq!(ids(&s.window_view(&w1).unwrap().tabs), [tab.id.as_str()]);
        assert!(s.closed().is_empty(), "nothing goes to Recently closed");
    }

    #[test]
    fn closing_one_of_two_windows_sends_its_tabs_to_recently_closed() {
        let (_dir, s) = store();
        let w1 = s.window_ids()[0].clone();
        let w2 = s.add_window().unwrap();
        let a = s.create_tab(&w2, None).unwrap();
        let b = s.create_tab(&w2, Some("/p".into())).unwrap();
        let mut closed = s.close_window_unless_last(&w2).unwrap().expect("another window is open");
        closed.sort();
        let mut both = vec![a.id.clone(), b.id.clone()];
        both.sort();
        assert_eq!(closed, both);
        assert_eq!(s.window_ids(), [w1.as_str()]);
        let recently: Vec<String> = s.closed().into_iter().map(|t| t.id).collect();
        assert!(recently.contains(&a.id) && recently.contains(&b.id), "{recently:?}");
        // A second close request right after (two windows closed in a row) finds w1 alone.
        assert_eq!(s.close_window_unless_last(&w1).unwrap(), None);
    }

    #[test]
    fn a_draft_saved_after_its_window_closed_is_kept() {
        // The webview's pending draft save can land after the shell closed its window.
        let (_dir, s) = store();
        let w2 = s.add_window().unwrap();
        let tab = s.create_tab(&w2, None).unwrap();
        s.close_window_unless_last(&w2).unwrap();
        s.save_draft(&tab.id, "typed just before closing".into()).unwrap();
        assert_eq!(s.tab(&tab.id).unwrap().draft, "typed just before closing");
        assert!(s.closed().iter().any(|t| t.id == tab.id), "and the tab stays closed");
    }

    #[test]
    fn launch_opens_every_workspace_window_in_order() {
        let (_dir, s) = store();
        let w1 = s.window_ids()[0].clone();
        let w2 = s.add_window().unwrap();
        assert_eq!(s.windows_to_open().unwrap(), [w1, w2]);
    }

    #[test]
    fn launch_never_opens_zero_windows() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(sessions.join("workspace.json"), r#"{"version":1,"nextWindow":3,"windows":[]}"#).unwrap();
        let s = SessionStore::new(sessions);
        let opened = s.windows_to_open().unwrap();
        assert_eq!(opened.len(), 1);
        assert_eq!(s.window_ids(), opened, "and it's saved in the workspace");
    }

    #[test]
    fn windows_are_named_by_their_active_tab() {
        let (_dir, s) = store();
        let w1 = s.window_ids()[0].clone();
        let w2 = s.add_window().unwrap();
        s.create_tab(&w1, None).unwrap();
        let b = s.create_tab(&w1, Some("/p".into())).unwrap(); // made active
        s.rename_tab(&b.id, Some("Checkout retry".into())).unwrap();
        let list = s.windows().unwrap();
        assert_eq!(list.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(), [w1.as_str(), w2.as_str()]);
        assert_eq!(list[0].tabs, 2);
        let active = list[0].active.as_ref().expect("w1 has an active tab");
        assert_eq!((active.id.as_str(), active.title.as_deref()), (b.id.as_str(), Some("Checkout retry")));
        assert_eq!((list[1].tabs, list[1].active.is_none()), (0, true));
    }

    #[test]
    fn a_moved_tab_leaves_its_window_and_shows_in_the_new_one() {
        let (_dir, s) = store();
        let w1 = s.window_ids()[0].clone();
        let a = s.create_tab(&w1, None).unwrap();
        let b = s.create_tab(&w1, None).unwrap(); // active
        let (from, to) = s.move_tab(&b.id, None).unwrap();
        assert_eq!(from, w1);
        assert!(s.window_ids().contains(&to), "None makes a new window");
        s.activate(&to, &b.id).unwrap(); // what the move_tab command does next
        let there = s.window_view(&to).unwrap();
        assert_eq!(ids(&there.tabs), [b.id.as_str()]);
        assert_eq!(there.active.map(|t| t.id), Some(b.id.clone()));
        let here = s.window_view(&w1).unwrap();
        assert_eq!(ids(&here.tabs), [a.id.as_str()]);
        assert_ne!(here.active.map(|t| t.id), Some(b.id.clone()), "the old window no longer shows it");
        // Into an existing window, the tab goes last.
        assert_eq!(s.move_tab(&a.id, Some(to.as_str())).unwrap(), (w1.clone(), to.clone()));
        assert_eq!(ids(&s.window_view(&to).unwrap().tabs), [b.id.as_str(), a.id.as_str()]);
    }
}
