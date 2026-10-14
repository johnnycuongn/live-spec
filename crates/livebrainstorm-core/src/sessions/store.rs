//! `SessionStore`: `workspace.json` and `tabs/<id>.json` under
//! `<app_data>/sessions/`. One `std` mutex guards the workspace and a cache of
//! the tabs read so far. It is held only for synchronous file I/O, never
//! across `.await`. The store is the only writer, so the cache stays current.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::{first_line, tab_id, valid_id, Snapshot, Tab, TabSummary, WindowView, Workspace, HISTORY_LEN};
use crate::store::{load_json, write_atomic, Loaded};
use crate::{Error, Result};

pub struct SessionStore {
    dir: PathBuf,
    inner: Mutex<Inner>,
    /// Unix milliseconds. Injectable, so tests can order timestamps.
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
}

#[derive(Default)]
struct Inner {
    /// Read on first use; `None` until then, or after a read error.
    workspace: Option<Workspace>,
    /// Every tab read so far, by id.
    tabs: HashMap<String, Tab>,
    /// `workspace.json` didn't exist when this process first read it, and no import has run.
    fresh: bool,
    /// Files kept as `.bad` during this process.
    unreadable: Vec<String>,
    /// The per-process part of new tab ids.
    counter: u64,
}

impl SessionStore {
    /// `dir` is `<app_data>/sessions`. Nothing is read until first use.
    pub fn new(dir: PathBuf) -> Self {
        Self::with_clock(dir, || SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64))
    }

    /// `clock` returns unix milliseconds.
    pub(crate) fn with_clock(dir: PathBuf, clock: impl Fn() -> u64 + Send + Sync + 'static) -> Self {
        Self { dir, inner: Mutex::new(Inner::default()), clock: Box::new(clock) }
    }

    // ---------- windows ----------

    /// Every window, in order. A first run has one empty window, `w-1`, saved
    /// with the first change. A workspace that can't be read gives none.
    pub fn window_ids(&self) -> Vec<String> {
        let mut inner = self.lock();
        match self.read_ws(&mut inner) {
            Ok(ws) => ws.windows.into_iter().map(|w| w.id).collect(),
            Err(e) => {
                eprintln!("LiveBrainstorm: couldn't read the saved windows: {e}");
                Vec::new()
            }
        }
    }

    /// One window's tabs, its full active tab, and every window's closed tabs.
    /// A tab file that can't be read is left out of the lists.
    pub fn window_view(&self, window: &str) -> Result<WindowView> {
        validate(window)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let ws = self.read_ws(inner)?;
        let entry = ws.windows.iter().find(|w| w.id == window).ok_or_else(window_closed)?;
        let mut tabs = Vec::new();
        for id in &entry.tabs {
            if let Some(tab) = self.read_tab_logged(inner, id) {
                tabs.push(tab.summary());
            }
        }
        let active = match &entry.active {
            Some(id) => self.read_tab_logged(inner, id),
            None => None,
        };
        let closed = self.closed_in(inner, &ws);
        Ok(WindowView {
            window: window.to_owned(),
            tabs,
            active,
            closed,
            fresh: inner.fresh,
            unreadable: inner.unreadable.clone(),
        })
    }

    /// A new, empty window, `w-<next_window>`.
    pub fn add_window(&self) -> Result<String> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        let id = ws.push_window();
        self.put_ws(inner, ws)?;
        Ok(id)
    }

    /// The window that lists `id`, if the tab is open.
    pub fn window_of(&self, id: &str) -> Option<String> {
        if !valid_id(id) {
            return None;
        }
        let mut guard = self.lock();
        let ws = self.read_ws(&mut guard).ok()?;
        ws.window_of(id).map(str::to_owned)
    }

    // ---------- tabs ----------

    /// A new, empty tab at the end of `window`, made active.
    pub fn create_tab(&self, window: &str, root: Option<String>) -> Result<Tab> {
        validate(window)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        if ws.window_mut(window).is_none() {
            return Err(window_closed());
        }
        let tab = self.new_tab(inner, root, String::new());
        // The file first: if the app stops before the workspace is saved, the
        // tab shows as closed rather than as a listed id with no file.
        self.put_tab(inner, tab.clone())?;
        let entry = ws.window_mut(window).ok_or_else(window_closed)?;
        entry.tabs.push(tab.id.clone());
        entry.active = Some(tab.id.clone());
        self.put_ws(inner, ws)?;
        Ok(tab)
    }

    /// Any tab file, open or closed.
    pub fn tab(&self, id: &str) -> Result<Tab> {
        validate(id)?;
        let mut guard = self.lock();
        self.existing_tab(&mut guard, id)
    }

    /// The tab's project, if a window lists the tab; "This tab was closed." otherwise.
    pub fn open_tab_root(&self, id: &str) -> Result<Option<String>> {
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        if self.read_ws(inner)?.window_of(id).is_none() {
            return Err(tab_closed());
        }
        Ok(self.existing_tab(inner, id)?.root)
    }

    /// Open or closed: the webview's pending save can land just after its tab closed.
    /// Returns whether the first line changed: it's the only part of a draft any list shows.
    pub fn save_draft(&self, id: &str, draft: String) -> Result<bool> {
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut tab = self.existing_tab(inner, id)?;
        if tab.draft == draft {
            return Ok(false);
        }
        let retitled = first_line(&tab.draft) != first_line(&draft);
        tab.draft = draft;
        tab.updated_at = self.now_secs();
        self.put_tab(inner, tab)?;
        Ok(retitled)
    }

    /// `None`, or a blank title, returns the tab to its automatic title.
    pub fn rename_tab(&self, id: &str, title: Option<String>) -> Result<()> {
        validate(id)?;
        let title = title.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut tab = self.existing_tab(inner, id)?;
        if tab.title == title {
            return Ok(());
        }
        tab.title = title;
        self.put_tab(inner, tab)
    }

    /// Shows `id` in `window`. A window shows only the tabs it lists.
    pub fn activate(&self, window: &str, id: &str) -> Result<()> {
        validate(window)?;
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        let elsewhere = ws.window_of(id).is_some_and(|w| w != window);
        let entry = ws.window_mut(window).ok_or_else(window_closed)?;
        if !entry.tabs.iter().any(|t| t == id) {
            return Err(if elsewhere { open_elsewhere() } else { tab_closed() });
        }
        if entry.active.as_deref() == Some(id) {
            return Ok(());
        }
        entry.active = Some(id.to_owned());
        self.put_ws(inner, ws)
    }

    /// Every tab file no window lists, newest first.
    pub fn closed(&self) -> Vec<TabSummary> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        match self.read_ws(inner) {
            Ok(ws) => self.closed_in(inner, &ws),
            Err(_) => Vec::new(), // which tabs are open is unknown
        }
    }

    // ---------- history ----------

    /// Saves `snap` as the tab's newest analysis and keeps `HISTORY_LEN` of
    /// them. Returns the id it was given.
    pub fn record_snapshot(&self, id: &str, mut snap: Snapshot) -> Result<u64> {
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut tab = self.existing_tab(inner, id)?;
        // Never reuse an id still in the history, even if the file lost its counter.
        let next = tab.history.iter().map(|h| h.id + 1).max().unwrap_or(1).max(tab.next_snapshot);
        snap.id = next;
        tab.next_snapshot = next + 1;
        tab.history.insert(0, snap);
        tab.history.truncate(HISTORY_LEN);
        self.put_tab(inner, tab)?;
        Ok(next)
    }

    /// Changes one saved analysis; a no-op once it has been trimmed away.
    pub fn update_snapshot(&self, id: &str, snapshot: u64, f: impl FnOnce(&mut Snapshot)) -> Result<()> {
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut tab = self.existing_tab(inner, id)?;
        let Some(saved) = tab.history.iter_mut().find(|h| h.id == snapshot) else { return Ok(()) };
        f(saved);
        self.put_tab(inner, tab)
    }

    // ---------- files ----------

    fn now_secs(&self) -> u64 {
        (self.clock)() / 1000
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("sessions lock")
    }

    fn workspace_path(&self) -> PathBuf {
        self.dir.join("workspace.json")
    }

    fn tab_path(&self, id: &str) -> PathBuf {
        self.dir.join("tabs").join(format!("{id}.json"))
    }

    /// The workspace, read on first use. A missing file is a first run: one
    /// empty window, saved with the first change. A file that doesn't parse is
    /// kept as `.bad` and replaced right away by one empty window, so its tabs
    /// show under Recently closed. A read error is returned and nothing is written.
    fn read_ws(&self, inner: &mut Inner) -> Result<Workspace> {
        if let Some(ws) = &inner.workspace {
            return Ok(ws.clone());
        }
        let path = self.workspace_path();
        let ws = match load_json::<Workspace>(&path)? {
            Loaded::Parsed(ws) => ws.sanitized(),
            Loaded::Missing => {
                inner.fresh = true;
                Workspace::first()
            }
            Loaded::Corrupt { kept: Some(kept) } => {
                inner.unreadable.push(kept.display().to_string());
                let ws = Workspace::first();
                if let Err(e) = write_json(&path, &ws) {
                    eprintln!("LiveBrainstorm: couldn't save a new workspace: {e}");
                }
                ws
            }
            Loaded::Corrupt { kept: None } => return Err(not_kept(&path)),
        };
        inner.workspace = Some(ws.clone());
        Ok(ws)
    }

    fn put_ws(&self, inner: &mut Inner, ws: Workspace) -> Result<()> {
        write_json(&self.workspace_path(), &ws)?;
        inner.workspace = Some(ws);
        Ok(())
    }

    /// A tab from the cache or its file. `None` when there's no such file, or
    /// it didn't parse (it's kept as `.bad` and skipped).
    fn read_tab(&self, inner: &mut Inner, id: &str) -> Result<Option<Tab>> {
        if let Some(tab) = inner.tabs.get(id) {
            return Ok(Some(tab.clone()));
        }
        let path = self.tab_path(id);
        match load_json::<Tab>(&path)? {
            Loaded::Parsed(mut tab) => {
                tab.id = id.to_owned(); // the file name is the id
                inner.tabs.insert(tab.id.clone(), tab.clone());
                Ok(Some(tab))
            }
            Loaded::Missing => Ok(None),
            Loaded::Corrupt { kept: Some(kept) } => {
                inner.unreadable.push(kept.display().to_string());
                Ok(None)
            }
            Loaded::Corrupt { kept: None } => Err(not_kept(&path)),
        }
    }

    /// For lists: a tab that can't be read is left out, never fatal to the list.
    fn read_tab_logged(&self, inner: &mut Inner, id: &str) -> Option<Tab> {
        self.read_tab(inner, id).unwrap_or_else(|e| {
            eprintln!("LiveBrainstorm: couldn't read tab {id}: {e}");
            None
        })
    }

    fn existing_tab(&self, inner: &mut Inner, id: &str) -> Result<Tab> {
        self.read_tab(inner, id)?.ok_or_else(tab_closed)
    }

    fn put_tab(&self, inner: &mut Inner, tab: Tab) -> Result<()> {
        write_json(&self.tab_path(&tab.id), &tab)?;
        inner.tabs.insert(tab.id.clone(), tab);
        Ok(())
    }

    /// A tab with a fresh id: `t-<millis hex>-<counter hex>`, never one in use.
    fn new_tab(&self, inner: &mut Inner, root: Option<String>, draft: String) -> Tab {
        let now = (self.clock)();
        let id = loop {
            inner.counter += 1;
            let id = tab_id(now, inner.counter);
            if !inner.tabs.contains_key(&id) && !self.tab_path(&id).exists() {
                break id;
            }
        };
        Tab { id, root, created_at: now / 1000, updated_at: now / 1000, draft, next_snapshot: 1, ..Tab::default() }
    }

    /// Every tab file no window lists, newest first: by `closed_at`, or, for a
    /// tab whose close never finished writing, by `updated_at`.
    fn closed_in(&self, inner: &mut Inner, ws: &Workspace) -> Vec<TabSummary> {
        let listed: HashSet<&str> = ws.windows.iter().flat_map(|w| w.tabs.iter().map(String::as_str)).collect();
        let mut closed = Vec::new();
        for id in self.tab_ids_on_disk() {
            if listed.contains(id.as_str()) {
                continue;
            }
            if let Some(tab) = self.read_tab_logged(inner, &id) {
                closed.push(tab.summary());
            }
        }
        closed.sort_by(|a, b| newest_first(b).cmp(&newest_first(a)).then_with(|| b.id.cmp(&a.id)));
        closed
    }

    /// Tab ids from `tabs/*.json`. `.tmp`, `.bad` and other names are skipped.
    fn tab_ids_on_disk(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.dir.join("tabs")) else { return Vec::new() };
        entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let id = name.strip_suffix(".json")?;
                (id.starts_with("t-") && valid_id(id)).then(|| id.to_owned())
            })
            .collect()
    }
}

fn validate(id: &str) -> Result<()> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(Error::Precondition("Unknown tab.".into()))
    }
}

fn tab_closed() -> Error {
    Error::Precondition("This tab was closed.".into())
}

fn window_closed() -> Error {
    Error::Precondition("This window was closed.".into())
}

fn open_elsewhere() -> Error {
    Error::Precondition("That tab is already open in another window.".into())
}

/// A file that didn't parse and couldn't be moved aside: never written over.
fn not_kept(path: &Path) -> Error {
    std::io::Error::other(format!("couldn't keep a copy of the damaged {}", path.display())).into()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value)?)
}

/// Recently closed order: when it was closed, else when it was last edited.
fn newest_first(t: &TabSummary) -> u64 {
    t.closed_at.unwrap_or(t.updated_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A store whose clock moves one second per reading, so timestamps order.
    fn store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = at(&dir);
        (dir, store)
    }

    /// A new store on the same folder: what the next launch reads.
    fn at(dir: &tempfile::TempDir) -> SessionStore {
        let ms = AtomicU64::new(1_760_400_000_000);
        SessionStore::with_clock(dir.path().join("sessions"), move || ms.fetch_add(1_000, Ordering::Relaxed))
    }

    fn sessions(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("sessions")
    }

    fn snap(draft: &str) -> Snapshot {
        Snapshot { draft: draft.into(), context: "/p|1|".into(), markdown: "## Intent\n- x".into(), ..Snapshot::default() }
    }

    fn ids(list: &[TabSummary]) -> Vec<&str> {
        list.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn a_first_run_has_one_empty_window_and_is_fresh() {
        let (dir, s) = store();
        assert_eq!(s.window_ids(), ["w-1"]);
        let view = s.window_view("w-1").unwrap();
        assert!(view.fresh);
        assert!(view.tabs.is_empty() && view.active.is_none() && view.closed.is_empty() && view.unreadable.is_empty());
        assert!(!sessions(&dir).join("workspace.json").exists(), "nothing is written until something changes");
    }

    #[test]
    fn tabs_are_created_saved_and_renamed() {
        let (dir, s) = store();
        let tab = s.create_tab("w-1", Some("/p".into())).unwrap();
        assert!(tab.id.starts_with("t-") && valid_id(&tab.id), "{}", tab.id);
        assert_eq!((tab.root.as_deref(), tab.draft.as_str(), tab.next_snapshot), (Some("/p"), "", 1));
        assert!(s.save_draft(&tab.id, "Retry failed payments".into()).unwrap(), "a new first line changes the lists");
        assert!(!s.save_draft(&tab.id, "Retry failed payments\nwith backoff".into()).unwrap(), "a later line doesn't");
        s.rename_tab(&tab.id, Some("  Checkout retry ".into())).unwrap();
        let view = s.window_view("w-1").unwrap();
        assert_eq!(ids(&view.tabs), [tab.id.as_str()]);
        assert_eq!(view.tabs[0].title.as_deref(), Some("Checkout retry"));
        assert_eq!(view.tabs[0].first_line, "Retry failed payments");
        assert!(view.tabs[0].updated_at > tab.updated_at, "saving a draft moves updatedAt");
        assert_eq!(view.active.map(|t| t.draft), Some("Retry failed payments\nwith backoff".to_owned()));
        s.rename_tab(&tab.id, Some("   ".into())).unwrap();
        assert_eq!(s.tab(&tab.id).unwrap().title, None, "a blank name returns to the automatic title");
        s.rename_tab(&tab.id, Some("Checkout retry".into())).unwrap();
        s.rename_tab(&tab.id, None).unwrap();
        // The next launch reads the same files back.
        let again = at(&dir);
        let read = again.tab(&tab.id).unwrap();
        assert_eq!((read.title, read.draft.as_str()), (None, "Retry failed payments\nwith backoff"));
        assert_eq!(again.open_tab_root(&tab.id).unwrap().as_deref(), Some("/p"));
        assert!(!again.window_view("w-1").unwrap().fresh, "the workspace file exists now");
    }

    #[test]
    fn a_new_tab_goes_last_and_becomes_active() {
        let (_dir, s) = store();
        let a = s.create_tab("w-1", None).unwrap();
        let b = s.create_tab("w-1", None).unwrap();
        assert_ne!(a.id, b.id);
        let view = s.window_view("w-1").unwrap();
        assert_eq!(ids(&view.tabs), [a.id.as_str(), b.id.as_str()]);
        assert_eq!(view.active.map(|t| t.id), Some(b.id.clone()));
        s.activate("w-1", &a.id).unwrap();
        assert_eq!(s.window_view("w-1").unwrap().active.map(|t| t.id), Some(a.id.clone()));
        assert_eq!(s.window_of(&a.id).as_deref(), Some("w-1"));
        assert_eq!(s.open_tab_root(&a.id).unwrap(), None, "a tab with no project");
        let w2 = s.add_window().unwrap();
        assert_eq!(s.activate(&w2, &a.id).unwrap_err().to_string(), "That tab is already open in another window.");
        assert_eq!(s.open_tab_root("t-dead").unwrap_err().to_string(), "This tab was closed.");
        assert_eq!(s.activate("w-1", "t-dead").unwrap_err().to_string(), "This tab was closed.");
    }

    #[test]
    fn windows_get_new_ids_and_unknown_ones_are_refused() {
        let (_dir, s) = store();
        assert_eq!(s.add_window().unwrap(), "w-2");
        assert_eq!(s.window_ids(), ["w-1", "w-2"]);
        assert_eq!(s.window_view("w-9").unwrap_err().to_string(), "This window was closed.");
        assert_eq!(s.create_tab("w-9", None).unwrap_err().to_string(), "This window was closed.");
        assert!(s.closed().is_empty(), "no tab file was written for it");
    }

    #[test]
    fn ids_that_fail_validation_never_reach_the_disk() {
        let (dir, s) = store();
        std::fs::write(dir.path().join("secret.json"), r#"{"draft":"private"}"#).unwrap();
        let too_long = format!("t-{}", "a".repeat(41));
        for bad in ["t-../secret", "t-../../secret", "t-1/2", "T-1", "t-", "x-1", "w-1;rm", too_long.as_str()] {
            assert_eq!(s.tab(bad).unwrap_err().to_string(), "Unknown tab.", "{bad}");
            assert_eq!(s.save_draft(bad, "x".into()).unwrap_err().to_string(), "Unknown tab.", "{bad}");
            assert_eq!(s.record_snapshot(bad, snap("x")).unwrap_err().to_string(), "Unknown tab.", "{bad}");
            assert_eq!(s.window_view(bad).unwrap_err().to_string(), "Unknown tab.", "{bad}");
            assert_eq!(s.create_tab(bad, None).unwrap_err().to_string(), "Unknown tab.", "{bad}");
        }
        assert_eq!(std::fs::read_to_string(dir.path().join("secret.json")).unwrap(), r#"{"draft":"private"}"#);
        assert!(!sessions(&dir).exists(), "nothing was written");
    }

    #[test]
    fn history_keeps_the_newest_five() {
        let (_dir, s) = store();
        let tab = s.create_tab("w-1", None).unwrap();
        let saved: Vec<u64> = (0..7).map(|i| s.record_snapshot(&tab.id, snap(&format!("draft {i}"))).unwrap()).collect();
        assert_eq!(saved, [1, 2, 3, 4, 5, 6, 7]);
        let t = s.tab(&tab.id).unwrap();
        assert_eq!(t.history.iter().map(|h| h.id).collect::<Vec<_>>(), [7, 6, 5, 4, 3]);
        assert_eq!(t.history[0].draft, "draft 6");
        assert_eq!(t.next_snapshot, 8);
    }

    #[test]
    fn update_snapshot_finds_its_id_and_ignores_trimmed_ones() {
        let (_dir, s) = store();
        let tab = s.create_tab("w-1", None).unwrap();
        let first = s.record_snapshot(&tab.id, snap("one")).unwrap();
        s.record_snapshot(&tab.id, snap("two")).unwrap();
        s.update_snapshot(&tab.id, first, |h| h.checks_stopped = true).unwrap();
        let t = s.tab(&tab.id).unwrap();
        assert_eq!((t.history[0].checks_stopped, t.history[1].checks_stopped), (false, true));
        for i in 0..HISTORY_LEN {
            s.record_snapshot(&tab.id, snap(&format!("more {i}"))).unwrap();
        }
        s.update_snapshot(&tab.id, first, |h| h.markdown = "changed".into()).unwrap(); // trimmed: nothing to update
        assert!(s.tab(&tab.id).unwrap().history.iter().all(|h| h.markdown != "changed"));
    }

    #[test]
    fn a_corrupt_workspace_is_kept_and_its_tabs_show_as_closed() {
        let (dir, s) = store();
        let a = s.create_tab("w-1", Some("/p".into())).unwrap();
        drop(s);
        std::fs::write(sessions(&dir).join("workspace.json"), "{ not json").unwrap();
        std::fs::write(sessions(&dir).join("workspace.json.bad"), "an older copy").unwrap();
        let s = at(&dir);
        let view = s.window_view("w-1").unwrap();
        assert!(view.tabs.is_empty() && !view.fresh, "a damaged file is not a first run");
        assert_eq!(ids(&view.closed), [a.id.as_str()]);
        assert_eq!(view.unreadable.len(), 1);
        assert!(view.unreadable[0].ends_with("workspace.json.bad.1"), "{:?}", view.unreadable);
        assert_eq!(std::fs::read_to_string(sessions(&dir).join("workspace.json.bad")).unwrap(), "an older copy");
        assert_eq!(std::fs::read_to_string(sessions(&dir).join("workspace.json.bad.1")).unwrap(), "{ not json");
        assert!(!at(&dir).window_view("w-1").unwrap().fresh, "an empty workspace was saved in its place right away");
    }

    #[test]
    fn a_lost_workspace_shows_every_tab_as_closed() {
        let (dir, s) = store();
        let a = s.create_tab("w-1", None).unwrap();
        let b = s.create_tab("w-1", None).unwrap();
        drop(s);
        std::fs::remove_file(sessions(&dir).join("workspace.json")).unwrap();
        let view = at(&dir).window_view("w-1").unwrap();
        assert!(view.tabs.is_empty());
        let mut closed = ids(&view.closed);
        closed.sort_unstable();
        let mut both = vec![a.id.as_str(), b.id.as_str()];
        both.sort_unstable();
        assert_eq!(closed, both);
    }

    #[test]
    fn a_corrupt_tab_is_kept_and_skipped() {
        let (dir, s) = store();
        let good = s.create_tab("w-1", None).unwrap();
        let bad = s.create_tab("w-1", None).unwrap(); // active
        drop(s);
        let tabs = sessions(&dir).join("tabs");
        std::fs::write(tabs.join(format!("{}.json", bad.id)), "garbage").unwrap();
        let s = at(&dir);
        let view = s.window_view("w-1").unwrap();
        assert_eq!(ids(&view.tabs), [good.id.as_str()]);
        assert!(view.active.is_none(), "the active tab was the damaged one");
        assert_eq!(view.unreadable.len(), 1);
        assert!(view.unreadable[0].ends_with(&format!("{}.json.bad", bad.id)), "{:?}", view.unreadable);
        assert_eq!(std::fs::read_to_string(tabs.join(format!("{}.json.bad", bad.id))).unwrap(), "garbage");
        assert_eq!(s.tab(&bad.id).unwrap_err().to_string(), "This tab was closed.");
        assert_eq!(s.tab(&good.id).unwrap().id, good.id, "the other tabs are fine");
    }

    #[test]
    fn a_tab_file_that_cannot_be_read_is_reported_and_left_alone() {
        let (dir, s) = store();
        let tab = s.create_tab("w-1", None).unwrap();
        drop(s);
        // A directory where the file should be: reading fails, but not with NotFound.
        let path = sessions(&dir).join(format!("tabs/{}.json", tab.id));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let s = at(&dir);
        assert!(s.tab(&tab.id).is_err());
        assert!(s.save_draft(&tab.id, "x".into()).is_err());
        let view = s.window_view("w-1").unwrap();
        assert!(view.tabs.is_empty() && view.unreadable.is_empty(), "left out of the list, and not counted as corrupt");
        assert!(path.is_dir(), "nothing was renamed or replaced");
    }

    #[test]
    fn a_workspace_that_cannot_be_read_is_never_overwritten() {
        let (dir, s) = store();
        let ws = sessions(&dir).join("workspace.json");
        std::fs::create_dir_all(&ws).unwrap();
        assert!(s.window_view("w-1").is_err());
        assert!(s.create_tab("w-1", None).is_err());
        assert!(s.window_ids().is_empty());
        assert!(ws.is_dir(), "nothing was renamed or replaced");
    }

    #[test]
    fn concurrent_changes_never_lose_an_update() {
        let (_dir, s) = store();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    s.create_tab("w-1", None).unwrap();
                });
            }
        });
        assert_eq!(s.window_view("w-1").unwrap().tabs.len(), 8);
    }
}
