//! `SessionStore`: `workspace.json` and `tabs/<id>.json` under
//! `<app_data>/sessions/`. One `std` mutex guards the workspace and a cache of
//! the tabs read so far. It is held only for synchronous file I/O, never
//! across `.await`. The store is the only writer, so the cache stays current.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::{first_line, tab_id, valid_id, LegacyDraft, Snapshot, Tab, TabSummary, WindowView, Workspace, HISTORY_LEN};
use crate::jev::check::CheckEvent;
use crate::store::{load_json, write_atomic, Loaded};
use crate::sections;
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
                eprintln!("Live Spec: couldn't read the saved windows: {e}");
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

    /// A new, empty tab at the end of `window`, made active, with `sections`
    /// (unknown ids dropped; nothing known means the default set).
    pub fn create_tab(&self, window: &str, root: Option<String>, sections: Vec<String>) -> Result<Tab> {
        validate(window)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        if ws.window_mut(window).is_none() {
            return Err(window_closed());
        }
        let mut tab = self.new_tab(inner, root, String::new());
        tab.sections = sections::normalize(&sections);
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

    /// Stores the tab's section choice, normalized. A non-empty list that names
    /// no catalogue section is refused: the user would see nothing.
    pub fn set_sections(&self, id: &str, sections: Vec<String>) -> Result<()> {
        validate(id)?;
        let normalized = sections::normalize(&sections);
        if normalized.is_empty() && !sections.is_empty() {
            return Err(Error::Precondition("Keep at least one section.".into()));
        }
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut tab = self.existing_tab(inner, id)?;
        if tab.sections == normalized {
            return Ok(());
        }
        tab.sections = normalized;
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

    /// Closes a tab: its window drops it (the next tab becomes active) and its
    /// file gets `closed_at`, so it shows under Recently closed. Returns the
    /// window it was in, or `None` if it was already closed. A tab its window
    /// lists but whose file is lost or damaged is dropped from the window all
    /// the same, so it can't stay listed forever.
    pub fn close_tab(&self, id: &str) -> Result<Option<String>> {
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        let Some(window) = ws.window_of(id).map(str::to_owned) else {
            self.existing_tab(inner, id)?;
            return Ok(None);
        };
        if let Some(entry) = ws.window_mut(&window) {
            entry.remove_tab(id);
        }
        // The workspace first: if the app stops before the tab file is written,
        // the tab is still closed (no window lists it) and sorts by its last edit.
        self.put_ws(inner, ws)?;
        if let Some(mut tab) = self.read_tab_logged(inner, id) {
            tab.closed_at = Some(self.now_secs());
            self.put_tab(inner, tab)?;
        }
        Ok(Some(window))
    }

    /// Closes every tab of `window` (they show under Recently closed) and
    /// removes the window. Returns the ids of the tabs it closed.
    pub fn close_window(&self, window: &str) -> Result<Vec<String>> {
        Ok(self.close_window_inner(window, false)?.unwrap_or_default())
    }

    /// `keep_last`: leave `window` alone (`None`) when no other window is listed. The check
    /// and the close share one lock, so two closes at once can't both see the other window.
    fn close_window_inner(&self, window: &str, keep_last: bool) -> Result<Option<Vec<String>>> {
        validate(window)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        if keep_last && !ws.windows.iter().any(|w| w.id != window) {
            return Ok(None);
        }
        let pos = ws.windows.iter().position(|w| w.id == window).ok_or_else(window_closed)?;
        let entry = ws.windows.remove(pos);
        self.put_ws(inner, ws)?;
        let now = self.now_secs();
        for id in &entry.tabs {
            let Some(mut tab) = self.read_tab_logged(inner, id) else { continue };
            tab.closed_at = Some(now);
            if let Err(e) = self.put_tab(inner, tab) {
                // Still closed (no window lists it); it sorts by its last edit.
                eprintln!("Live Spec: couldn't mark tab {id} closed: {e}");
            }
        }
        Ok(Some(entry.tabs))
    }

    /// `close_window`, unless `window` is the only window: then nothing changes (`None`).
    /// One lock covers the decision and the close.
    pub(super) fn close_window_if_another(&self, window: &str) -> Result<Option<Vec<String>>> {
        self.close_window_inner(window, true)
    }

    /// Puts a closed tab back, at the end of `window`, as its active tab.
    pub fn reopen_tab(&self, id: &str, window: &str) -> Result<()> {
        validate(id)?;
        validate(window)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        if ws.window_of(id).is_some() {
            return Err(open_elsewhere());
        }
        if ws.window_mut(window).is_none() {
            return Err(window_closed());
        }
        let mut tab = self.existing_tab(inner, id)?;
        tab.closed_at = None;
        self.put_tab(inner, tab)?;
        let entry = ws.window_mut(window).ok_or_else(window_closed)?;
        entry.tabs.push(id.to_owned());
        entry.active = Some(id.to_owned());
        self.put_ws(inner, ws)
    }

    /// Deletes a closed tab's file for good.
    pub fn delete_tab(&self, id: &str) -> Result<()> {
        validate(id)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        if self.read_ws(inner)?.window_of(id).is_some() {
            return Err(Error::Precondition("Close this tab before deleting it.".into()));
        }
        match std::fs::remove_file(self.tab_path(id)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        inner.tabs.remove(id);
        Ok(())
    }

    /// Moves an open tab to the end of window `to`, or of a new window
    /// (`None`), which then shows it. Its old window shows another tab.
    /// Returns `(from, to)`. The shell cancels the tab's run and makes it
    /// active in `to`.
    pub fn move_tab(&self, id: &str, to: Option<&str>) -> Result<(String, String)> {
        validate(id)?;
        if let Some(window) = to {
            validate(window)?;
        }
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        let from = ws.window_of(id).map(str::to_owned).ok_or_else(tab_closed)?;
        let to = match to {
            Some(window) if window == from => return Ok((from.clone(), from)),
            Some(window) if ws.windows.iter().any(|w| w.id == window) => window.to_owned(),
            Some(_) => return Err(window_closed()),
            None => ws.push_window(),
        };
        if let Some(source) = ws.window_mut(&from) {
            source.remove_tab(id);
        }
        let target = ws.window_mut(&to).ok_or_else(window_closed)?;
        target.tabs.push(id.to_owned());
        if target.active.is_none() {
            target.active = Some(id.to_owned());
        }
        self.put_ws(inner, ws)?;
        Ok((from, to))
    }

    /// The first launch after the upgrade: every non-empty draft the old build
    /// kept becomes a tab at the end of `window`, and `active_root`'s tab (else
    /// the first one imported) is active. Runs once: while the workspace is
    /// `fresh`, which it then stops being. Later calls do nothing.
    pub fn import_legacy(&self, window: &str, drafts: Vec<LegacyDraft>, active_root: Option<String>) -> Result<()> {
        validate(window)?;
        let mut guard = self.lock();
        let inner = &mut *guard;
        let mut ws = self.read_ws(inner)?;
        if !inner.fresh {
            return Ok(());
        }
        if ws.window_mut(window).is_none() {
            return Err(window_closed());
        }
        let mut imported: Vec<String> = Vec::new();
        let mut active = None;
        for legacy in drafts {
            if legacy.draft.trim().is_empty() {
                continue;
            }
            let tab = self.new_tab(inner, legacy.root, legacy.draft);
            if active.is_none() && tab.root == active_root {
                active = Some(tab.id.clone());
            }
            let id = tab.id.clone();
            if let Err(e) = self.put_tab(inner, tab) {
                // All or nothing: a retry (the workspace is still fresh) would
                // otherwise import these again.
                for done in &imported {
                    let _ = std::fs::remove_file(self.tab_path(done));
                    inner.tabs.remove(done);
                }
                return Err(e);
            }
            imported.push(id);
        }
        let entry = ws.window_mut(window).ok_or_else(window_closed)?;
        if let Some(first) = imported.first() {
            entry.active = Some(active.unwrap_or_else(|| first.clone()));
        }
        entry.tabs.extend(imported);
        self.put_ws(inner, ws)?;
        inner.fresh = false;
        Ok(())
    }

    /// The projects of every open tab, in any window: what the shell keeps loaded.
    pub fn open_roots(&self) -> HashSet<String> {
        let mut guard = self.lock();
        let inner = &mut *guard;
        let Ok(ws) = self.read_ws(inner) else { return HashSet::new() };
        let mut roots = HashSet::new();
        for id in ws.windows.iter().flat_map(|w| &w.tabs) {
            if let Some(Tab { root: Some(root), .. }) = self.read_tab_logged(inner, id) {
                roots.insert(root);
            }
        }
        roots
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
        let next = tab.history.iter().map(|h| h.id.saturating_add(1)).max().unwrap_or(1).max(tab.next_snapshot);
        snap.id = next;
        tab.next_snapshot = next.saturating_add(1);
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
                    eprintln!("Live Spec: couldn't save a new workspace: {e}");
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
            eprintln!("Live Spec: couldn't read tab {id}: {e}");
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

/// Saves one run to its tab's history. Fact-check events can come before or
/// after `done`: before, they're kept and saved with the notes; after, they
/// update the saved snapshot. A run that never reaches `done` (a skip, an
/// error, an edit that aborted it) saves nothing.
pub struct Recorder<'a> {
    store: &'a SessionStore,
    tab: String,
    pending: Snapshot,
    saved: Option<u64>,
}

impl<'a> Recorder<'a> {
    pub fn new(store: &'a SessionStore, tab: &str, draft: &str, context: &str) -> Self {
        let pending = Snapshot { draft: draft.to_owned(), context: context.to_owned(), ..Snapshot::default() };
        Self { store, tab: tab.to_owned(), pending, saved: None }
    }

    /// A notice about these notes: the pre-phase's, or the missing sections'.
    pub fn notice(&mut self, message: &str) -> Result<()> {
        self.apply(|s| s.notices.push(message.to_owned()))
    }

    pub fn check(&mut self, ev: &CheckEvent) -> Result<()> {
        self.apply(|s| s.apply_check(ev))
    }

    /// Saves the notes and returns the snapshot's id. Call it before sending
    /// `done`, so a `get_tab` right after `done` includes them.
    pub fn done(&mut self, markdown: &str, partial: bool) -> Result<u64> {
        let snap = Snapshot { at: self.store.now_secs(), markdown: markdown.to_owned(), partial, ..self.pending.clone() };
        let id = self.store.record_snapshot(&self.tab, snap)?;
        self.saved = Some(id);
        Ok(id)
    }

    fn apply(&mut self, f: impl FnOnce(&mut Snapshot)) -> Result<()> {
        match self.saved {
            None => {
                f(&mut self.pending);
                Ok(())
            }
            Some(id) => self.store.update_snapshot(&self.tab, id, f),
        }
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

/// Whether `e` is the "window is not in the workspace" precondition.
pub(super) fn is_window_closed(e: &Error) -> bool {
    matches!((e, window_closed()), (Error::Precondition(a), Error::Precondition(b)) if *a == b)
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
    use crate::jev::check::{CheckResult, Verdict};
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
        let tab = s.create_tab("w-1", Some("/p".into()), vec![]).unwrap();
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
    fn sections_are_stored_normalized_and_a_list_naming_nothing_is_refused() {
        let (_dir, store) = store();
        let w = store.window_ids()[0].clone();
        let tab = store.create_tab(&w, None, vec!["questions".into(), "intent".into(), "bogus".into()]).unwrap();
        assert_eq!(tab.sections, ["intent", "questions"], "catalogue order, unknown ids dropped");
        store.set_sections(&tab.id, vec!["gaps".into(), "gaps".into()]).unwrap();
        assert_eq!(store.tab(&tab.id).unwrap().sections, ["gaps"]);
        store.set_sections(&tab.id, vec![]).unwrap();
        assert!(store.tab(&tab.id).unwrap().sections.is_empty(), "empty is the default set");
        let err = store.set_sections(&tab.id, vec!["bogus".into()]).unwrap_err();
        assert!(matches!(&err, Error::Precondition(m) if m == "Keep at least one section."), "{err}");
        assert!(store.tab(&tab.id).unwrap().sections.is_empty(), "the refused list changed nothing");
        assert!(store.set_sections("t-missing", vec!["gaps".into()]).is_err());
        assert!(store.set_sections("../x", vec!["gaps".into()]).is_err(), "ids are validated first");
    }

    #[test]
    fn a_new_tab_goes_last_and_becomes_active() {
        let (_dir, s) = store();
        let a = s.create_tab("w-1", None, vec![]).unwrap();
        let b = s.create_tab("w-1", None, vec![]).unwrap();
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
        assert_eq!(s.create_tab("w-9", None, vec![]).unwrap_err().to_string(), "This window was closed.");
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
            assert_eq!(s.create_tab(bad, None, vec![]).unwrap_err().to_string(), "Unknown tab.", "{bad}");
        }
        assert_eq!(std::fs::read_to_string(dir.path().join("secret.json")).unwrap(), r#"{"draft":"private"}"#);
        assert!(!sessions(&dir).exists(), "nothing was written");
    }

    #[test]
    fn history_keeps_the_newest_five() {
        let (_dir, s) = store();
        let tab = s.create_tab("w-1", None, vec![]).unwrap();
        let saved: Vec<u64> = (0..7).map(|i| s.record_snapshot(&tab.id, snap(&format!("draft {i}"))).unwrap()).collect();
        assert_eq!(saved, [1, 2, 3, 4, 5, 6, 7]);
        let t = s.tab(&tab.id).unwrap();
        assert_eq!(t.history.iter().map(|h| h.id).collect::<Vec<_>>(), [7, 6, 5, 4, 3]);
        assert_eq!(t.history[0].draft, "draft 6");
        assert_eq!(t.next_snapshot, 8);
    }

    #[test]
    fn a_damaged_snapshot_counter_at_the_top_of_the_range_does_not_panic() {
        let (dir, s) = store();
        let tab = s.create_tab("w-1", None, vec![]).unwrap();
        drop(s);
        let path = sessions(&dir).join(format!("tabs/{}.json", tab.id));
        let mut doc: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        doc["nextSnapshot"] = u64::MAX.into();
        doc["history"] = serde_json::json!([{ "id": u64::MAX }]);
        std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
        let s = at(&dir);
        assert_eq!(s.record_snapshot(&tab.id, snap("x")).unwrap(), u64::MAX);
        assert!(!s.tab(&tab.id).unwrap().history.is_empty(), "saved, and the mutex is not poisoned");
        assert!(s.save_draft(&tab.id, "still works".into()).is_ok());
    }

    #[test]
    fn update_snapshot_finds_its_id_and_ignores_trimmed_ones() {
        let (_dir, s) = store();
        let tab = s.create_tab("w-1", None, vec![]).unwrap();
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
        let a = s.create_tab("w-1", Some("/p".into()), vec![]).unwrap();
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
        let a = s.create_tab("w-1", None, vec![]).unwrap();
        let b = s.create_tab("w-1", None, vec![]).unwrap();
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
        let good = s.create_tab("w-1", None, vec![]).unwrap();
        let bad = s.create_tab("w-1", None, vec![]).unwrap(); // active
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
    fn closing_a_listed_tab_whose_file_is_lost_drops_it_from_the_window() {
        let (dir, s) = store();
        let a = s.create_tab("w-1", None, vec![]).unwrap();
        let ghost = s.create_tab("w-1", None, vec![]).unwrap();
        let c = s.create_tab("w-1", None, vec![]).unwrap(); // active
        s.activate("w-1", &ghost.id).unwrap();
        drop(s);
        let tabs = sessions(&dir).join("tabs");
        std::fs::write(tabs.join(format!("{}.json", ghost.id)), "garbage").unwrap();
        let s = at(&dir);
        let view = s.window_view("w-1").unwrap();
        assert_eq!(ids(&view.tabs), [a.id.as_str(), c.id.as_str()]);
        assert!(view.active.is_none(), "the active tab is the damaged one");
        assert_eq!(s.close_tab(&ghost.id).unwrap(), Some("w-1".to_owned()), "no file is needed to close it");
        assert_eq!(s.window_of(&ghost.id), None);
        let view = s.window_view("w-1").unwrap();
        assert_eq!(ids(&view.tabs), [a.id.as_str(), c.id.as_str()]);
        assert_eq!(view.active.map(|t| t.id), Some(c.id.clone()), "the tab that took its place");
        s.delete_tab(&ghost.id).unwrap(); // nothing left on disk to delete: fine
        // A tab that was closed for real and whose file is gone still reports it.
        assert_eq!(s.close_tab(&ghost.id).unwrap_err().to_string(), "This tab was closed.");
    }

    #[test]
    fn a_tab_file_that_cannot_be_read_is_reported_and_left_alone() {
        let (dir, s) = store();
        let tab = s.create_tab("w-1", None, vec![]).unwrap();
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
        assert!(s.create_tab("w-1", None, vec![]).is_err());
        assert!(s.window_ids().is_empty());
        assert!(ws.is_dir(), "nothing was renamed or replaced");
    }

    #[test]
    fn concurrent_changes_never_lose_an_update() {
        let (_dir, s) = store();
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    s.create_tab("w-1", None, vec![]).unwrap();
                });
            }
        });
        assert_eq!(s.window_view("w-1").unwrap().tabs.len(), 8);
    }

    #[test]
    fn closing_a_tab_keeps_it_for_recently_closed() {
        let (_dir, s) = store();
        let a = s.create_tab("w-1", Some("/a".into()), vec![]).unwrap();
        let b = s.create_tab("w-1", None, vec![]).unwrap();
        let c = s.create_tab("w-1", None, vec![]).unwrap();
        s.activate("w-1", &b.id).unwrap();
        assert_eq!(s.close_tab(&b.id).unwrap().as_deref(), Some("w-1"));
        let view = s.window_view("w-1").unwrap();
        assert_eq!(ids(&view.tabs), [a.id.as_str(), c.id.as_str()]);
        assert_eq!(view.active.map(|t| t.id), Some(c.id.clone()), "the tab that took its place");
        assert_eq!(ids(&view.closed), [b.id.as_str()]);
        assert!(view.closed[0].closed_at.is_some());
        assert_eq!(s.open_tab_root(&b.id).unwrap_err().to_string(), "This tab was closed.");
        assert_eq!(s.tab(&b.id).unwrap().id, b.id, "its file stays");
        assert_eq!(s.close_tab(&b.id).unwrap(), None, "closing it again changes nothing");
        s.close_tab(&c.id).unwrap();
        assert_eq!(s.window_view("w-1").unwrap().active.map(|t| t.id), Some(a.id.clone()), "at the end: the one before it");
        s.close_tab(&a.id).unwrap();
        let empty = s.window_view("w-1").unwrap();
        assert!(empty.tabs.is_empty() && empty.active.is_none(), "the window stays, empty");
        assert_eq!(s.window_ids(), ["w-1"]);
    }

    #[test]
    fn recently_closed_is_newest_first_and_a_half_finished_close_sorts_by_its_last_edit() {
        let (dir, s) = store();
        let a = s.create_tab("w-1", None, vec![]).unwrap();
        let b = s.create_tab("w-1", None, vec![]).unwrap();
        s.close_tab(&a.id).unwrap();
        s.close_tab(&b.id).unwrap();
        let closed = s.closed();
        assert_eq!(ids(&closed), [b.id.as_str(), a.id.as_str()]);
        let (a_at, b_at) = (closed[1].closed_at.unwrap(), closed[0].closed_at.unwrap());
        // Tab files no window lists that never got `closedAt` (the app stopped mid-close),
        // and files that aren't tabs.
        let tabs = sessions(&dir).join("tabs");
        std::fs::write(tabs.join("t-c0ffee.json"), format!(r#"{{"id":"t-c0ffee","updatedAt":{}}}"#, b_at + 100)).unwrap();
        std::fs::write(tabs.join("t-01d.json"), format!(r#"{{"id":"t-01d","updatedAt":{}}}"#, a_at - 100)).unwrap();
        std::fs::write(tabs.join("t-abc.json.bad"), "kept").unwrap();
        std::fs::write(tabs.join("t-abc.json.tmp"), "half written").unwrap();
        std::fs::write(tabs.join("notes.txt"), "not a tab").unwrap();
        assert_eq!(ids(&s.closed()), ["t-c0ffee", b.id.as_str(), a.id.as_str(), "t-01d"]);
    }

    #[test]
    fn reopening_puts_a_tab_back_and_refuses_an_open_one() {
        let (_dir, s) = store();
        let a = s.create_tab("w-1", None, vec![]).unwrap();
        s.close_tab(&a.id).unwrap();
        let w2 = s.add_window().unwrap();
        s.reopen_tab(&a.id, &w2).unwrap();
        let view = s.window_view(&w2).unwrap();
        assert_eq!(ids(&view.tabs), [a.id.as_str()]);
        assert_eq!(view.active.as_ref().map(|t| (t.id.as_str(), t.closed_at)), Some((a.id.as_str(), None)));
        assert!(view.closed.is_empty());
        let err = s.reopen_tab(&a.id, "w-1").unwrap_err();
        assert!(matches!(err, Error::Precondition(_)));
        assert_eq!(err.to_string(), "That tab is already open in another window.");
        assert_eq!(s.reopen_tab("t-dead", "w-1").unwrap_err().to_string(), "This tab was closed.");
        s.close_tab(&a.id).unwrap();
        assert_eq!(s.reopen_tab(&a.id, "w-9").unwrap_err().to_string(), "This window was closed.");
    }

    #[test]
    fn only_closed_tabs_can_be_deleted() {
        let (dir, s) = store();
        let a = s.create_tab("w-1", None, vec![]).unwrap();
        assert_eq!(s.delete_tab(&a.id).unwrap_err().to_string(), "Close this tab before deleting it.");
        s.close_tab(&a.id).unwrap();
        s.delete_tab(&a.id).unwrap();
        assert!(!sessions(&dir).join(format!("tabs/{}.json", a.id)).exists());
        assert!(s.closed().is_empty());
        assert_eq!(s.tab(&a.id).unwrap_err().to_string(), "This tab was closed.");
        s.delete_tab(&a.id).unwrap(); // already gone: nothing to do
    }

    #[test]
    fn a_closed_tab_still_takes_a_late_draft_save() {
        let (_dir, s) = store();
        let a = s.create_tab("w-1", None, vec![]).unwrap();
        s.close_tab(&a.id).unwrap();
        s.save_draft(&a.id, "typed just before closing".into()).unwrap();
        assert_eq!(s.tab(&a.id).unwrap().draft, "typed just before closing");
        assert_eq!(ids(&s.closed()), [a.id.as_str()], "and it stays closed");
    }

    #[test]
    fn closing_a_window_closes_its_tabs() {
        let (_dir, s) = store();
        let w2 = s.add_window().unwrap();
        let x = s.create_tab(&w2, Some("/x".into()), vec![]).unwrap();
        let y = s.create_tab(&w2, None, vec![]).unwrap();
        assert_eq!(s.close_window(&w2).unwrap(), [x.id.clone(), y.id.clone()]);
        assert_eq!(s.window_ids(), ["w-1"]);
        let closed = s.closed();
        assert_eq!(closed.len(), 2);
        assert!(closed.iter().all(|t| t.closed_at.is_some()));
        assert_eq!(s.close_window(&w2).unwrap_err().to_string(), "This window was closed.");
        assert_eq!(s.add_window().unwrap(), "w-3", "window ids are never reused");
    }

    #[test]
    fn moving_a_tab() {
        let (_dir, s) = store();
        let a = s.create_tab("w-1", None, vec![]).unwrap();
        let b = s.create_tab("w-1", None, vec![]).unwrap(); // active
        assert_eq!(s.move_tab(&b.id, None).unwrap(), ("w-1".to_owned(), "w-2".to_owned()), "None makes a new window");
        let here = s.window_view("w-1").unwrap();
        assert_eq!(ids(&here.tabs), [a.id.as_str()]);
        assert_eq!(here.active.map(|t| t.id), Some(a.id.clone()), "the old window no longer shows it");
        let there = s.window_view("w-2").unwrap();
        assert_eq!(ids(&there.tabs), [b.id.as_str()]);
        assert_eq!(there.active.map(|t| t.id), Some(b.id.clone()));
        assert_eq!(s.move_tab(&a.id, Some("w-2")).unwrap(), ("w-1".to_owned(), "w-2".to_owned()));
        let there = s.window_view("w-2").unwrap();
        assert_eq!(ids(&there.tabs), [b.id.as_str(), a.id.as_str()], "it joins the end");
        assert_eq!(there.active.map(|t| t.id), Some(b.id.clone()), "that window keeps showing its own tab");
        assert!(s.window_view("w-1").unwrap().active.is_none());
        assert_eq!(s.move_tab(&a.id, Some("w-2")).unwrap(), ("w-2".to_owned(), "w-2".to_owned()), "already there");
        assert_eq!(s.move_tab(&a.id, Some("w-9")).unwrap_err().to_string(), "This window was closed.");
        assert_eq!(ids(&s.window_view("w-2").unwrap().tabs), [b.id.as_str(), a.id.as_str()], "a failed move changes nothing");
        s.close_tab(&a.id).unwrap();
        assert_eq!(s.move_tab(&a.id, None).unwrap_err().to_string(), "This tab was closed.");
    }

    #[test]
    fn legacy_drafts_are_imported_once() {
        let (dir, s) = store();
        assert!(s.window_view("w-1").unwrap().fresh);
        let drafts = vec![
            LegacyDraft { root: Some("/b".into()), draft: "b idea".into() },
            LegacyDraft { root: None, draft: "a loose idea".into() },
            LegacyDraft { root: Some("/a".into()), draft: "  \n".into() },
        ];
        s.import_legacy("w-1", drafts, Some("/b".into())).unwrap();
        let view = s.window_view("w-1").unwrap();
        assert!(!view.fresh);
        let tabs: Vec<(Option<&str>, &str)> = view.tabs.iter().map(|t| (t.root.as_deref(), t.first_line.as_str())).collect();
        assert_eq!(tabs, [(Some("/b"), "b idea"), (None, "a loose idea")], "empty drafts are skipped");
        assert_eq!(view.active.and_then(|t| t.root), Some("/b".to_owned()), "the last project's tab is active");
        s.import_legacy("w-1", vec![LegacyDraft { root: None, draft: "again".into() }], None).unwrap();
        assert_eq!(s.window_view("w-1").unwrap().tabs.len(), 2, "a second import does nothing");
        assert!(!at(&dir).window_view("w-1").unwrap().fresh, "nor does the next launch offer one");
    }

    #[test]
    fn a_failed_legacy_import_leaves_nothing_behind_and_can_be_retried() {
        let (dir, s) = store();
        let drafts = || {
            vec![
                LegacyDraft { root: None, draft: "one".into() },
                LegacyDraft { root: None, draft: "two".into() },
                LegacyDraft { root: None, draft: "three".into() },
            ]
        };
        // The second tab's id is known from the clock; a directory where its
        // temp file goes makes writing it fail.
        let blocked = sessions(&dir).join("tabs").join(format!("{}.json.tmp", tab_id(1_760_400_001_000, 2)));
        std::fs::create_dir_all(&blocked).unwrap();
        assert!(s.import_legacy("w-1", drafts(), None).is_err());
        let files = || std::fs::read_dir(sessions(&dir).join("tabs")).unwrap().flatten().count();
        assert_eq!(files(), 1, "only the blocking directory is left");
        assert!(s.window_view("w-1").unwrap().fresh, "still a first run");
        std::fs::remove_dir(&blocked).unwrap();
        s.import_legacy("w-1", drafts(), None).unwrap();
        let view = s.window_view("w-1").unwrap();
        assert_eq!(view.tabs.len(), 3, "the retry imports each draft once");
        assert!(view.closed.is_empty(), "no orphan from the failed try");
    }

    #[test]
    fn without_a_last_project_the_no_project_draft_is_active() {
        let (_dir, s) = store();
        let drafts = vec![
            LegacyDraft { root: Some("/a".into()), draft: "a".into() },
            LegacyDraft { root: None, draft: "loose".into() },
        ];
        s.import_legacy("w-1", drafts, None).unwrap();
        assert_eq!(s.window_view("w-1").unwrap().active.map(|t| t.draft), Some("loose".to_owned()));
        let (_dir2, s2) = store();
        s2.import_legacy("w-1", vec![LegacyDraft { root: Some("/a".into()), draft: "a".into() }], Some("/gone".into())).unwrap();
        assert_eq!(s2.window_view("w-1").unwrap().active.map(|t| t.draft), Some("a".to_owned()), "no match: the first tab");
    }

    #[test]
    fn open_roots_are_the_projects_of_open_tabs() {
        let (_dir, s) = store();
        s.create_tab("w-1", Some("/a".into()), vec![]).unwrap();
        let b = s.create_tab("w-1", Some("/b".into()), vec![]).unwrap();
        s.create_tab("w-1", None, vec![]).unwrap();
        let w2 = s.add_window().unwrap();
        s.create_tab(&w2, Some("/c".into()), vec![]).unwrap();
        s.close_tab(&b.id).unwrap();
        let mut roots: Vec<String> = s.open_roots().into_iter().collect();
        roots.sort_unstable();
        assert_eq!(roots, ["/a", "/c"]);
        assert_eq!(s.window_of(&b.id), None);
    }

    fn check(claim: &str) -> CheckResult {
        CheckResult {
            claim: claim.into(),
            verdict: Verdict::Confirmed,
            confidence: Some(0.9),
            path: Some("src/a.rs".into()),
            line: Some(1),
        }
    }

    #[test]
    fn a_run_is_saved_at_done_with_the_checks_that_came_first() {
        let (_dir, s) = store();
        let tab = s.create_tab("w-1", Some("/p".into()), vec![]).unwrap();
        let mut rec = Recorder::new(&s, &tab.id, "the draft", "/p|1|");
        rec.notice("Jev was slow; used keyword order.").unwrap();
        rec.check(&CheckEvent::Checking { count: 2, total: 3 }).unwrap();
        rec.check(&CheckEvent::Result(check("first"))).unwrap();
        assert!(s.tab(&tab.id).unwrap().history.is_empty(), "nothing is saved before done");
        let id = rec.done("## Intent\n- x", false).unwrap();
        let t = s.tab(&tab.id).unwrap();
        let saved = &t.history[0];
        assert_eq!(saved.id, id);
        assert_eq!((saved.draft.as_str(), saved.context.as_str(), saved.markdown.as_str()), ("the draft", "/p|1|", "## Intent\n- x"));
        assert_eq!(saved.notices, ["Jev was slow; used keyword order."]);
        assert_eq!(saved.checks, [check("first")]);
        assert_eq!((saved.checks_total, saved.checks_faults, saved.checks_stopped, saved.partial), (Some(2), Some(3), false, false));
        assert!(saved.at > 0);
        // Later checks and notices update the saved snapshot.
        rec.check(&CheckEvent::Result(check("second"))).unwrap();
        rec.check(&CheckEvent::Notice("Jev is unavailable.".into())).unwrap();
        let t = s.tab(&tab.id).unwrap();
        assert_eq!(t.history.len(), 1);
        assert_eq!(t.history[0].checks, [check("first"), check("second")]);
        assert_eq!(t.history[0].notices, ["Jev was slow; used keyword order.", "Jev is unavailable."]);
        assert!(t.history[0].checks_stopped);
    }

    #[test]
    fn checks_cut_off_by_an_edit_stay_short_of_their_total() {
        // The next keystroke aborts the run after done: the snapshot keeps what arrived.
        let (_dir, s) = store();
        let tab = s.create_tab("w-1", None, vec![]).unwrap();
        let mut rec = Recorder::new(&s, &tab.id, "d", "|0|");
        rec.check(&CheckEvent::Checking { count: 3, total: 3 }).unwrap();
        rec.done("## Faults\n- a", true).unwrap();
        rec.check(&CheckEvent::Result(check("first"))).unwrap();
        drop(rec);
        let t = s.tab(&tab.id).unwrap();
        assert_eq!((t.history[0].checks.len(), t.history[0].checks_total, t.history[0].checks_stopped), (1, Some(3), false));
        assert!(t.history[0].partial);
    }

    #[test]
    fn a_run_that_never_reaches_done_saves_nothing() {
        let (_dir, s) = store();
        let tab = s.create_tab("w-1", None, vec![]).unwrap();
        let mut rec = Recorder::new(&s, &tab.id, "d", "|0|");
        rec.notice("Jev was slow; used keyword order.").unwrap();
        rec.check(&CheckEvent::Checking { count: 1, total: 1 }).unwrap();
        drop(rec);
        assert!(s.tab(&tab.id).unwrap().history.is_empty());
        let mut gone = Recorder::new(&s, "t-dead", "d", "|0|");
        assert_eq!(gone.done("## Intent", false).unwrap_err().to_string(), "This tab was closed.");
    }
}
