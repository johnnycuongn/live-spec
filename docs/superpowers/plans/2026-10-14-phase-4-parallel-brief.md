# Phase 4: Parallel Brief — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the project brief with several explorer agents mapping slices of the repo at once, then one merge call. Keep that only as the default if it measurably beats a single explorer on this repo.

**Architecture:**
- **Core:** `livebrainstorm-core` gains `project::explore`, a set of pure helpers:
  - `slices(index, n)` splits the indexed files into balanced folder slices.
  - `collect_notes` gathers the explorers' notes and the slices nobody mapped.
  - `brief_source` builds the "written by" line.
  - `in_slice` tests whether a path belongs to a slice.
- **Digest and prompts:** `digest` gains `build_slice` and `overview`. `prompt` gains `explorer_request` and `merge_request`, and `ChatRequest` gains `max_turns`, so explorers get 20 turns.
- **Shell:** the Tauri `build_brief` command runs explorers via `agents::run_limited`, tagging their activity with `lane = agent number`, then streams the merge as lane 0. With one slice it takes today's single-explorer path unchanged.

**Tech Stack:** Rust 2021 (tokio, futures-util), Tauri v2, React 19, vitest.

**Spec:** `docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md` (phase 4: "Parallel brief"; section "Brief build (parallel explorers)"; Settings; Testing; and the phase 3 measurements, which motivate measuring here before keeping the default).

## Global Constraints

- **Slices:**
  - Group indexed files by their first path segment. Root-level files are excluded and go to the merge step.
  - If one group holds more than 50% of the bytes and has subfolders, split it one level deeper: its subfolders become groups, and files directly inside it become single-file entries.
  - Assign groups largest-first to the lightest slice. Slice count is `min(n, groups)`.
  - If fewer than 40 files are indexed, use 1 slice.
  - A slice is a list of prefixes: one ending in `/` is a folder subtree, one without is a single file.
- **`effective_brief_agents()`:** `max_agents()` if `parallel_brief` is on and the provider is not Local; otherwise 1.
- **One slice means today's path:** with 1 slice (or 1 agent), `build_brief` runs today's `prompt::brief_request` unchanged, with no merge step.
- **Explorers:**
  - **Claude CLI:** `claude -p` in the project root with today's read-only tools (`--tools Read,Glob,Grep`, `--restricted`) and `max_turns` 20.
  - **System prompt:** "You are agent {k} of {n} mapping this codebase in parallel. Explore only: {prefixes}. Others cover the rest. Write at most 400 words of Markdown notes: ## Modules (`path`: responsibility), ## Flows, ## Conventions, ## Features and gaps. Cite paths. Output only the notes."
  - **API provider:** no tools; the user message is `digest::build_slice(index, prefixes, budget)`.
- **Merge:** one Deep-tier call with no tools. Input:
  - each explorer's notes as `<notes agent="k" paths="…">`;
  - `digest::overview` (tree and key files);
  - `<unmapped>` with the prefixes of failed explorers, named under "Gaps and TODOs".

  Output: today's `BRIEF_FORMAT` (1,200 words), streamed as `text { lane: 0 }`.
- **Failure:** if some explorers fail, merge the rest. If all fail, return the first error. An explorer that returns empty text counts as failed.
- **Activity:** explorer activity is `activity { label, lane: k }` for k ≥ 1; explorers' own text is not shown. The drawer prefixes lines with "Agent k · ".
- **Stored brief:** `StoredBrief.source` gains " · N agents" when N > 1, e.g. "Claude Code (haiku) · 4 agents". Fingerprint, caching and staleness are unchanged.
- **Default decision (Task 7):** `parallel_brief` stays on by default only if, on this repo with Claude Code, 4 explorers finish at least 20% faster than 1 with no missing brief sections. Otherwise it defaults to off.
- **Process:**
  - Branch `feat/parallel-brief`.
  - Commit trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  - Never run `cargo fmt` across the repo; `cargo clippy --workspace --all-targets` stays warning-free.
  - `src-tauri` has no tests.
  - Build futures awaited in a spawned task in `for` loops, not `.map` closures over borrowed items.
  - Node 20.19+ for `npm run build` (prefix `PATH="$HOME/.nvm/versions/node/v24.15.0/bin:$PATH"`).

## Review Focus

1. **A small repo, `max_agents: 1`, the Local provider, or `parallelBrief: false`:** exactly today's single-explorer brief, with no merge call. Pinned in Task 1 (`brief_agents_follow_the_settings`) and Task 2 (`small_repos_get_one_slice`).
2. **Every indexed non-root file is covered by exactly one slice,** so nothing is silently unmapped and nothing is mapped twice. Pinned in Task 2 (`every_file_is_in_exactly_one_slice`).
3. **One explorer fails or returns nothing:** the brief still builds and names the unmapped paths. If all fail, the user sees the first error. Pinned in Task 2 (`failed_or_empty_explorers_become_unmapped`, `all_explorers_failing_is_an_error`).
4. **An API-provider explorer only sees its own slice:** no file outside the slice appears in its digest. Pinned in Task 3 (`a_slice_digest_has_only_its_files`) and Task 4 (`api_explorers_get_their_slice_digest`).
5. **A dominant folder** (for example `src/` holding most of the code) is split, not given to one agent while the others idle. Pinned in Task 2 (`a_dominant_folder_is_split_one_level`).

---

### Task 1: Brief agent count

**Files:**
- Modify: `crates/livebrainstorm-core/src/settings.rs`

**Interfaces:**
- Produces: `Settings::effective_brief_agents(&self) -> usize`.

- [ ] **Step 0: Branch**

```bash
git switch -c feat/parallel-brief
```

- [ ] **Step 1: Write the failing test** (append inside `mod tests` in `settings.rs`)

```rust
    #[test]
    fn brief_agents_follow_the_settings() {
        let mut s = Settings::default();
        assert_eq!(s.effective_brief_agents(), 4);
        s.agents.max_agents = 6;
        assert_eq!(s.effective_brief_agents(), 6, "explorers are not capped at four");
        s.agents.max_agents = 1;
        assert_eq!(s.effective_brief_agents(), 1);
        s.agents = AgentSettings { parallel_brief: false, ..AgentSettings::default() };
        assert_eq!(s.effective_brief_agents(), 1);
        s.agents = AgentSettings::default();
        s.provider = ProviderSettings::Local { model_id: "qwen3-4b-instruct".into() };
        assert_eq!(s.effective_brief_agents(), 1);
    }
```

- [ ] **Step 2: Run and confirm the failure**

Run: `cargo test -p livebrainstorm-core settings`
Expected: a compile error, because `effective_brief_agents` is not found.

- [ ] **Step 3: Implement** (inside `impl Settings`, after `effective_note_agents`)

```rust
    /// How many explorers map the project for the brief. The Local model gets
    /// one, because a single llama-server would queue the rest.
    pub fn effective_brief_agents(&self) -> usize {
        if self.agents.parallel_brief && !self.is_local() {
            self.max_agents()
        } else {
            1
        }
    }
```

- [ ] **Step 4: Run and confirm the pass**

Run: `cargo test -p livebrainstorm-core --lib && cargo clippy --workspace --all-targets`
Expected: all tests pass; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/settings.rs
git commit -m "feat(core): how many explorers build the brief" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: `project::explore` — slices, notes, source

**Files:**
- Create: `crates/livebrainstorm-core/src/project/explore.rs`
- Modify: `crates/livebrainstorm-core/src/project/mod.rs` (`pub mod explore;`)

**Interfaces:**
- Consumes: `ProjectIndex`, `FileEntry`, `scan` (existing); `crate::{Error, Result}`.
- Produces:
  - `explore::Slice { pub prefixes: Vec<String>, pub files: usize, pub bytes: u64 }`
  - `explore::MIN_FILES_TO_SPLIT: usize = 40`
  - `explore::in_slice(rel: &str, prefixes: &[String]) -> bool`
  - `explore::slices(index: &ProjectIndex, n: usize) -> Vec<Slice>`
  - `explore::Note { pub agent: usize, pub prefixes: Vec<String>, pub text: String }`
  - `explore::collect_notes(slices: &[Slice], results: Vec<Result<String>>) -> Result<(Vec<Note>, Vec<String>)>`, returning notes and unmapped prefixes
  - `explore::brief_source(provider_label: &str, agents: usize) -> String`

- [ ] **Step 1: Write the failing tests**

Add `pub mod explore;` to `project/mod.rs`, after `pub mod digest;`. Create `crates/livebrainstorm-core/src/project/explore.rs` containing only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::scan;

    /// A repo with the given (path, size in bytes) files.
    fn repo(files: &[(&str, usize)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, size) in files {
            let p = dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "x".repeat(*size)).unwrap();
        }
        dir
    }

    /// 12 files in each of a/ b/ c/ d/ (sizes 4:3:2:1), plus a root README.
    fn four_folders() -> tempfile::TempDir {
        let mut files: Vec<(String, usize)> = vec![("README.md".into(), 500)];
        for (folder, size) in [("a", 4_000), ("b", 3_000), ("c", 2_000), ("d", 1_000)] {
            for i in 0..12 {
                files.push((format!("{folder}/f{i}.rs"), size));
            }
        }
        let refs: Vec<(&str, usize)> = files.iter().map(|(p, s)| (p.as_str(), *s)).collect();
        repo(&refs)
    }

    #[test]
    fn small_repos_get_one_slice() {
        let files: Vec<(String, usize)> = (0..10).map(|i| (format!("src/f{i}.rs"), 100)).collect();
        let refs: Vec<(&str, usize)> = files.iter().map(|(p, s)| (p.as_str(), *s)).collect();
        let idx = scan(repo(&refs).path()).unwrap();
        assert_eq!(slices(&idx, 4).len(), 1);
    }

    #[test]
    fn folders_are_balanced_across_slices() {
        let idx = scan(four_folders().path()).unwrap();
        let two = slices(&idx, 2);
        assert_eq!(two.len(), 2);
        let mut groups: Vec<Vec<String>> = two.iter().map(|s| s.prefixes.clone()).collect();
        groups.sort();
        // 4+1 vs 3+2: largest-first into the lightest slice.
        assert_eq!(groups, [vec!["a/".to_string(), "d/".to_string()], vec!["b/".to_string(), "c/".to_string()]]);
        assert_eq!(slices(&idx, 8).len(), 4, "never more slices than folders");
        assert_eq!(slices(&idx, 1).len(), 1);
    }

    #[test]
    fn every_file_is_in_exactly_one_slice() {
        let idx = scan(four_folders().path()).unwrap();
        for n in 1..=5 {
            let s = slices(&idx, n);
            for f in idx.files.iter().filter(|f| f.rel.contains('/')) {
                let holders = s.iter().filter(|slice| in_slice(&f.rel, &slice.prefixes)).count();
                assert_eq!(holders, 1, "n={n} {}", f.rel);
            }
            assert!(s.iter().all(|slice| !in_slice("README.md", &slice.prefixes)), "root files go to the merge");
        }
    }

    #[test]
    fn a_dominant_folder_is_split_one_level() {
        let mut files: Vec<(String, usize)> = vec![("src/main.rs".into(), 2_000)];
        for i in 0..20 {
            files.push((format!("src/ui/f{i}.ts"), 3_000));
            files.push((format!("src/core/f{i}.rs"), 3_000));
        }
        for i in 0..5 {
            files.push((format!("docs/f{i}.md"), 500));
        }
        let refs: Vec<(&str, usize)> = files.iter().map(|(p, s)| (p.as_str(), *s)).collect();
        let idx = scan(repo(&refs).path()).unwrap();
        let all: Vec<String> = slices(&idx, 3).into_iter().flat_map(|s| s.prefixes).collect();
        for expected in ["src/ui/", "src/core/", "src/main.rs", "docs/"] {
            assert!(all.contains(&expected.to_string()), "{expected} in {all:?}");
        }
        assert!(!all.contains(&"src/".to_string()));
    }

    #[test]
    fn prefixes_match_folders_and_single_files() {
        let p = vec!["src/ui/".to_string(), "src/main.rs".to_string()];
        assert!(in_slice("src/ui/app.ts", &p) && in_slice("src/main.rs", &p));
        assert!(!in_slice("src/main.rs.bak", &p) && !in_slice("src/uix/a.ts", &p));
    }

    fn slice(prefix: &str) -> Slice {
        Slice { prefixes: vec![prefix.into()], files: 1, bytes: 1 }
    }

    #[test]
    fn failed_or_empty_explorers_become_unmapped() {
        let s = vec![slice("a/"), slice("b/"), slice("c/")];
        let (notes, unmapped) =
            collect_notes(&s, vec![Ok("## Modules\n- a".into()), Err(Error::Provider("boom".into())), Ok("  ".into())]).unwrap();
        assert_eq!(notes, vec![Note { agent: 1, prefixes: vec!["a/".into()], text: "## Modules\n- a".into() }]);
        assert_eq!(unmapped, ["b/", "c/"]);
    }

    #[test]
    fn all_explorers_failing_is_an_error() {
        let s = vec![slice("a/"), slice("b/")];
        let err = collect_notes(&s, vec![Err(Error::Provider("first".into())), Err(Error::Provider("second".into()))]).unwrap_err();
        assert_eq!(err.to_string(), "first");
    }

    #[test]
    fn source_names_the_agent_count() {
        assert_eq!(brief_source("Claude Code (haiku)", 1), "Claude Code (haiku)");
        assert_eq!(brief_source("Claude Code (haiku)", 4), "Claude Code (haiku) · 4 agents");
    }
}
```

- [ ] **Step 2: Run and confirm the failure**

Run: `cargo test -p livebrainstorm-core explore`
Expected: compile errors, because `slices`, `in_slice`, `Slice`, `Note`, `collect_notes` and `brief_source` are not found.

- [ ] **Step 3: Implement** (insert at the top of `explore.rs`)

```rust
//! Building the brief with parallel explorers: split the indexed files into
//! balanced folder slices, one per agent; collect each agent's notes; name
//! what nobody mapped. The shell runs the agents; this module stays pure.

use std::collections::BTreeMap;

use super::ProjectIndex;
use crate::{Error, Result};

/// Below this many indexed files one explorer maps everything.
pub const MIN_FILES_TO_SPLIT: usize = 40;

#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
    /// `dir/` is a folder subtree; a prefix without a trailing `/` is one file.
    pub prefixes: Vec<String>,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// 1-based, as shown in the drawer ("Agent 2 · …").
    pub agent: usize,
    pub prefixes: Vec<String>,
    pub text: String,
}

pub fn in_slice(rel: &str, prefixes: &[String]) -> bool {
    prefixes.iter().any(|p| if p.ends_with('/') { rel.starts_with(p.as_str()) } else { rel == p })
}

/// Up to `n` slices of roughly equal size. Root-level files belong to none
/// of them (the merge step reads them); fewer than `MIN_FILES_TO_SPLIT`
/// files, or `n <= 1`, gives a single slice.
pub fn slices(index: &ProjectIndex, n: usize) -> Vec<Slice> {
    // (prefix -> files, bytes) for every first-level folder.
    let mut groups: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    for f in index.files.iter().filter(|f| f.rel.contains('/')) {
        let top = format!("{}/", f.rel.split('/').next().unwrap_or_default());
        let entry = groups.entry(top).or_default();
        entry.0 += 1;
        entry.1 += f.size;
    }
    let one_flat_folder = groups.len() <= 1 && !has_subfolders(index, groups.keys().next());
    if n <= 1 || index.files.len() < MIN_FILES_TO_SPLIT || one_flat_folder {
        return vec![whole(&groups)];
    }

    // A folder holding most of the bytes is split one level deeper, so one
    // agent doesn't get nearly everything while the others idle.
    let total: u64 = groups.values().map(|(_, b)| b).sum();
    if let Some(big) = groups.iter().find(|(_, (_, b))| *b * 2 > total).map(|(p, _)| p.clone()) {
        if has_subfolders(index, Some(&big)) {
            groups.remove(&big);
            for f in index.files.iter().filter(|f| f.rel.starts_with(&big)) {
                let rest = &f.rel[big.len()..];
                let key = match rest.split_once('/') {
                    Some((sub, _)) => format!("{big}{sub}/"),
                    None => f.rel.clone(),
                };
                let entry = groups.entry(key).or_default();
                entry.0 += 1;
                entry.1 += f.size;
            }
        }
    }

    let count = n.min(groups.len()).max(1);
    let mut ordered: Vec<(String, (usize, u64))> = groups.into_iter().collect();
    ordered.sort_by(|a, b| b.1 .1.cmp(&a.1 .1).then(a.0.cmp(&b.0)));
    let mut out: Vec<Slice> = (0..count).map(|_| Slice { prefixes: vec![], files: 0, bytes: 0 }).collect();
    for (prefix, (files, bytes)) in ordered {
        let lightest = out.iter_mut().min_by_key(|s| s.bytes).expect("count >= 1");
        lightest.prefixes.push(prefix);
        lightest.files += files;
        lightest.bytes += bytes;
    }
    for s in &mut out {
        s.prefixes.sort();
    }
    out
}

fn whole(groups: &BTreeMap<String, (usize, u64)>) -> Slice {
    Slice {
        prefixes: groups.keys().cloned().collect(),
        files: groups.values().map(|(f, _)| f).sum(),
        bytes: groups.values().map(|(_, b)| b).sum(),
    }
}

fn has_subfolders(index: &ProjectIndex, prefix: Option<&String>) -> bool {
    prefix.is_some_and(|p| index.files.iter().any(|f| f.rel.strip_prefix(p.as_str()).is_some_and(|rest| rest.contains('/'))))
}

/// Notes from the explorers that succeeded with something to say, and the
/// prefixes of those that didn't. Only if none succeeded is it an error.
pub fn collect_notes(slices: &[Slice], results: Vec<Result<String>>) -> Result<(Vec<Note>, Vec<String>)> {
    let (mut notes, mut unmapped, mut first_error) = (Vec::new(), Vec::new(), None);
    for (i, (slice, result)) in slices.iter().zip(results).enumerate() {
        match result.map(|t| t.trim().to_owned()) {
            Ok(text) if !text.is_empty() => notes.push(Note { agent: i + 1, prefixes: slice.prefixes.clone(), text }),
            Ok(_) => unmapped.extend(slice.prefixes.iter().cloned()),
            Err(e) => {
                unmapped.extend(slice.prefixes.iter().cloned());
                first_error.get_or_insert(e);
            }
        }
    }
    if notes.is_empty() {
        return Err(first_error.unwrap_or_else(|| Error::Provider("No explorer returned any notes.".into())));
    }
    Ok((notes, unmapped))
}

/// "Written by …" for the drawer: the provider, plus the agent count.
pub fn brief_source(provider_label: &str, agents: usize) -> String {
    if agents > 1 {
        format!("{provider_label} · {agents} agents")
    } else {
        provider_label.to_owned()
    }
}

```

- [ ] **Step 4: Run and confirm the pass**

Run: `cargo test -p livebrainstorm-core --lib && cargo clippy --workspace --all-targets`
Expected: all tests pass, including the 8 new `explore::` tests; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/project
git commit -m "feat(core): split the repo into explorer slices and collect their notes" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Digest for a slice, and an overview for the merge

**Files:**
- Modify: `crates/livebrainstorm-core/src/project/digest.rs`

**Interfaces:**
- Consumes: `explore::in_slice` (Task 2).
- Produces:
  - `digest::build_slice(index: &ProjectIndex, prefixes: &[String], budget: usize) -> String`
  - `digest::overview(index: &ProjectIndex, budget: usize) -> String`, giving the tree and key files with no source excerpts
  - `digest::build` (unchanged signature and behaviour)

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `digest.rs`)

```rust
    #[test]
    fn a_slice_digest_has_only_its_files() {
        let idx = scan(fixture().path()).unwrap();
        let d = build_slice(&idx, &["src/billing/".to_string()], 4_000);
        assert!(d.contains("src/billing/invoice.ts") && d.contains("createInvoice"));
        assert!(!d.contains("CartStore") && !d.contains("src/cart/cart.ts"));
        assert!(!d.contains("An online shop"), "root files belong to the merge, not to a slice");
        assert!(d.len() <= 4_000);
    }

    #[test]
    fn the_overview_has_tree_and_key_files_but_no_excerpts() {
        let idx = scan(fixture().path()).unwrap();
        let o = overview(&idx, 4_000);
        assert!(o.contains("## File tree") && o.contains("src/cart/cart.ts"));
        assert!(o.contains("An online shop"));
        assert!(!o.contains("Source excerpts") && !o.contains("CartStore"));
    }
```

- [ ] **Step 2: Run and confirm the failure**

Run: `cargo test -p livebrainstorm-core digest`
Expected: compile errors, because `build_slice` and `overview` are not found.

- [ ] **Step 3: Implement**

Replace `pub fn build` and `fn push_tree` in `digest.rs` with:

```rust
pub fn build(index: &ProjectIndex, budget: usize) -> String {
    build_where(index, budget, |_| true, true)
}

/// The digest of one explorer's slice: its part of the tree, its key files
/// and the heads of its central files.
pub fn build_slice(index: &ProjectIndex, prefixes: &[String], budget: usize) -> String {
    build_where(index, budget, |rel| in_slice(rel, prefixes), true)
}

/// What the merge step needs besides the explorers' notes: the whole tree
/// and the key files (README, manifests), without source excerpts.
pub fn overview(index: &ProjectIndex, budget: usize) -> String {
    build_where(index, budget, |_| true, false)
}

fn build_where(index: &ProjectIndex, budget: usize, keep: impl Fn(&str) -> bool, excerpts: bool) -> String {
    let mut out = String::new();
    push_tree(&mut out, index, budget / 5, &keep);

    let (key, rest): (Vec<&FileEntry>, Vec<&FileEntry>) = index
        .files
        .iter()
        .filter(|f| keep(&f.rel))
        .partition(|f| KEY_FILES.contains(&file_name(&f.rel).to_lowercase().as_str()));

    out.push_str("\n## Key files\n");
    for f in key {
        if !push_file(&mut out, f, usize::MAX, budget / 8, budget) {
            break;
        }
    }
    if !excerpts {
        return out;
    }

    let mut rest = rest;
    rest.sort_by_key(|f| centrality(&f.rel));
    out.push_str("\n## Source excerpts (file heads)\n");
    for f in rest {
        if !push_file(&mut out, f, HEAD_LINES, budget / 20, budget) {
            break;
        }
    }
    out
}

fn push_tree(out: &mut String, index: &ProjectIndex, budget: usize, keep: &impl Fn(&str) -> bool) {
    let paths: Vec<&String> = index.tree.iter().filter(|p| keep(p)).collect();
    out.push_str(&format!("## File tree ({} files)\n", paths.len()));
    let full: String = paths.iter().map(|p| format!("{p}\n")).collect();
    if full.len() <= budget {
        out.push_str(&full);
        return;
    }
    // Too big: summarise by top-level directory.
    let mut dirs: std::collections::BTreeMap<&str, usize> = Default::default();
    for p in &paths {
        let top = p.split_once('/').map_or(p.as_str(), |(d, _)| d);
        *dirs.entry(top).or_default() += 1;
    }
    for (d, n) in dirs {
        out.push_str(&format!("{d}{}\n", if n > 1 { format!("/ ({n} files)") } else { String::new() }));
    }
}
```

Change the import line at the top of `digest.rs` to:

```rust
use super::explore::in_slice;
use super::{FileEntry, ProjectIndex};
```

- [ ] **Step 4: Run and confirm the pass**

Run: `cargo test -p livebrainstorm-core --lib && cargo clippy --workspace --all-targets`
Expected: all tests pass, including the existing `stays_within_budget_and_includes_readme` and the 2 new ones; no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/project/digest.rs
git commit -m "feat(core): slice digests for explorers and an overview for the merge" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Explorer and merge prompts; `max_turns`

**Files:**
- Modify: `crates/livebrainstorm-core/src/llm/mod.rs` (`ChatRequest.max_turns`)
- Modify: `crates/livebrainstorm-core/src/llm/claude_cli.rs` (`args` uses `max_turns`; test helper)
- Modify: `crates/livebrainstorm-core/src/prompt.rs` (`explorer_request`, `merge_request`; `max_turns: None` in the existing builders)

**Interfaces:**
- Consumes: `explore::{Slice, Note}` (Task 2); `digest::{build_slice, overview}` (Task 3).
- Produces:
  - `ChatRequest.max_turns: Option<u32>`. `None` means the default: 40 with repo tools, 1 without.
  - `prompt::explorer_request(index: &ProjectIndex, provider: &Provider, slice: &Slice, agent: usize, agents: usize) -> ChatRequest`
  - `prompt::merge_request(index: &ProjectIndex, provider: &Provider, notes: &[Note], unmapped: &[String]) -> ChatRequest`

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` in `prompt.rs`:

```rust
    use crate::llm::{AnthropicApi, ClaudeCli};
    use crate::project::explore::{Note, Slice};
    use crate::project::scan::tests::fixture;

    fn claude() -> Provider {
        Provider::ClaudeCli(ClaudeCli { binary: "claude".into(), model: "sonnet".into(), path_env: None })
    }

    fn api() -> Provider {
        Provider::AnthropicApi(AnthropicApi { api_key: "k".into(), model: "m".into(), http: reqwest::Client::new() })
    }

    #[test]
    fn claude_explorers_map_only_their_slice_with_20_turns() {
        let idx = crate::project::scan(fixture().path()).unwrap();
        let slice = Slice { prefixes: vec!["src/billing/".into(), "src/cart/".into()], files: 2, bytes: 100 };
        let req = explorer_request(&idx, &claude(), &slice, 2, 4);
        assert!(req.allow_repo_tools && req.cwd.as_deref() == Some(idx.root.as_path()));
        assert_eq!(req.max_turns, Some(20));
        assert!(req.system.contains("agent 2 of 4") && req.system.contains("src/billing/, src/cart/"));
        assert!(req.system.contains("at most 400 words") && req.system.contains("## Modules"));
    }

    #[test]
    fn api_explorers_get_their_slice_digest() {
        let idx = crate::project::scan(fixture().path()).unwrap();
        let slice = Slice { prefixes: vec!["src/billing/".into()], files: 1, bytes: 10 };
        let req = explorer_request(&idx, &api(), &slice, 1, 2);
        assert!(!req.allow_repo_tools && req.cwd.is_none());
        assert!(req.user.contains("createInvoice") && !req.user.contains("CartStore"));
    }

    #[test]
    fn the_merge_gets_every_note_the_overview_and_the_gaps() {
        let idx = crate::project::scan(fixture().path()).unwrap();
        let notes = vec![
            Note { agent: 1, prefixes: vec!["src/billing/".into()], text: "## Modules\n- billing".into() },
            Note { agent: 3, prefixes: vec!["src/cart/".into()], text: "## Modules\n- cart".into() },
        ];
        let req = merge_request(&idx, &claude(), &notes, &["docs/".into()]);
        assert!(!req.allow_repo_tools && req.max_turns.is_none() && req.cwd.is_none());
        assert!(req.user.contains("<notes agent=\"1\" paths=\"src/billing/\">") && req.user.contains("- cart"));
        assert!(req.user.contains("<unmapped>docs/</unmapped>") && req.user.contains("An online shop"));
        assert!(req.system.contains("## Purpose") && req.system.contains("1,200 words") && req.system.contains("Gaps and TODOs"));
    }
```

In `claude_cli.rs`, change the test helper to include the new field:

```rust
    fn request(allow_repo_tools: bool) -> ChatRequest {
        ChatRequest { system: "sys".into(), user: "u".into(), max_tokens: 900, cwd: None, allow_repo_tools, max_turns: None }
    }
```

and append inside its `mod tests`:

```rust
    #[test]
    fn a_turn_limit_overrides_the_default() {
        let req = ChatRequest { max_turns: Some(20), ..request(true) };
        assert_eq!(after(&args("sonnet", &req), "--max-turns"), Some("20"));
        assert_eq!(after(&args("sonnet", &request(true)), "--max-turns"), Some("40"));
    }
```

- [ ] **Step 2: Run and confirm the failure**

Run: `cargo test -p livebrainstorm-core --lib`
Expected: compile errors, because `max_turns` is not a field of `ChatRequest` and `explorer_request`/`merge_request` are not found.

- [ ] **Step 3: Implement**

1. In `llm/mod.rs`, add to `ChatRequest` after `allow_repo_tools`:

```rust
    /// Claude CLI turn limit; None means 40 with repo tools, 1 without.
    pub max_turns: Option<u32>,
```

2. In `claude_cli.rs` `args`, compute the limit once at the top of the function:

```rust
    let turns = req.max_turns.unwrap_or(if req.allow_repo_tools { 40 } else { 1 }).to_string();
```

Then replace the literal `"40"` in the repo-tools branch and `"1"` in the analysis branch with `turns.as_str()`.

3. In `prompt.rs`, add `max_turns: None,` to the `ChatRequest` literals in `brief_request` (both) and `analysis_request`. Then add to the imports:

```rust
use crate::project::explore::{Note, Slice};
```

and add after `brief_request`:

```rust
const EXPLORER_NOTES: &str = "\
Write at most 400 words of Markdown notes: ## Modules (`path`: responsibility), ## Flows, ## Conventions, \
## Features and gaps. Cite paths. Output only the notes.";

/// One of several explorers mapping a slice of the repo for the brief.
pub fn explorer_request(index: &ProjectIndex, provider: &Provider, slice: &Slice, agent: usize, agents: usize) -> ChatRequest {
    let paths = slice.prefixes.join(", ");
    if provider.can_explore_repo() {
        return ChatRequest {
            system: format!(
                "You are agent {agent} of {agents} mapping this codebase in parallel. Explore only: {paths}. \
                 Others cover the rest. Use read-only tools; don't read every file. {EXPLORER_NOTES}"
            ),
            user: format!("Map your part of the project \"{}\".", index.name),
            max_tokens: 1_500,
            cwd: Some(index.root.clone()),
            allow_repo_tools: true,
            max_turns: Some(20),
        };
    }
    let budget = provider.context_budget_chars() * 3 / 4;
    ChatRequest {
        system: format!(
            "You are agent {agent} of {agents} mapping part of a codebase ({paths}) from the excerpts provided. {EXPLORER_NOTES}"
        ),
        user: format!("<project name=\"{}\">\n{}\n</project>", index.name, digest::build_slice(index, &slice.prefixes, budget)),
        max_tokens: 1_000,
        cwd: None,
        allow_repo_tools: false,
        max_turns: None,
    }
}

/// Combines the explorers' notes into the brief. No tools: everything it
/// needs is in the message.
pub fn merge_request(index: &ProjectIndex, provider: &Provider, notes: &[Note], unmapped: &[String]) -> ChatRequest {
    let format = BRIEF_FORMAT.replace("{WORDS}", "1,200");
    let mut user = format!(
        "<project name=\"{}\">\n{}\n</project>\n",
        index.name,
        digest::overview(index, provider.context_budget_chars() / 4)
    );
    for n in notes {
        user.push_str(&format!("<notes agent=\"{}\" paths=\"{}\">\n{}\n</notes>\n", n.agent, n.prefixes.join(", "), n.text));
    }
    if !unmapped.is_empty() {
        user.push_str(&format!("<unmapped>{}</unmapped>\n", unmapped.join(", ")));
    }
    ChatRequest {
        system: format!(
            "You are combining notes from {} agents who each mapped part of a codebase into one project brief. \
             If <unmapped> lists paths, say under ## Gaps and TODOs that they were not mapped. {format}",
            notes.len()
        ),
        user,
        max_tokens: 4_000,
        cwd: None,
        allow_repo_tools: false,
        max_turns: None,
    }
}
```

- [ ] **Step 4: Run and confirm the pass**

Run: `cargo test -p livebrainstorm-core --lib && cargo clippy --workspace --all-targets`
Expected: all tests pass (the 3 new prompt tests, the 1 new claude_cli test, and every existing test), and the shell compiles; no warnings. If any other `ChatRequest { … }` literal fails to compile, add `max_turns: None,` to it.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core
git commit -m "feat(core): explorer and merge prompts; per-request turn limit" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Shell — run the explorers in `build_brief`

**Files:**
- Modify: `src-tauri/src/commands.rs` (`build_brief`)
- Modify: `AGENTS.md`, `README.md`

**Interfaces:**
- Consumes: `Settings::effective_brief_agents` (Task 1); `explore::{slices, collect_notes, brief_source}` (Task 2); `prompt::{brief_request, explorer_request, merge_request}` (Task 4); `agents::run_limited` (phase 3).
- Produces the IPC contract Task 6 mirrors:
  - `activity { label, lane: k }` for explorer k ≥ 1
  - the merge streams as `text { lane: 0 }`
  - with one slice, events are exactly as today (lane 0)

- [ ] **Step 1: Replace `build_brief`**

```rust
#[tauri::command]
pub async fn build_brief(app: AppHandle, on_event: Channel<UiEvent>) -> Result<()> {
    let state = app.state::<AppState>();
    let index = state.project().ok_or_else(|| Error::Precondition("Open a project first.".into()))?;
    let task_app = app.clone();

    let task = tauri::async_runtime::spawn(async move {
        let result = async {
            let settings = task_app.state::<AppState>().settings();
            let provider = provider::resolve(&task_app, Tier::Deep).await?;
            let slices = explore::slices(&index, settings.effective_brief_agents());
            let (text, agents) = if slices.len() <= 1 {
                // One explorer: today's path, unchanged.
                let req = prompt::brief_request(&index, &provider);
                (provider.stream(&req, &mut on_chunk(&on_event)).await?, 1)
            } else {
                // Explorers map their slices at once; their activity shows as
                // "Agent k". A `for` loop, not `.map(…)`, inside this spawned task.
                let n = slices.len();
                let mut explorers = Vec::new();
                for (k, slice) in slices.iter().enumerate() {
                    let req = prompt::explorer_request(&index, &provider, slice, k + 1, n);
                    let (provider, on_event) = (&provider, &on_event);
                    explorers.push(async move {
                        let lane = k + 1;
                        let mut sink = |chunk: Chunk| {
                            if let Chunk::Activity(label) = chunk {
                                send(on_event, UiEvent::Activity { label, lane });
                            }
                        };
                        provider.stream(&req, &mut sink).await
                    });
                }
                let (notes, unmapped) = explore::collect_notes(&slices, agents::run_limited(n, explorers).await)?;
                let req = prompt::merge_request(&index, &provider, &notes, &unmapped);
                (provider.stream(&req, &mut on_chunk(&on_event)).await?, n)
            };
            let brief = StoredBrief {
                root: index.root.display().to_string(),
                fingerprint: index.fingerprint.clone(),
                text: text.trim().to_owned(),
                created_at: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
                source: explore::brief_source(&settings.provider.label(), agents),
            };
            let state = task_app.state::<AppState>();
            state.briefs.save(&brief)?;
            // Only adopt it if the user hasn't switched projects meanwhile.
            if state.project().is_some_and(|p| p.root == index.root) {
                *state.brief.write().expect("brief lock") = Some(brief.clone());
            }
            Ok(brief.text)
        }
        .await;
        finish(&on_event, result);
    });
    replace_task(&state.brief_task, task);
    Ok(())
}
```

Add `use livebrainstorm_core::project::explore;` to the imports. The existing `use livebrainstorm_core::project::{self, BriefStatus, StoredBrief};` stays.

- [ ] **Step 2: Build and lint**

Run: `cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core`
Expected: no warnings; all tests pass. If rustc reports "FnOnce is not general enough", replace the closure over a borrowed item with an owned value or a loop, ledger a ruling, and re-run.

- [ ] **Step 3: Update the docs**

1. In `AGENTS.md`, under "### Project understanding: two layers", replace the "**Brief (once per project).**" bullet with:

```markdown
- **Brief (once per project).** With `Settings::effective_brief_agents()` > 1 and at least 40 indexed files, `project::explore::slices` splits the non-root files into balanced folder slices (a folder with more than half the bytes is split one level deeper). Explorers run at once via `agents::run_limited`: with Claude CLI each is `claude -p` with read-only tools and 20 turns, told "Explore only: <prefixes>"; with the API each gets `digest::build_slice`. Their activity streams as `activity { lane: k }` ("Agent k" in the drawer); their own text isn't shown. One merge call (no tools) gets the notes, `digest::overview` and an `<unmapped>` list of failed slices, and streams the brief in today's format as lane 0. With one slice (small repo, 1 agent, Local, or `agents.parallelBrief: false`) it's the single explorer as before. Briefs are cached at `<app_data>/briefs/<fnv(root)>.json` and marked stale when the project fingerprint changes; `source` notes the agent count.
```

2. In `README.md`, under "**Understanding the project happens in two layers**", append to item 1:

```markdown
 With several agents allowed (`agents.maxAgents`, `agents.parallelBrief`), larger repos are split into slices mapped by parallel explorers, and one call merges their notes.
```

- [ ] **Step 4: Commit**

```bash
git add src-tauri AGENTS.md README.md
git commit -m "feat: build the brief with parallel explorers and one merge call" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Frontend — tag explorer activity

**Files:**
- Create: `src/lib/brief.ts`, `src/lib/brief.test.ts`
- Modify: `src/App.tsx` (the brief event handler)

**Interfaces:**
- Consumes: the IPC contract from Task 5 (`activity { label, lane }`).
- Produces: `activityLine(label: string, lane: number): string`.

- [ ] **Step 1: Write the failing test** (`src/lib/brief.test.ts`)

```ts
import { describe, expect, it } from "vitest";
import { activityLine } from "./brief";

describe("activityLine", () => {
  it("tags explorer activity with its agent", () => {
    expect(activityLine("Read src/main.rs", 2)).toBe("Agent 2 · Read src/main.rs");
  });

  it("leaves single-explorer activity as it was", () => {
    expect(activityLine("Read src/main.rs", 0)).toBe("Read src/main.rs");
  });
});
```

- [ ] **Step 2: Run and confirm the failure**

Run: `npm test`
Expected: FAIL, because `./brief` cannot be resolved.

- [ ] **Step 3: Implement**

`src/lib/brief.ts`:

```ts
// While the brief builds, parallel explorers report on lanes 1..n and the
// single explorer (or the merge) on lane 0. Mirrors build_brief in Rust.
export const activityLine = (label: string, lane: number) => (lane > 0 ? `Agent ${lane} · ${label}` : label);
```

In `src/App.tsx`, add `import { activityLine } from "./lib/brief";`. In `buildBrief`, change

```ts
        else if (ev.type === "activity") setBriefActivity((a) => [...a, ev.label]);
```

to

```ts
        else if (ev.type === "activity") setBriefActivity((a) => [...a, activityLine(ev.label, ev.lane)]);
```

- [ ] **Step 4: Verify**

Run: `npm test && npm run typecheck && PATH="$HOME/.nvm/versions/node/v24.15.0/bin:$PATH" npm run build`
Expected: 31 tests pass (29 existing and 2 new); no type errors; the build prints `✓ built in`.

- [ ] **Step 5: Commit**

```bash
git add src
git commit -m "feat(ui): tag brief activity with the explorer that did it" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Measure 1 vs 4 explorers, decide the default, verify end to end

**Files:**
- Create: `crates/livebrainstorm-core/tests/brief_live.rs`
- Modify: `docs/superpowers/specs/2026-10-08-parallel-agents-and-jev-design.md` (a "Measurements (phase 4)" section)
- Modify: `AGENTS.md` (the live-test command)
- Possibly modify: `crates/livebrainstorm-core/src/settings.rs` (default), `README.md`, only per Step 3's rule

**Interfaces:**
- Consumes: `explore::{slices, collect_notes}`, `prompt::{brief_request, explorer_request, merge_request}`, `agents::run_limited`, `llm::{ClaudeCli, Provider, Chunk}`, `shell_env`, `project::scan`.

- [ ] **Step 1: Write the opt-in live test** (`crates/livebrainstorm-core/tests/brief_live.rs`)

```rust
//! Builds this repo's brief for real with Claude Code (sonnet; uses the
//! subscription, several minutes): 1 explorer vs 4 explorers plus merge.
//!   cargo test -p livebrainstorm-core --test brief_live -- --ignored --nocapture

use std::path::Path;
use std::time::Instant;

use livebrainstorm_core::llm::{Chunk, ClaudeCli, Provider};
use livebrainstorm_core::project::{explore, scan};
use livebrainstorm_core::{agents, prompt, shell_env};

const SECTIONS: [&str; 8] =
    ["Purpose", "Architecture", "Modules", "Core flows", "Conventions", "Existing features", "Gaps and TODOs", "Glossary"];

#[tokio::test]
#[ignore = "runs Claude Code several times; uses the subscription"]
async fn four_explorers_vs_one() {
    let path_env = shell_env::login_shell_path();
    let Some(binary) = shell_env::find_executable("claude", path_env.as_deref()) else {
        eprintln!("skipped: claude not found");
        return;
    };
    let provider = Provider::ClaudeCli(ClaudeCli { binary, model: "sonnet".into(), path_env });
    let index = scan(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))).unwrap();
    eprintln!("{} indexed files", index.files.len());

    for n in [1, 4] {
        let started = Instant::now();
        let slices = explore::slices(&index, n);
        let text = if slices.len() <= 1 {
            provider.stream(&prompt::brief_request(&index, &provider), &mut |_: Chunk| {}).await.unwrap()
        } else {
            let mut explorers = Vec::new();
            for (k, slice) in slices.iter().enumerate() {
                let req = prompt::explorer_request(&index, &provider, slice, k + 1, slices.len());
                let provider = &provider;
                explorers.push(async move {
                    let at = Instant::now();
                    let result = provider.stream(&req, &mut |_: Chunk| {}).await;
                    (at.elapsed(), result)
                });
            }
            let done = agents::run_limited(slices.len(), explorers).await;
            for (slice, (took, result)) in slices.iter().zip(&done) {
                eprintln!(
                    "  explorer {:?}: {} ms, {} words",
                    slice.prefixes,
                    took.as_millis(),
                    result.as_ref().map_or(0, |t| t.split_whitespace().count())
                );
            }
            let (notes, unmapped) = explore::collect_notes(&slices, done.into_iter().map(|(_, r)| r).collect()).unwrap();
            let merging = Instant::now();
            let text = provider
                .stream(&prompt::merge_request(&index, &provider, &notes, &unmapped), &mut |_: Chunk| {})
                .await
                .unwrap();
            eprintln!("  merge: {} ms, unmapped={unmapped:?}", merging.elapsed().as_millis());
            text
        };
        let missing: Vec<_> = SECTIONS.iter().filter(|s| !text.contains(&format!("## {s}"))).collect();
        eprintln!(
            "explorers={n}  total {} ms  words={}  missing={missing:?}",
            started.elapsed().as_millis(),
            text.split_whitespace().count()
        );
    }
}
```

- [ ] **Step 2: Run it**

Run: `cargo test -p livebrainstorm-core --test brief_live -- --ignored --nocapture 2>&1 | grep -E "indexed files|explorer|merge:|explorers=|skipped|test result|panicked"`
Expected: an indexed-file count of 40 or more; one `explorers=1` line; four `explorer [...]` lines, a `merge:` line and an `explorers=4` line; then `test result: ok`.

- [ ] **Step 3: Decide the default by this rule**

1. **Faster by at least 20%, nothing missing:** if the 4-explorer total is at least 20% below the 1-explorer total and `missing=[]` for both, keep `parallel_brief: true`.
2. **Too close to call:** if the difference is under 20% either way, run Step 2 once more and apply the rule to the sum of both runs.
3. **Otherwise, turn it off:**
   - In `settings.rs`, change the `AgentSettings::default()` body to:

     ```rust
             // Parallel notes and briefs are opt-in: neither measured faster
             // than one agent with Claude Code (see the spec's measurements).
             Self { max_agents: 4, parallel_brief: false, parallel_notes: false }
     ```

   - In `brief_agents_follow_the_settings`, change the first assertion to `assert_eq!(s.effective_brief_agents(), 1, "parallel briefs are opt-in");`, and set `s.agents.parallel_brief = true;` before the `max_agents = 6` line.
   - In `files_from_before_agents_get_defaults`, expect `parallel_brief: false` and `"parallelBrief": false`.
   - In `README.md`, change the appended sentence to say parallel briefs are off by default and are enabled with `agents.parallelBrief: true`.
   - Re-run `cargo test -p livebrainstorm-core --lib`.

Record which branch of the rule applied in Step 4.

- [ ] **Step 4: Record the measurements** (append to the spec, using the real numbers)

```markdown
## Measurements (phase 4, 2026-10-14)

`cargo test -p livebrainstorm-core --test brief_live -- --ignored --nocapture`, Claude Code `sonnet`, this repo (<N> indexed files):

| Explorers | Total | Words | Missing sections | Notes |
|---|---|---|---|---|
| 1 | <ms> | <words> | <…> | today's single explorer |
| 4 | <ms> | <words> | <…> | explorers <ms each>, merge <ms>, unmapped <…> |

- Default: <kept on | turned off>, because <rule branch and the numbers>.
```

Replace every `<…>` with the measured values before committing.

- [ ] **Step 5: Note the live test in AGENTS.md**

Under "### Jev (TypeSafe)", after the parallel-notes live-test bullet, add:

```markdown
- Parallel-brief live test (Claude Code sonnet, several minutes, uses the subscription; 1 explorer vs 4 plus merge): `cargo test -p livebrainstorm-core --test brief_live -- --ignored --nocapture`.
```

- [ ] **Step 6: Final verification and commit**

Run: `cargo clippy --workspace --all-targets && cargo test -p livebrainstorm-core && npm test && npm run typecheck`
Expected: no warnings; all tests pass.

```bash
git add crates/livebrainstorm-core docs/superpowers/specs AGENTS.md README.md
git commit -m "test: live 1 vs 4 explorer brief measurement and default decision" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 7: End-to-end check (the user, in the running app)**

Ask the user to restart `npm run tauri dev`, open this repo, and click **Rebuild brief** in the drawer.
1. **If parallel briefs are on:** activity lines read "Agent 1 · Read …", "Agent 3 · Grep …", and the brief text streams in once the merge starts. The "Written by" line ends with "· N agents".
2. **If they are off:** it behaves exactly as before.
