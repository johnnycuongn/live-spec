# Live Spec: rename, section catalogue and per-tab sections — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn LiveBrainstorm into Live Spec: a five-section catalogue of spec-review notes that each tab picks from, templates that carry a section set, a Copy spec button, and a full rename down to the bundle identifier with a one-time data and keychain migration.

**Architecture:** A new core module `sections.rs` is the single source of truth for section ids, headings, guide text and colours; the prompt builder, lane grouping, both Markdown parsers and the fact-check look sections up there instead of in a hard-coded four. Each tab file stores its chosen ids, the webview sends them with every `analyze` call, and the `TabSession` folds them into the context key only when they differ from the default. The rename is the last task so every earlier task lands against stable names, and a core `migrate.rs` moves the old app folder on first launch.

**Tech Stack:** Rust (Tauri v2 shell, `livebrainstorm-core` → `livespec-core`), React 19 + TypeScript + Vite 7, Tailwind v4, vitest, cargo test.

**Spec:** `docs/superpowers/specs/2026-10-15-live-spec-rename-and-sections-design.md`

## Global Constraints

- Node 20.19+ or 22.12+ to run the app; typecheck and vitest run on older Node.
- No `cargo fmt` across the repo; hand-wrap Rust at about 120 columns and match surrounding style. No ESLint or Prettier.
- Logic that needs tests lives in the core crate; `src-tauri` has no tests.
- Cross-language contracts are kept in sync by hand: `sections.rs` ↔ `src/lib/sections.ts` (ids, order, alias table, default set), `Tab` / `Template` ↔ `ipc.ts`.
- Every tab id from the webview passes `check_id` in the command and `validate` in the store before use.
- `Precondition` messages are shown verbatim and written as an instruction to the user.
- Model output renders only through `MiniMarkdown`; never `innerHTML`.
- Identifier after the rename: `app.livespec.desktop`. Old identifier: `app.livebrainstorm.desktop`. The migration never deletes old data and never stops the launch.
- Commit after every task with the attribution line `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Run `cargo clippy --workspace --all-targets` before the final commit; it must stay warning-free.

## Review Focus

1. **A model writes the UI title as the heading** (`## Conflicts with code`, `### Questions for you`): both parsers must open the section from the first word. Pinned in Task 2 (`section_body_matches_the_ui_parser`) and Task 3 (`parseSections` loose formats).
2. **A tab file saved with ids that no longer exist** (hand edit, future catalogue change): `resolve` drops them and falls back to the default set; `set_sections` refuses a list that names nothing. Pinned in Task 1 and Task 4.
3. **Toggling a section while notes are streaming**: the old run's events must be dropped and a new run scheduled without the gate. Pinned in Task 6 (`setSections during a run supersedes it`).
4. **The new app folder exists but is empty at first launch** (a plugin created it before `setup`): the migration must still move the old folder in. Pinned in Task 10 (`an empty new folder is treated as missing`).
5. **Fewer sections than lanes** (two sections, four agents): no empty lane may be spawned. Pinned in Task 2 (`section_groups_never_outnumber_the_sections`).

---

### Task 1: The section catalogue in core

**Files:**
- Create: `crates/livebrainstorm-core/src/sections.rs`
- Modify: `crates/livebrainstorm-core/src/lib.rs` (add `pub mod sections;`)

**Interfaces:**
- Produces:
  - `pub struct SectionSpec { pub id: &'static str, pub heading: &'static str, pub guide: &'static str, pub checkable: bool }`
  - `pub const CATALOGUE: [SectionSpec; 5]` in order intent, conflicts, gaps, ambiguities, questions
  - `pub const DEFAULT_SECTIONS: [&str; 5]`
  - `pub fn find(id: &str) -> Option<&'static SectionSpec>`
  - `pub fn normalize(ids: &[String]) -> Vec<String>` (known ids, catalogue order, deduplicated; empty stays empty)
  - `pub fn resolve(ids: &[String]) -> Vec<&'static SectionSpec>` (the specs for `normalize(ids)`; the whole catalogue when nothing is known)
  - `pub fn heading_id(word: &str) -> Option<&'static str>` (exact-word alias table)

- [ ] **Step 1: Write the failing tests**

Create `crates/livebrainstorm-core/src/sections.rs` with only the test module for now:

```rust
//! The catalogue of note sections. A tab picks a subset; the prompt, the lane
//! split, both Markdown parsers and the fact-check key on it. Mirrors
//! `SECTIONS` in `src/lib/sections.ts`: ids, order, the default set and the
//! heading alias table must match.

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(specs: &[&SectionSpec]) -> Vec<&str> {
        specs.iter().map(|s| s.id).collect()
    }

    #[test]
    fn the_catalogue_is_the_default_set_in_order() {
        assert_eq!(ids(&CATALOGUE.iter().collect::<Vec<_>>()), DEFAULT_SECTIONS);
        assert_eq!(DEFAULT_SECTIONS, ["intent", "conflicts", "gaps", "ambiguities", "questions"]);
        assert_eq!(CATALOGUE.iter().filter(|s| s.checkable).map(|s| s.id).collect::<Vec<_>>(), ["conflicts"]);
    }

    #[test]
    fn resolve_keeps_known_ids_in_catalogue_order_once_each() {
        let picked = resolve(&["questions".into(), "intent".into(), "questions".into(), "bogus".into()]);
        assert_eq!(ids(&picked), ["intent", "questions"]);
        assert_eq!(normalize(&["questions".into(), "intent".into(), "questions".into()]), ["intent", "questions"]);
    }

    #[test]
    fn nothing_known_means_the_default_set() {
        assert_eq!(ids(&resolve(&[])), DEFAULT_SECTIONS);
        assert_eq!(ids(&resolve(&["bogus".into()])), DEFAULT_SECTIONS, "a hand-edited file falls back, never to zero sections");
        assert!(normalize(&[]).is_empty(), "stored as empty: the default set");
        assert!(normalize(&["bogus".into()]).is_empty());
    }

    #[test]
    fn heading_words_map_exactly_including_the_old_builds_names() {
        for (word, id) in [
            ("Intent", "intent"),
            ("conflicts", "conflicts"),
            ("Conflict", "conflicts"),
            ("Faults", "conflicts"),
            ("fault", "conflicts"),
            ("Gaps", "gaps"),
            ("gap", "gaps"),
            ("Ambiguities", "ambiguities"),
            ("ambiguity", "ambiguities"),
            ("Questions", "questions"),
            ("question", "questions"),
        ] {
            assert_eq!(heading_id(word), Some(id), "{word}");
        }
        for word in ["Intentional", "Gapless", "Directions", "Summary", ""] {
            assert_eq!(heading_id(word), None, "{word}");
        }
        assert_eq!(find("gaps").map(|s| s.heading), Some("Gaps"));
        assert!(find("directions").is_none(), "the old build's section is not in the catalogue");
    }
}
```

Add `pub mod sections;` to `crates/livebrainstorm-core/src/lib.rs` between `pub mod project;` and `pub mod prompt;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core sections::tests`
Expected: compile error, `cannot find value CATALOGUE in this scope` (and the other names).

- [ ] **Step 3: Write the catalogue**

Above the test module in `sections.rs`:

```rust
#[derive(Debug, PartialEq, Eq)]
pub struct SectionSpec {
    /// Stable id: stored in tab files and templates, sent over IPC.
    pub id: &'static str,
    /// The heading the model writes (`## Conflicts`).
    pub heading: &'static str,
    /// What the section asks for, in the system prompt.
    pub guide: &'static str,
    /// Fact-checked against the code when present.
    pub checkable: bool,
}

/// Every section, in the order the notes show them. A tab picks a subset.
pub const CATALOGUE: [SectionSpec; 5] = [
    SectionSpec {
        id: "intent",
        heading: "Intent",
        guide: "What the spec asks for, for whom, and what done looks like, in one or two bullets. \
If the spec doesn't say, name what it fails to say.",
        checkable: false,
    },
    SectionSpec {
        id: "conflicts",
        heading: "Conflicts",
        guide: "Where the spec contradicts how the project works, re-does something that exists, or assumes wrongly. \
Cite file paths when the project context supports it. If nothing is worth raising, write \"- Nothing so far.\"",
        checkable: true,
    },
    SectionSpec {
        id: "gaps",
        heading: "Gaps",
        guide: "What an agent implementing this spec would need but can't find in it: acceptance criteria it could turn \
into tests, non-goals, error and edge cases, data or state the project doesn't have, commands to run. \
Name the gap, not a fix.",
        checkable: false,
    },
    SectionSpec {
        id: "ambiguities",
        heading: "Ambiguities",
        guide: "Phrases an agent would have to guess at. Quote the phrase, give the two readings, and propose precise wording.",
        checkable: false,
    },
    SectionSpec {
        id: "questions",
        heading: "Questions",
        guide: "One to three questions whose answers would most change the spec.",
        checkable: false,
    },
];

/// What a tab with no stored choice gets: the whole catalogue.
pub const DEFAULT_SECTIONS: [&str; 5] = ["intent", "conflicts", "gaps", "ambiguities", "questions"];

pub fn find(id: &str) -> Option<&'static SectionSpec> {
    CATALOGUE.iter().find(|s| s.id == id)
}

/// Known ids in catalogue order, each once. Empty stays empty: the default set.
pub fn normalize(ids: &[String]) -> Vec<String> {
    CATALOGUE.iter().filter(|s| ids.iter().any(|id| id == s.id)).map(|s| s.id.to_owned()).collect()
}

/// The specs a tab asked for, normalized. Nothing known (an empty list, or
/// ids a later build dropped) means the default set, never zero sections.
pub fn resolve(ids: &[String]) -> Vec<&'static SectionSpec> {
    let known: Vec<&'static SectionSpec> = CATALOGUE.iter().filter(|s| ids.iter().any(|id| id == s.id)).collect();
    if known.is_empty() {
        CATALOGUE.iter().collect()
    } else {
        known
    }
}

/// Which section a heading's first word opens, as an exact word (singular or
/// plural). "Faults" is the old build's name for Conflicts, so saved notes
/// still parse. Mirrors `keyFor` in `src/lib/sections.ts`.
pub fn heading_id(word: &str) -> Option<&'static str> {
    match word.to_ascii_lowercase().as_str() {
        "intent" => Some("intent"),
        "conflict" | "conflicts" | "fault" | "faults" => Some("conflicts"),
        "gap" | "gaps" => Some("gaps"),
        "ambiguity" | "ambiguities" => Some("ambiguities"),
        "question" | "questions" => Some("questions"),
        _ => None,
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p livebrainstorm-core sections::tests`
Expected: `test result: ok. 4 passed`

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/sections.rs crates/livebrainstorm-core/src/lib.rs
git commit -m "feat(core): section catalogue with ids, headings, guides and the heading alias table

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: The prompt, lane grouping and section parsing key on the catalogue

**Files:**
- Modify: `crates/livebrainstorm-core/src/prompt.rs` (constants at the top, `section_groups`, `analysis_system`, `analysis_request`, `heading_key`, `section_body`, tests)
- Modify: `crates/livebrainstorm-core/src/analysis.rs:86-118` (`merge_lanes`) and its tests at `:230-280`
- Modify: `crates/livebrainstorm-core/tests/notes_live.rs:27-53`

**Interfaces:**
- Consumes: `sections::{SectionSpec, CATALOGUE, resolve, find, heading_id}` from Task 1.
- Produces:
  - `pub fn section_groups(n: usize, chosen: &[&'static SectionSpec]) -> Vec<Vec<&'static SectionSpec>>`
  - `pub fn analysis_system(sections: &[&SectionSpec], instructions: &str) -> String`
  - `pub fn analysis_request(draft, index, brief, snippets, provider, sections: &[&SectionSpec], instructions) -> ChatRequest`
  - `pub fn section_body(markdown: &str, id: &str) -> Option<String>` (takes the **id**, e.g. `"conflicts"`)
  - `pub fn merge_lanes(groups: &[Vec<&SectionSpec>], results: Vec<Result<String>>) -> Result<(String, Option<String>)>`
  - `pub use crate::sections::{SectionSpec, CATALOGUE, DEFAULT_SECTIONS, resolve};` re-exported from `prompt`
  - `prompt::SECTIONS` and `SECTION_GUIDE` are **removed**.

- [ ] **Step 1: Rewrite the prompt tests for the catalogue**

In `prompt.rs` tests, replace `TODAYS_PROMPT` and the tests `all_four_sections_is_todays_prompt`, `a_lane_prompt_has_only_its_own_headings`, `blank_instructions_leave_the_prompt_unchanged`, `sections_are_grouped_per_the_spec`, `section_body_matches_the_ui_parser`, `analysis_without_project_still_works`, and the `every_lane_gets_the_same_clipped_instructions` body, with:

```rust
    fn all() -> Vec<&'static SectionSpec> {
        resolve(&[])
    }

    fn pick(ids: &[&str]) -> Vec<&'static SectionSpec> {
        ids.iter().map(|id| crate::sections::find(id).expect("catalogue id")).collect()
    }

    /// The single-lane prompt with every section and no instructions: pinned byte for byte.
    const BASELINE_PROMPT: &str = "\
You are Live Spec, a reviewer sitting beside a software engineer who is writing a spec that an AI coding agent \
will implement against the project described in <project>. The spec is unfinished and typed in real time. \
Your job is to make it precise enough that the agent needs no further discovery or guessing: grounded in how \
the code actually works, unambiguous, complete on what matters, and no longer than it needs to be. \
Never rewrite the spec.

Respond in Markdown with exactly these five sections, in this order, and nothing else:
## Intent
What the spec asks for, for whom, and what done looks like, in one or two bullets. If the spec doesn't say, name what it fails to say.
## Conflicts
Where the spec contradicts how the project works, re-does something that exists, or assumes wrongly. Cite file paths when the project context supports it. If nothing is worth raising, write \"- Nothing so far.\"
## Gaps
What an agent implementing this spec would need but can't find in it: acceptance criteria it could turn into tests, non-goals, error and edge cases, data or state the project doesn't have, commands to run. Name the gap, not a fix.
## Ambiguities
Phrases an agent would have to guess at. Quote the phrase, give the two readings, and propose precise wording.
## Questions
One to three questions whose answers would most change the spec.

Rules: bullets only, at most 3 per section, each under 35 words. Ground claims in the project context; \
when the context doesn't cover something, say so instead of guessing. Never restate the spec; quote it only to \
point at a phrase. Do not use tools.";

    #[test]
    fn all_five_sections_is_the_single_lane_prompt() {
        assert_eq!(analysis_system(&all(), ""), BASELINE_PROMPT);
    }

    #[test]
    fn a_subset_prompt_has_only_its_own_headings() {
        let one = analysis_system(&pick(&["conflicts"]), "");
        assert!(one.contains("## Conflicts") && one.contains("exactly this one section"));
        for other in ["## Intent", "## Gaps", "## Ambiguities", "## Questions"] {
            assert!(!one.contains(other), "{other}");
        }
        assert!(one.contains("Write only the sections above"), "stops the model adding the rest");
        let pair = analysis_system(&pick(&["gaps", "questions"]), "");
        assert!(pair.contains("exactly these two sections") && pair.contains("## Gaps") && pair.contains("## Questions"));
        assert!(!pair.contains("## Conflicts"));
        let three = analysis_system(&pick(&["intent", "gaps", "questions"]), "");
        assert!(three.contains("exactly these three sections"));
        assert!(!analysis_system(&all(), "").contains("Write only the sections above"));
    }

    #[test]
    fn blank_instructions_leave_the_prompt_unchanged() {
        assert_eq!(analysis_system(&all(), ""), BASELINE_PROMPT);
        assert_eq!(analysis_system(&all(), "  \n\t "), BASELINE_PROMPT);
    }

    #[test]
    fn section_groups_split_the_chosen_list_contiguously() {
        let names = |n, ids: &[&str]| {
            section_groups(n, &pick(ids)).iter().map(|g| g.iter().map(|s| s.id).collect::<Vec<_>>().join("+")).collect::<Vec<_>>()
        };
        let five = ["intent", "conflicts", "gaps", "ambiguities", "questions"];
        assert_eq!(names(1, &five), ["intent+conflicts+gaps+ambiguities+questions"]);
        assert_eq!(names(2, &five), ["intent+conflicts+gaps", "ambiguities+questions"]);
        assert_eq!(names(3, &five), ["intent+conflicts", "gaps+ambiguities", "questions"]);
        assert_eq!(names(4, &five), ["intent+conflicts", "gaps", "ambiguities", "questions"]);
        assert_eq!(names(0, &five), names(1, &five));
        assert_eq!(names(2, &["conflicts", "gaps", "questions"]), ["conflicts+gaps", "questions"]);
    }

    #[test]
    fn section_groups_never_outnumber_the_sections() {
        assert_eq!(section_groups(4, &pick(&["conflicts", "questions"])).len(), 2);
        assert_eq!(section_groups(8, &all()).len(), 5);
        assert_eq!(section_groups(4, &pick(&["gaps"])).len(), 1);
        assert!(section_groups(4, &[]).is_empty(), "no sections, no lanes");
    }

    #[test]
    fn section_body_matches_the_ui_parser() {
        let md = "## Intent\n- add caching\n## Conflicts\n- `cache.ts` exists\n- second\n## Gaps\n- tests";
        assert_eq!(section_body(md, "conflicts").as_deref(), Some("- `cache.ts` exists\n- second"));
        assert_eq!(section_body("**Conflicts:**\n- x\n### Questions for you\n- y", "conflicts").as_deref(), Some("- x"));
        assert_eq!(section_body("## Conflicts with code\n- x", "conflicts").as_deref(), Some("- x"), "the UI title as a heading");
        assert_eq!(section_body("## Faults\n- old notes", "conflicts").as_deref(), Some("- old notes"), "the old build's name");
        assert_eq!(section_body("Sure, here you go\n## Intent\n- a", "conflicts"), None);
        assert_eq!(section_body("## Intent\n- **Intentional** duplication\n- b", "intent").as_deref(), Some("- **Intentional** duplication\n- b"));
    }

    #[test]
    fn analysis_without_project_still_works() {
        let req = analysis_request("add caching to search", None, None, &[], &local(), &all(), "");
        assert!(req.user.contains("No project selected") && req.cwd.is_none());
        assert!(req.user.contains("<spec>\nadd caching to search\n</spec>"));
        for s in CATALOGUE.iter() {
            assert!(req.system.contains(&format!("## {}", s.heading)));
        }
    }
```

In `every_lane_gets_the_same_clipped_instructions`, change the loop and the local call to:

```rust
        for sections in section_groups(4, &all()) {
            let req = analysis_request("add caching to search", None, None, &[], &claude(), &sections, &long);
            assert_eq!(kept(&req.system), MAX_INSTRUCTION_CHARS, "{}", sections.iter().map(|s| s.id).collect::<Vec<_>>().join("+"));
        }
        let req = analysis_request("add caching to search", None, None, &[], &local(), &all(), &long);
```

In `instructions_sit_between_the_sections_and_the_rules` and `instructions_cannot_close_their_own_block`, replace `&SECTIONS` with `&all()` and `&["Faults"]` with `&pick(&["conflicts"])`. In `instructions_sit_between_the_sections_and_the_rules`, change the assertion `lane.find("written separately")` to `lane.find("Write only the sections above")`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core prompt::tests`
Expected: compile errors (`resolve`, `SectionSpec` not in scope; `analysis_system` expects `&[&str]`).

- [ ] **Step 3: Rewrite the top of `prompt.rs`**

Replace everything from `/// Headings the UI splits the analysis on` through the end of `analysis_system` with:

```rust
pub use crate::sections::{resolve, SectionSpec, CATALOGUE, DEFAULT_SECTIONS};

const ANALYSIS_INTRO: &str = "\
You are Live Spec, a reviewer sitting beside a software engineer who is writing a spec that an AI coding agent \
will implement against the project described in <project>. The spec is unfinished and typed in real time. \
Your job is to make it precise enough that the agent needs no further discovery or guessing: grounded in how \
the code actually works, unambiguous, complete on what matters, and no longer than it needs to be. \
Never rewrite the spec.

";

const ANALYSIS_RULES: &str = "

Rules: bullets only, at most 3 per section, each under 35 words. Ground claims in the project context; \
when the context doesn't cover something, say so instead of guessing. Never restate the spec; quote it only to \
point at a phrase. Do not use tools.";

/// Session instructions longer than this are clipped (small local models keep less).
pub const MAX_INSTRUCTION_CHARS: usize = 4_000;

/// Splits the chosen sections into at most `n` contiguous lanes of near-equal
/// size (earlier lanes take the remainder). Never more lanes than sections.
pub fn section_groups(n: usize, chosen: &[&'static SectionSpec]) -> Vec<Vec<&'static SectionSpec>> {
    if chosen.is_empty() {
        return Vec::new();
    }
    let k = n.clamp(1, chosen.len());
    let (base, extra) = (chosen.len() / k, chosen.len() % k);
    let mut groups = Vec::with_capacity(k);
    let mut start = 0;
    for i in 0..k {
        let size = base + usize::from(i < extra);
        groups.push(chosen[start..start + size].to_vec());
        start += size;
    }
    groups
}

/// The system prompt for an agent writing `sections`, steered by the user's
/// session instructions. With the whole catalogue and no instructions it is
/// exactly the single-lane prompt.
pub fn analysis_system(sections: &[&SectionSpec], instructions: &str) -> String {
    const COUNT: [&str; 4] = ["two", "three", "four", "five"];
    let lead = match sections.len() {
        0 | 1 => "exactly this one section and nothing else".to_owned(),
        k => format!("exactly these {} sections, in this order, and nothing else", COUNT[(k - 2).min(3)]),
    };
    let mut prompt = format!("{ANALYSIS_INTRO}Respond in Markdown with {lead}:");
    for s in sections {
        prompt.push_str(&format!("\n## {}\n{}", s.heading, s.guide));
    }
    // A lane of several, or a tab that picked a subset: either way the rest is not this call's to write.
    if sections.len() < CATALOGUE.len() {
        prompt.push_str("\n\nWrite only the sections above, even if more seem useful.");
    }
    // Before the rules, which win: instructions steer what the notes say, never their shape.
    let instructions = instructions.trim().replace("</session_instructions", "<\\/session_instructions");
    if !instructions.is_empty() {
        prompt.push_str(&format!(
            "\n\n<session_instructions>\n{instructions}\n</session_instructions>\n\
             These instructions come from the person drafting. Follow them, but the headings above, \
             their order, and the rules below always win."
        ));
    }
    prompt.push_str(ANALYSIS_RULES);
    prompt
}
```

In `analysis_request`, change the parameter to `sections: &[&SectionSpec]`, the no-project line to `project.push_str("<project>No project selected: review the spec on its own.</project>\n\n");` and the user message to `format!("{project}<spec>\n{}\n</spec>", shape_draft(draft))`.

Replace `heading_key` and `section_body` with:

```rust
/// Which catalogue section a heading line opens, as tolerant as
/// `src/lib/sections.ts`: "## Conflicts", "### Questions for you", "**Faults:**".
fn heading_key(line: &str) -> Option<&'static str> {
    let t = line.trim_start();
    let rest = match t.strip_prefix("**") {
        Some(rest) => rest,
        None => {
            let hashes = t.len() - t.trim_start_matches('#').len();
            if !(1..=4).contains(&hashes) {
                return None;
            }
            &t[hashes..]
        }
    };
    let word = rest.trim_start().split(|c: char| !c.is_ascii_alphabetic()).next()?;
    crate::sections::heading_id(word)
}

/// The body under one section, by id (e.g. "conflicts"), if present. Mirrors
/// `parseSections` in `src/lib/sections.ts`; keep the two in sync.
pub fn section_body(markdown: &str, id: &str) -> Option<String> {
    let mut body: Option<String> = None;
    let mut current = None;
    for line in markdown.lines() {
        if let Some(key) = heading_key(line) {
            current = Some(key);
            if key == id {
                body.get_or_insert_with(String::new);
            }
            continue;
        }
        if current == Some(id) {
            if let Some(b) = body.as_mut() {
                b.push_str(line);
                b.push('\n');
            }
        }
    }
    body.map(|b| b.trim().to_owned())
}
```

Update the module doc comment's first line to `//! Prompt construction. Everything the model sees is built here, so tuning` / `//! Live Spec's behaviour is a one-file change.`

- [ ] **Step 4: Update `merge_lanes` and its tests**

In `analysis.rs`, replace `merge_lanes` with:

```rust
pub fn merge_lanes(groups: &[Vec<&SectionSpec>], results: Vec<crate::Result<String>>) -> crate::Result<(String, Option<String>)> {
    let (mut parts, mut missing, mut first_error) = (Vec::new(), Vec::new(), None);
    for (sections, result) in groups.iter().zip(results) {
        let text = match result {
            Ok(text) => text,
            Err(e) => {
                missing.extend(sections.iter().map(|s| s.heading));
                first_error.get_or_insert(e);
                continue;
            }
        };
        for s in sections {
            match prompt::section_body(&text, s.id).filter(|body| !body.is_empty()) {
                Some(body) => parts.push(format!("## {}\n{body}", s.heading)),
                None => {
                    missing.push(s.heading);
                    first_error.get_or_insert_with(|| Error::Provider(format!("the reply had no {} section", s.heading)));
                }
            }
        }
    }
    if parts.is_empty() {
        return Err(first_error.unwrap_or_else(|| Error::Provider("No notes were written.".into())));
    }
    let notice = first_error.filter(|_| !missing.is_empty()).map(|e| format!("Couldn't write {}: {e}", missing.join(" and ")));
    Ok((parts.join("\n"), notice))
}
```

Add `use crate::sections::SectionSpec;` to the imports. In the tests module add:

```rust
    fn pick(ids: &[&str]) -> Vec<&'static SectionSpec> {
        ids.iter().map(|id| crate::sections::find(id).expect("catalogue id")).collect()
    }
```

and rewrite the five `merge_lanes` tests so each `groups` uses `pick`, and the markdown uses the new headings. For example the first becomes:

```rust
    #[test]
    fn lanes_merge_in_lane_order() {
        let groups = vec![pick(&["intent", "conflicts"]), pick(&["gaps", "questions"])];
        let merged = merge_lanes(&groups, vec![Ok("## Intent\n- i\n## Conflicts\n- f\n".into()), Ok("## Gaps\n- d\n## Questions\n- q".into())]);
        let (text, notice) = merged.unwrap();
        assert_eq!(text, "## Intent\n- i\n## Conflicts\n- f\n## Gaps\n- d\n## Questions\n- q");
        assert!(notice.is_none());
    }
```

Apply the same substitutions (`"Faults"` → `pick(&["conflicts"])` / `## Conflicts`, `"Directions"` → `gaps` / `## Gaps`) to `failed_lanes_become_one_notice_naming_every_missing_section`, `a_lane_keeps_only_its_own_sections`, and the two that follow; the expected notice text becomes `"Couldn't write Conflicts: the reply had no Conflicts section"`.

- [ ] **Step 5: Update the live test**

In `crates/livebrainstorm-core/tests/notes_live.rs`, replace line 27 with
`let chosen = prompt::resolve(&[]);` and `let groups = prompt::section_groups(n, &chosen);`, and line 53 with:

```rust
        let missing: Vec<_> = chosen.iter().filter(|s| prompt::section_body(&text, s.id).is_none()).map(|s| s.heading).collect();
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p livebrainstorm-core prompt:: analysis::` then `cargo test -p livebrainstorm-core --no-run --tests`
Expected: all prompt and analysis tests pass; the ignored live tests compile. `src-tauri` will not compile yet (fixed in Task 5), so do not run `cargo check --workspace` here.

- [ ] **Step 7: Commit**

```bash
git add crates/livebrainstorm-core/src/prompt.rs crates/livebrainstorm-core/src/analysis.rs crates/livebrainstorm-core/tests/notes_live.rs
git commit -m "feat(core): prompt, lane grouping and section parsing key on the catalogue; spec-review intro and rules

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: The frontend section table and parser

**Files:**
- Modify: `src/lib/sections.ts` (whole file)
- Modify: `src/lib/sections.test.ts` (whole file)

**Interfaces:**
- Produces:
  - `type SectionKey = "intent" | "conflicts" | "gaps" | "ambiguities" | "questions" | "directions"`
  - `interface SectionDef { key: SectionKey; title: string; dot: string; legacy?: boolean }`
  - `const SECTIONS: SectionDef[]` (six: the five, then legacy `directions`)
  - `const CATALOGUE: SectionDef[]` (the five pickable ones, in order)
  - `const DEFAULT_SECTIONS: string[]` (the five ids)
  - `normalizeSections(ids: string[]): string[]` (known, catalogue order, deduplicated; empty stays empty)
  - `effectiveSections(ids: string[]): string[]` (`DEFAULT_SECTIONS` when empty)
  - `isDefaultSections(ids: string[]): boolean`
  - `toggleSection(chosen: string[], key: string, allowEmpty = false): string[]`
  - `sectionsSentence(ids: string[]): string` ("Intent, Conflicts with code and Questions for you.")
  - `parseSections(markdown): Partial<Record<SectionKey, string>>` (unchanged signature)

- [ ] **Step 1: Write the failing tests**

Replace `src/lib/sections.test.ts` with:

```ts
import { describe, expect, it } from "vitest";
import {
  CATALOGUE,
  DEFAULT_SECTIONS,
  SECTIONS,
  effectiveSections,
  isDefaultSections,
  normalizeSections,
  parseSections,
  sectionsSentence,
  toggleSection,
} from "./sections";

describe("the catalogue", () => {
  it("mirrors sections.rs: ids, order and the default set", () => {
    expect(CATALOGUE.map((s) => s.key)).toEqual(["intent", "conflicts", "gaps", "ambiguities", "questions"]);
    expect(DEFAULT_SECTIONS).toEqual(CATALOGUE.map((s) => s.key));
    expect(SECTIONS.filter((s) => s.legacy).map((s) => s.key)).toEqual(["directions"]);
  });

  it("normalizes to catalogue order, known ids only, once each; empty stays empty", () => {
    expect(normalizeSections(["questions", "intent", "questions", "bogus"])).toEqual(["intent", "questions"]);
    expect(normalizeSections([])).toEqual([]);
    expect(normalizeSections(["directions"])).toEqual([]);
    expect(effectiveSections([])).toEqual(DEFAULT_SECTIONS);
    expect(effectiveSections(["gaps"])).toEqual(["gaps"]);
  });

  it("the default set is empty or the whole catalogue in order", () => {
    expect(isDefaultSections([])).toBe(true);
    expect(isDefaultSections([...DEFAULT_SECTIONS])).toBe(true);
    expect(isDefaultSections(["intent", "conflicts", "gaps", "ambiguities"])).toBe(false);
    expect(isDefaultSections(["questions", "intent", "conflicts", "gaps", "ambiguities"])).toBe(false);
  });

  it("toggles keep catalogue order and never empty a tab's list", () => {
    expect(toggleSection(["intent", "gaps"], "conflicts")).toEqual(["intent", "conflicts", "gaps"]);
    expect(toggleSection(["intent", "gaps"], "gaps")).toEqual(["intent"]);
    expect(toggleSection(["gaps"], "gaps")).toEqual(["gaps"]);
    expect(toggleSection(["gaps"], "gaps", true)).toEqual([]); // a template may leave the tab's sections alone
    expect(toggleSection([], "gaps")).toEqual(["intent", "conflicts", "ambiguities", "questions"]); // empty means the default set
  });

  it("writes the empty-state sentence from the chosen titles", () => {
    expect(sectionsSentence(["intent", "conflicts", "questions"])).toBe("Intent, Conflicts with code and Questions for you.");
    expect(sectionsSentence(["gaps"])).toBe("Gaps.");
    expect(sectionsSentence([])).toBe("Intent, Conflicts with code, Gaps, Ambiguities and Questions for you.");
  });
});

describe("parseSections", () => {
  it("splits canonical headings", () => {
    const md = "## Intent\n- add caching\n## Conflicts\n- `cache.ts` exists\n## Gaps\n- tests\n## Ambiguities\n- 'fast'\n## Questions\n- TTL?";
    expect(parseSections(md)).toEqual({
      intent: "- add caching",
      conflicts: "- `cache.ts` exists",
      gaps: "- tests",
      ambiguities: "- 'fast'",
      questions: "- TTL?",
    });
  });

  it("handles partial streams and loose formats", () => {
    expect(parseSections("## Intent\n- half")).toEqual({ intent: "- half" });
    expect(parseSections("**Conflicts:**\n- x\n### Questions for you\n- y")).toEqual({ conflicts: "- x", questions: "- y" });
    expect(parseSections("## Conflicts with code\n- x\n## Ambiguity\n- y")).toEqual({ conflicts: "- x", ambiguities: "- y" });
  });

  it("reads the old build's notes: Faults are conflicts, Directions stay as a legacy section", () => {
    expect(parseSections("## Faults\n- f\n## Directions\n- d")).toEqual({ conflicts: "- f", directions: "- d" });
  });

  it("ignores text before the first heading and bold words inside bullets", () => {
    expect(parseSections("Sure, here you go\n## Intent\n- a")).toEqual({ intent: "- a" });
    expect(parseSections("## Intent\n- a\n**Intentional** duplication\n- b")).toEqual({ intent: "- a\n**Intentional** duplication\n- b" });
  });
});
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx vitest run src/lib/sections.test.ts`
Expected: FAIL, `CATALOGUE` is not exported (and the rest).

- [ ] **Step 3: Rewrite `src/lib/sections.ts`**

```ts
// The section catalogue and the streaming parser. Mirrors sections.rs in
// crates/livebrainstorm-core: ids, order, the default set and the heading
// alias table must match. Tolerant on purpose: small local models write
// "**Conflicts:**" or "### Conflicts with code".

export type SectionKey = "intent" | "conflicts" | "gaps" | "ambiguities" | "questions" | "directions";

export interface SectionDef {
  key: SectionKey;
  title: string;
  /** The eyebrow dot: red for a clash with the code, blue for something to add, amber for an open item. */
  dot: string;
  /** The old build's section: rendered when a saved analysis has it, never offered. */
  legacy?: boolean;
}

export const SECTIONS: SectionDef[] = [
  { key: "intent", title: "Intent", dot: "bg-muted" },
  { key: "conflicts", title: "Conflicts with code", dot: "bg-fault" },
  { key: "gaps", title: "Gaps", dot: "bg-accent" },
  { key: "ambiguities", title: "Ambiguities", dot: "bg-question" },
  { key: "questions", title: "Questions for you", dot: "bg-question" },
  { key: "directions", title: "Directions", dot: "bg-accent", legacy: true },
];

/** What a tab can pick, in the order the notes show them. */
export const CATALOGUE: SectionDef[] = SECTIONS.filter((s) => !s.legacy);

/** A tab with no stored choice (an empty list) gets the whole catalogue. */
export const DEFAULT_SECTIONS: string[] = CATALOGUE.map((s) => s.key);

/** Known ids in catalogue order, once each. Empty stays empty: the default set. */
export const normalizeSections = (ids: string[]): string[] => CATALOGUE.map((s) => s.key).filter((k) => ids.includes(k));

export const effectiveSections = (ids: string[]): string[] => (ids.length === 0 ? DEFAULT_SECTIONS : ids);

export const isDefaultSections = (ids: string[]): boolean =>
  ids.length === 0 || (ids.length === DEFAULT_SECTIONS.length && ids.every((id, i) => id === DEFAULT_SECTIONS[i]));

/**
 * Adds or removes `key`. A tab keeps at least one section (the last one stays);
 * `allowEmpty` is for templates, where an empty list means "leave the tab's sections alone".
 */
export function toggleSection(chosen: string[], key: string, allowEmpty = false): string[] {
  const current = effectiveSections(chosen);
  if (current.includes(key)) {
    const next = current.filter((k) => k !== key);
    return next.length === 0 && !allowEmpty ? current : next;
  }
  return normalizeSections([...current, key]);
}

/** "Intent, Conflicts with code and Questions for you." for the notes' empty state. */
export function sectionsSentence(ids: string[]): string {
  const titles = CATALOGUE.filter((s) => effectiveSections(ids).includes(s.key)).map((s) => s.title);
  if (titles.length <= 1) return `${titles.join("")}.`;
  return `${titles.slice(0, -1).join(", ")} and ${titles[titles.length - 1]}.`;
}

const HEADING = /^\s*(?:#{1,4}\s*|\*\*\s*)(intent|conflicts?|faults?|gaps?|ambiguit(?:y|ies)|questions?|directions?)\b[^\n]*$/i;

const keyFor = (word: string): SectionKey => {
  const w = word.toLowerCase();
  if (w.startsWith("intent")) return "intent";
  if (w.startsWith("conflict") || w.startsWith("fault")) return "conflicts";
  if (w.startsWith("gap")) return "gaps";
  if (w.startsWith("ambigu")) return "ambiguities";
  if (w.startsWith("direction")) return "directions";
  return "questions";
};

export function parseSections(markdown: string): Partial<Record<SectionKey, string>> {
  const out: Partial<Record<SectionKey, string>> = {};
  let current: SectionKey | null = null;
  for (const line of markdown.split("\n")) {
    const m = HEADING.exec(line);
    if (m) {
      current = keyFor(m[1]);
      out[current] = out[current] ?? "";
      continue;
    }
    if (current) out[current] += `${line}\n`;
  }
  for (const k of Object.keys(out) as SectionKey[]) out[k] = out[k]!.trim();
  return out;
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `npx vitest run src/lib/sections.test.ts`
Expected: PASS (9 tests). `npm run typecheck` will fail in `NotesPane.tsx` (`DOT` indexes the old keys) until Task 7; that is expected.

- [ ] **Step 5: Commit**

```bash
git add src/lib/sections.ts src/lib/sections.test.ts
git commit -m "feat(ui): section catalogue table, normalization and toggles; parser reads the new headings and the old build's

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Tabs store their sections

**Files:**
- Modify: `crates/livebrainstorm-core/src/sessions/mod.rs:53-67` (`Tab`), tests at `:242-291`
- Modify: `crates/livebrainstorm-core/src/sessions/store.rs:117-135` (`create_tab`), `:172-184` (after `rename_tab`), `:352` (`import_legacy`), `:533-545` (`new_tab`), tests

**Interfaces:**
- Consumes: `sections::normalize`.
- Produces:
  - `Tab.sections: Vec<String>` (`#[serde(default)]`; empty = default set)
  - `SessionStore::create_tab(&self, window: &str, root: Option<String>, sections: Vec<String>) -> Result<Tab>`
  - `SessionStore::set_sections(&self, id: &str, sections: Vec<String>) -> Result<()>`

- [ ] **Step 1: Write the failing tests**

In `sessions/mod.rs` tests, add `sections: vec!["conflicts".into()],` to the `Tab` literal in `tabs_serialize_with_the_frontends_field_names` (after `draft`), and `"sections": ["conflicts"],` to the expected JSON after `"draft": "d",`. Extend `a_tab_file_with_missing_fields_still_reads`:

```rust
    #[test]
    fn a_tab_file_with_missing_fields_still_reads() {
        let tab: Tab = serde_json::from_str(r#"{"id":"t-1","draft":"x"}"#).unwrap();
        assert_eq!((tab.draft.as_str(), tab.next_snapshot, tab.history.len()), ("x", 0, 0));
        assert!(tab.sections.is_empty(), "a tab from the old build has the default set");
    }
```

In `sessions/store.rs` tests, change every `store.create_tab(&w, root)` call to pass a third argument `vec![]` (use `grep -n "create_tab(" crates/livebrainstorm-core/src/sessions/store.rs`), then add:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core sessions::`
Expected: compile errors: no field `sections` on `Tab`, `create_tab` takes 2 arguments, no method `set_sections`.

- [ ] **Step 3: Add the field and the store methods**

In `sessions/mod.rs`, add to `Tab` after `pub draft: String,`:

```rust
    /// The catalogue ids this tab's notes cover (`sections::CATALOGUE`), in
    /// catalogue order. Empty: the default set, as every tab from before
    /// this field existed.
    pub sections: Vec<String>,
```

In `store.rs`, add `use crate::sections;` to the imports. Change `create_tab`:

```rust
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
        // ... the rest unchanged (put_tab, push, active, put_ws)
```

After `rename_tab`, add:

```rust
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
```

`new_tab` and `import_legacy` are unchanged (`..Tab::default()` leaves `sections` empty). While here, change the `Snapshot.checks_faults` doc comment in `sessions/mod.rs` to `/// The Conflicts count from the same event, so restored notes read "2 of 5 conflicts checked" as they did live. The field keeps its old name: it is a file and IPC contract.`

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p livebrainstorm-core sessions::`
Expected: all pass, including the new test.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/sessions
git commit -m "feat(core): tabs store their section choice; set_sections normalizes and refuses an empty pick

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Shell commands, IPC types and the fact-check hook

**Files:**
- Modify: `src-tauri/src/commands.rs:356-361` (`create_tab`), after `rename_tab` (`set_tab_sections`), `:496-660` (`analyze`), `:42` (doc comment)
- Modify: `src-tauri/src/lib.rs` (`generate_handler!`: add `commands::set_tab_sections`)
- Modify: `src/lib/ipc.ts:75-79` (`Template` is Task 8; here `Tab` at `:130-140`, `Snapshot` comment at `:124`, `api.createTab`, `api.analyze`, new `api.setTabSections`)
- Modify: `src/lib/checks.ts:14-18`, `src/lib/checks.test.ts:4-17`
- Modify: `src/components/SettingsDialog.tsx:492` (Jev toggle label)

**Interfaces:**
- Consumes: `sections::{resolve, SectionSpec}`, `prompt::section_groups(n, &chosen)`, `SessionStore::{create_tab, set_sections}`.
- Produces:
  - command `create_tab(window, root, sections: Vec<String>)`
  - command `set_tab_sections(tab_id, sections: Vec<String>)`
  - command `analyze(tab_id, draft, previous_draft, instructions, sections: Vec<String>, context, on_event)`
  - `ipc.ts`: `Tab.sections: string[]`; `api.createTab(window, root, sections)`, `api.setTabSections(tabId, sections)`, `api.analyze(tabId, draft, previousDraft, instructions, sections, context, onEvent)`
  - `checkSummary` says "conflict" / "conflicts"

- [ ] **Step 1: Write the failing frontend test**

In `src/lib/checks.test.ts`, change the four expectations to the new wording:

```ts
    expect(checkSummary({ count: 2, total: 3 }, 1, false)).toBe("Checking 2 of 3 conflicts…");
    expect(checkSummary({ count: 2, total: 3 }, 2, false)).toBe("2 of 3 conflicts checked");
    // ...
    expect(checkSummary({ count: 2, total: 3 }, 1, true)).toBe("1 of 3 conflicts checked");
    // ...
    expect(checkSummary({ count: 1, total: 1 }, 1, false)).toBe("1 of 1 conflict checked");
```

and rename the third test to `"uses the singular for one conflict"`.

- [ ] **Step 2: Run it to verify it fails**

Run: `npx vitest run src/lib/checks.test.ts`
Expected: FAIL, "faults" ≠ "conflicts".

- [ ] **Step 3: Update `checks.ts`**

```ts
/** `done` verdicts so far; `stopped` once Jev failed and the rest were dropped. */
export function checkSummary(checking: { count: number; total: number }, done: number, stopped: boolean): string {
  const conflicts = checking.total === 1 ? "conflict" : "conflicts";
  if (!stopped && done < checking.count) return `Checking ${checking.count} of ${checking.total} ${conflicts}…`;
  return `${done} of ${checking.total} ${conflicts} checked`;
}
```

Run: `npx vitest run src/lib/checks.test.ts` → PASS.

- [ ] **Step 4: Update the commands**

In `commands.rs` imports, add `use livebrainstorm_core::sections;`. Replace `create_tab`:

```rust
/// `sections`: the new tab's section choice (the shown tab's, when it is on the same project).
#[tauri::command]
pub fn create_tab(app: AppHandle, window: String, root: Option<String>, sections: Vec<String>) -> Result<Tab> {
    check_id(&window)?;
    let tab = app.state::<AppState>().sessions.create_tab(&window, root, sections)?;
    events::workspace_changed(&app, vec![window]);
    Ok(tab)
}
```

After `rename_tab`, add:

```rust
/// The tab's section choice. No `workspace-changed`: a tab belongs to one window and
/// lists don't show sections; the window that changed it updates its own view.
#[tauri::command]
pub fn set_tab_sections(state: State<'_, AppState>, tab_id: String, sections: Vec<String>) -> Result<()> {
    check_id(&tab_id)?;
    state.sessions.set_sections(&tab_id, sections)
}
```

In `analyze`, add the parameter `sections: Vec<String>,` after `instructions: String,`. Inside the task, replace the block from `let groups = prompt::section_groups(...)` through `drop(faults_tx);` with:

```rust
        let chosen = sections::resolve(&sections); // unknown or stale ids fall back to the default set
        let groups = prompt::section_groups(settings.effective_note_agents(), &chosen);
        let (conflicts_tx, conflicts_rx) = oneshot::channel::<String>();
        let mut conflicts_tx = Some(conflicts_tx);
        let mut lanes = Vec::new();
        for (lane, lane_sections) in groups.iter().enumerate() {
            let req = prompt::analysis_request(&draft, index.as_deref(), brief.as_deref(), &snippets, &provider, lane_sections, &instructions);
            let tx = if lane_sections.iter().any(|s| s.checkable) { conflicts_tx.take() } else { None };
            let (provider, on_event) = (&provider, &on_event);
            let (ledger, tag) = (ledger.clone(), tag.clone());
            lanes.push(async move {
                let mut sink = |chunk: Chunk| match chunk {
                    Chunk::Text(text) => send(on_event, UiEvent::Text { text, lane }),
                    Chunk::Activity(label) => send(on_event, UiEvent::Activity { label, lane }),
                    Chunk::Usage(_) => {}
                };
                let result = usage::metered(provider, &req, ledger, tag, &mut sink).await;
                if let (Some(tx), Ok(text)) = (tx, &result) {
                    let _ = tx.send(text.clone());
                }
                result
            });
        }
        drop(conflicts_tx); // with Conflicts unselected the check below ends at once
```

In the `check` future, replace `faults_rx` with `conflicts_rx` and the body check with:

```rust
            if let Some(conflicts) = prompt::section_body(&text, "conflicts") {
                check::run(jev, index, &conflicts, |ev| {
```

Update the comments: `// "Checked in code" starts as soon as the Conflicts lane finishes, …` and the `UiEvent::Checking` doc to `/// `count` of the `total` Conflicts name code that's being checked.`

In `src-tauri/src/lib.rs`, add `commands::set_tab_sections,` after `commands::rename_tab,` in `generate_handler!`.

Run: `cargo check -p livebrainstorm` → no errors. Run: `cargo clippy --workspace --all-targets` → no warnings.

- [ ] **Step 5: Mirror in `ipc.ts`**

Add `sections: string[];` to `Tab` after `draft: string;` with the comment `/** Catalogue ids in catalogue order; empty means the default set. */`. Change the `Snapshot.checksFaults` comment to `/** The Conflicts count from the same event (`total`); null in files saved before it existed. The field keeps its old name. */`. Update the wrappers:

```ts
  analyze: (
    tabId: string,
    draft: string,
    previousDraft: string | null,
    instructions: string,
    sections: string[],
    context: string,
    onEvent: (e: UiEvent) => void,
  ) => stream("analyze", { tabId, draft, previousDraft, instructions, sections, context }, onEvent),
  // ...
  createTab: (window: string, root: string | null, sections: string[]) => invoke<Tab>("create_tab", { window, root, sections }),
  setTabSections: (tabId: string, sections: string[]) => invoke<void>("set_tab_sections", { tabId, sections }),
```

Change the Jev toggle label in `SettingsDialog.tsx:492` to `"Check Conflicts against the code"`.

`npm run typecheck` still fails (Task 6 and Task 7 fix `tabs.ts`, `tabSession.ts`, tests and `NotesPane.tsx`); that is expected.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands.rs src-tauri/src/lib.rs src/lib/ipc.ts src/lib/checks.ts src/lib/checks.test.ts src/components/SettingsDialog.tsx
git commit -m "feat(shell): analyze takes the tab's sections; set_tab_sections and create_tab with sections; the check reads Conflicts

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: TabSession carries sections; the key changes only off the default

**Files:**
- Modify: `src/lib/instructions.ts` (add `tabContextKey`)
- Modify: `src/lib/instructions.test.ts` (add a `tabContextKey` block)
- Modify: `src/lib/tabSession.ts` (deps, `TabView.sections`, fields, `restore`, `setContext`, new `setSections`, new `applyKey`, `run`)
- Modify: `src/lib/tabSession.test.ts` (fixtures, `Call`, new tests)
- Modify: `src/lib/tabs.ts:50-57` (deps), `:163-173` (`newTab`), new `setSections`, export
- Modify: `src/lib/tabs.test.ts:27-38` (`full` fixture), new test
- Modify: `src/lib/tabSession.tabs.test.ts` and `src/lib/useTabSession.test.ts` (add `sections: []` to any `Tab` fixture; find them with `grep -ln "nextSnapshot" src/lib/*.test.ts`)

**Interfaces:**
- Consumes: `isDefaultSections` from Task 3; `api.analyze(…, sections, context, …)`, `api.setTabSections`, `api.createTab(window, root, sections)` from Task 5.
- Produces:
  - `tabContextKey(projectKey: string, sections: string[]): string`
  - `TabSessionDeps.analyze(tabId, draft, previousDraft, instructions, sections: string[], context, onEvent)`
  - `TabView.sections: string[]` (the stored ids; empty = default)
  - `TabSession.setSections(ids: string[]): void`
  - `tabs.setSections(tabId: string, ids: string[]): Promise<void>`

- [ ] **Step 1: Write the failing tests**

In `src/lib/instructions.test.ts`, import `tabContextKey` and add:

```ts
describe("tabContextKey", () => {
  it("adds nothing for the default set, so tabs from the old build keep their key", () => {
    expect(tabContextKey("/p|1|", [])).toBe("/p|1|");
    expect(tabContextKey("/p|1|", ["intent", "conflicts", "gaps", "ambiguities", "questions"])).toBe("/p|1|");
    expect(tabContextKey("/p|1|", ["conflicts", "questions"])).toBe("/p|1||conflicts,questions");
  });
});
```

In `src/lib/tabSession.test.ts`: add `sections: [],` to the `tab()` fixture after `draft: DRAFT,`; add `sections: string[];` to `Call` and `sections` to the recorded call in `harness` (the analyze mock signature becomes `(tabId, draft, previousDraft, instructions, sections, context, onEvent)`); in the first test's `toMatchObject` add `sections: []`. Then add a new block:

```ts
describe("sections", () => {
  it("a run carries the tab's stored sections", () => {
    const { s, calls } = harness(tab({ draft: "", history: [], sections: ["conflicts", "questions"] }));
    s.setContext("/p|1|", "");
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(1000);
    expect(calls[0]).toMatchObject({ sections: ["conflicts", "questions"], context: "/p|1||conflicts,questions" });
    expect(s.getState().sections).toEqual(["conflicts", "questions"]);
  });

  it("changing the sections re-reads the same draft past the gate, with the new key", () => {
    const { s, calls } = harness(tab());
    s.setContext("/p|1|", "");
    expect(calls).toEqual([]); // the saved notes match the draft
    s.setSections(["gaps"]);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ draft: DRAFT, previousDraft: null, sections: ["gaps"], context: "/p|1||gaps" });
    expect(s.getState().sections).toEqual(["gaps"]);
  });

  it("back to the default set is the saved key again: no run", () => {
    const { s, calls } = harness(tab({ sections: ["gaps"], history: [snap({ context: "/p|1||gaps" })] }));
    s.setContext("/p|1|", "");
    s.setSections([]);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0].context).toBe("/p|1|");
  });

  it("the same sections again change nothing", () => {
    const { s, calls } = harness(tab({ sections: ["gaps"], history: [snap({ context: "/p|1||gaps" })] }));
    s.setContext("/p|1|", "");
    s.setSections(["gaps"]);
    vi.advanceTimersByTime(1000);
    expect(calls).toEqual([]);
  });

  it("on a hidden tab the change waits until the tab is shown", () => {
    const { s, calls } = harness(tab());
    s.setContext("/p|1|", "");
    s.deactivate();
    s.setSections(["gaps"]);
    vi.advanceTimersByTime(1000);
    expect(calls).toEqual([]);
    expect(s.getState().contextDirty).toBe(true);
    s.activate();
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0].context).toBe("/p|1||gaps");
  });

  it("setSections during a run supersedes it", () => {
    const { s, calls } = harness(tab({ draft: "", history: [] }));
    s.setContext("/p|1|", "");
    s.setDraft(DRAFT);
    vi.advanceTimersByTime(1000);
    expect(s.getState().analysis.status).toBe("thinking");
    s.setSections(["gaps"]);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(2);
    calls[0].emit({ type: "text", text: "## Intent\n- stale", lane: 0 });
    expect(s.getState().analysis.markdown).toBe(""); // the first run's events are dropped
    calls[1].emit({ type: "started" });
    calls[1].emit({ type: "text", text: "## Gaps\n- g", lane: 0 });
    expect(s.getState().analysis.markdown).toBe("## Gaps\n- g");
  });

  it("without a project key (the folder couldn't be read) a change still re-runs with the new sections", () => {
    const { s, calls } = harness(tab());
    s.setContext(null, "");
    s.setSections(["gaps"]);
    vi.advanceTimersByTime(1000);
    expect(calls).toHaveLength(1);
    expect(calls[0]).toMatchObject({ sections: ["gaps"], context: "/p|1|" });
  });
});
```

In `src/lib/tabs.test.ts`: add `sections: [],` to the `full()` fixture after `draft,`. Add a test next to the existing `newTab` coverage (or at the end):

```ts
  it("a new tab on the shown tab's project copies its sections; elsewhere it gets the default", async () => {
    const a1 = full("a1", "/a");
    a1.sections = ["conflicts", "questions"];
    const { tabs } = await loadStore(view(stored, a1)); // `loadStore` returns the module
    api.createTab.mockResolvedValue(full("a3", "/a"));
    api.listWorkspace.mockResolvedValue(view([...stored, summary("a3", "/a")], full("a3", "/a")));
    await tabs.newTab("/a");
    expect(api.createTab).toHaveBeenLastCalledWith("w-1", "/a", ["conflicts", "questions"]);
    api.createTab.mockResolvedValue(full("b2", "/b"));
    await tabs.newTab("/b");
    expect(api.createTab).toHaveBeenLastCalledWith("w-1", "/b", []);
  });

  it("setSections saves, then tells the session; a failed save changes nothing", async () => {
    const a1 = full("a1", "/a");
    const mod = await loadStore(view(stored, a1));
    api.setTabSections.mockResolvedValue(undefined);
    await mod.tabs.setSections("a1", ["gaps"]);
    expect(api.setTabSections).toHaveBeenCalledWith("a1", ["gaps"]);
    expect(mod.sessionFor("a1")?.getState().sections).toEqual(["gaps"]);
    api.setTabSections.mockRejectedValue(new Error("disk full"));
    await mod.tabs.setSections("a1", ["intent"]);
    expect(mod.sessionFor("a1")?.getState().sections).toEqual(["gaps"]);
    expect(mod.workspaceState().error).toBe("disk full");
  });
```

`loadStore` returns the imported module (`mod`), so the first new test also reads `mod.tabs`. Add `setTabSections: vi.fn(),` to the hoisted `api` mock.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `npx vitest run src/lib/instructions.test.ts src/lib/tabSession.test.ts src/lib/tabs.test.ts`
Expected: FAIL (`tabContextKey` not exported; `setSections` not a function; `sections` undefined on calls).

- [ ] **Step 3: Add `tabContextKey`**

In `src/lib/instructions.ts`, import `isDefaultSections` from `./sections` and add after `analysisContextKey`:

```ts
/**
 * A tab's key: its project's key plus its sections, only when they differ
 * from the default set. So a tab saved by the old build keeps the same key
 * and relaunching never re-runs it.
 */
export const tabContextKey = (projectKey: string, sections: string[]) =>
  isDefaultSections(sections) ? projectKey : `${projectKey}|${sections.join(",")}`;
```

- [ ] **Step 4: Update `TabSession`**

In `tabSession.ts`:
- Import `tabContextKey` from `./instructions`.
- `TabSessionDeps.analyze` gains `sections: string[]` between `instructions` and `context`.
- `TabView` gains `/** The stored ids; empty is the default set. */ sections: string[];`.
- Add fields after `private instructions = "";`:

```ts
  /** The project's key as the window last handed it over; null until then, or when the project couldn't be read. */
  private projectKey: string | null = null;
  /** This tab's section choice (`Tab.sections`); `context` combines it with `projectKey`. */
  private sections: string[] = [];
```

- In `restore(tab)`, after `this.draft = tab.draft;` add `this.sections = tab.sections ?? [];` and `this.projectKey = null;`.
- Replace `setContext` with:

```ts
  setContext(key: string | null, instructions: string): void {
    if (this.disposed) return;
    this.instructions = instructions;
    this.contextReady = true;
    if (key !== null) this.projectKey = key;
    const combined = key === null ? null : tabContextKey(key, this.sections);
    const waiting = this.waiting;
    if (waiting && this.active) {
      this.waiting = null;
      if (combined !== null) this.context = combined;
      this.run(this.draft, waiting.rerun); // reads the context just set
      return;
    }
    if (combined !== null) this.applyKey(combined);
  }

  /**
   * The tab's section choice. A change is a context change: the shown tab
   * re-reads the draft after a pause with no gate, a hidden one when shown.
   * Without a project key the key can't move, but the next run still carries
   * the new sections, so it is scheduled anyway.
   */
  setSections(ids: string[]): void {
    if (this.disposed || (ids.length === this.sections.length && ids.every((id, i) => id === this.sections[i]))) return;
    this.sections = ids;
    if (this.projectKey !== null) {
      this.applyKey(tabContextKey(this.projectKey, ids));
    } else if (this.active) {
      this.resetAndSchedule();
    } else {
      this.contextDirty = true;
    }
    this.emit();
  }

  /** A combined key. The first one a tab with no saved analysis gets is not a change. */
  private applyKey(key: string): void {
    if (key === this.context) return;
    const first = this.context === "";
    this.context = key;
    if (first) return;
    if (this.active) this.resetAndSchedule();
    else this.contextDirty = true;
    this.emit();
  }
```

Keep the long doc comment that was above `setContext`, adding one sentence: "The key handed over is the project's; the tab adds its own sections (`tabContextKey`)."

- In `run`, pass `this.sections` to `analyze`: `this.deps.analyze(this.id, text, previousDraftFor(this.lastAnalyzed, rerun, ctx), this.instructions, this.sections, ctx, (ev) => { … })`.
- In `emit`, add `sections: this.sections,` to the state object.

- [ ] **Step 5: Update `tabs.ts`**

```ts
const deps: TabSessionDeps = {
  analyze: (tabId, draft, previousDraft, instructions, sections, context, onEvent) =>
    api.analyze(tabId, draft, previousDraft, instructions, sections, context, onEvent),
  // ...unchanged
};
```

In `newTab`:

```ts
async function newTab(root: string | null): Promise<void> {
  ++switching;
  set({ error: null });
  await saver.flush();
  try {
    // A tab opened from the shown tab's project starts with its sections; elsewhere, the default set.
    const shown = state.activeId !== null ? sessions.get(state.activeId) : undefined;
    const sections = shown && shown.root === root ? shown.getState().sections : [];
    const tab = await api.createTab(state.windowId, root, sections); // appended to the window and made active
    show(sessionOf(tab));
    await refresh();
  } catch (e) {
    set({ error: errorMessage(e) });
  }
}
```

Add before `export const tabs`:

```ts
/** Saves the tab's section choice, then applies it to its session (which re-runs the shown tab). */
async function setSections(tabId: string, ids: string[]): Promise<void> {
  set({ error: null });
  try {
    await api.setTabSections(tabId, ids);
    sessions.get(tabId)?.setSections(ids);
  } catch (e) {
    set({ error: errorMessage(e) });
  }
}
```

and `setSections,` to the `tabs` export after `rename,`.

Add `sections: []` to every `Tab` fixture in `src/lib/tabSession.tabs.test.ts` and `src/lib/useTabSession.test.ts` (search for `nextSnapshot`).

- [ ] **Step 6: Run the tests and typecheck**

Run: `npx vitest run src/lib` and `npm run typecheck`
Expected: all `src/lib` tests pass. Typecheck still reports `NotesPane.tsx` (`DOT`) until Task 7; nothing else.

- [ ] **Step 7: Commit**

```bash
git add src/lib
git commit -m "feat(ui): tab sessions carry their sections; the key adds them only off the default; new tabs copy the shown tab's

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: The Sections picker in the notes pane

**Files:**
- Modify: `src/components/ui.tsx:97-101` (`MenuItem`), `:172-187` (the item button)
- Modify: `src/components/NotesPane.tsx` (props, `DOT` removed, header row with the picker, empty state, section rendering)
- Modify: `src/App.tsx:260-271` (`NotesPane` props)

**Interfaces:**
- Consumes: `CATALOGUE`, `SECTIONS`, `effectiveSections`, `toggleSection`, `sectionsSentence` (Task 3); `TabView.sections` and `tabs.setSections` (Task 6).
- Produces: `MenuItem.checked?: boolean`, `MenuItem.keepOpen?: boolean`; `NotesPane` prop `onSections: (ids: string[]) => void`.

- [ ] **Step 1: Extend `MenuItem`**

In `ui.tsx`:

```ts
export interface MenuItem {
  label: string;
  onSelect: () => void;
  disabled?: boolean;
  /** Set: the item is a checkbox (role menuitemcheckbox) with a leading ✓ when true. */
  checked?: boolean;
  /** Choosing it leaves the menu open (toggles). */
  keepOpen?: boolean;
}
```

and the item button:

```tsx
        <button
          key={item.label}
          type="button"
          role={item.checked === undefined ? "menuitem" : "menuitemcheckbox"}
          aria-checked={item.checked}
          disabled={item.disabled}
          className="grid cursor-pointer grid-cols-[1rem_1fr] items-center gap-1 rounded px-2.5 py-1.5 text-left text-[13px] text-ink hover:bg-subtle focus-visible:bg-subtle focus-visible:outline-none disabled:cursor-default disabled:opacity-50"
          onClick={() => {
            if (!item.keepOpen) close.current();
            item.onSelect();
          }}
        >
          <span aria-hidden>{item.checked ? "✓" : ""}</span>
          <span>{item.label}</span>
        </button>
```

- [ ] **Step 2: Add the picker and the sections to `NotesPane`**

Imports: replace `import { SECTIONS, parseSections, type SectionKey } from "../lib/sections";` with
`import { CATALOGUE, SECTIONS, effectiveSections, parseSections, sectionsSentence, toggleSection } from "../lib/sections";` and add `Menu` to the `./ui` import. Delete the `DOT` record.

Add to `Props`: `/** The tab's section choice changed (stored ids; empty is the default set). */ onSections: (ids: string[]) => void;`.

Add a picker component above `NotesPane`:

```tsx
/** `Sections ▾`: toggles for the catalogue. The last checked one can't be unchecked. */
function SectionsPicker({ chosen, disabled, onChange }: { chosen: string[]; disabled: boolean; onChange: (ids: string[]) => void }) {
  const [at, setAt] = useState<{ x: number; y: number } | null>(null);
  const current = effectiveSections(chosen);
  return (
    <>
      <Button
        variant="mini"
        aria-haspopup="menu"
        aria-expanded={at !== null}
        disabled={disabled}
        title="Which sections the notes cover"
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setAt(at ? null : { x: r.left, y: r.bottom + 4 });
        }}
      >
        Sections ▾
      </Button>
      {at && (
        <Menu
          at={at}
          label="Sections"
          onClose={() => setAt(null)}
          items={CATALOGUE.map((s) => {
            const checked = current.includes(s.key);
            return {
              label: s.title,
              checked,
              keepOpen: true,
              disabled: checked && current.length === 1,
              onSelect: () => onChange(toggleSection(chosen, s.key)),
            };
          })}
        />
      )}
    </>
  );
}
```

In `NotesPane`, destructure `onSections` and replace the status paragraph with a header row:

```tsx
        {older ? (
          <EarlierDraft key={older.id} draft={older.draft} />
        ) : (
          <div className="mb-5 flex items-center justify-between gap-2">
            <p className="text-xs text-faint" aria-live="polite">
              {statusText(view)}
            </p>
            <SectionsPicker chosen={view.sections} disabled={false} onChange={onSections} />
          </div>
        )}
```

Replace the idle text with:

```tsx
        {a.status === "idle" && (
          <div className="max-w-[46ch] space-y-2.5 text-[13px] leading-relaxed text-muted">
            <p>Start writing your spec on the left. When you pause, notes appear here:</p>
            <p>{sectionsSentence(view.sections)}</p>
          </div>
        )}
```

Replace the section rendering loop:

```tsx
            {SECTIONS.map((s) =>
              sections[s.key] !== undefined ? (
                <section key={s.key} className="grid gap-1.5">
                  <h2>
                    <Eyebrow dot={s.dot}>{s.title}</Eyebrow>
                  </h2>
                  <MiniMarkdown text={sections[s.key]!} />
                </section>
              ) : null,
            )}
```

Change the "thinking" placeholder to `Reading your spec against the project…`.

In `App.tsx`, add to the `NotesPane` element: `onSections={(ids) => void tabs.setSections(session.id, ids)}`.

- [ ] **Step 3: Typecheck and run the app**

Run: `npm run typecheck` → clean. Run: `npm test` → all pass.
Manual (`npm run tauri dev`, Node 20.19+): open a tab, click `Sections ▾`, uncheck Gaps: the menu stays open, the mark disappears, and after a pause the notes re-run without a Gaps section. Uncheck everything but one: the last item is greyed. Press ‹ to an older analysis: the picker is absent (the header is the earlier-draft banner).

- [ ] **Step 4: Commit**

```bash
git add src/components/ui.tsx src/components/NotesPane.tsx src/App.tsx
git commit -m "feat(ui): Sections picker above the notes; sections render from the catalogue table

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: Templates carry sections; spec-oriented starters

**Files:**
- Modify: `crates/livebrainstorm-core/src/instructions.rs:14-20` (`Template`), `:58-105` (`STARTERS`, `starters`), tests
- Modify: `src/lib/ipc.ts:75-79` (`Template.sections`)
- Modify: `src/lib/instructions.ts:36-40` (`applyTemplate`), `src/lib/instructions.test.ts`
- Modify: `src/components/InstructionsDrawer.tsx` (props `sections`, `onSections`; apply/undo; lead text)
- Modify: `src/components/TemplateList.tsx` (prop `sections`; new template default; editor toggles)
- Modify: `src/App.tsx:308-325` (drawer and list props)

**Interfaces:**
- Consumes: `normalizeSections`, `toggleSection`, `CATALOGUE`, `effectiveSections` (Task 3); `tabs.setSections` (Task 6).
- Produces:
  - Rust `Template.sections: Vec<String>` (`#[serde(default)]`); starters per the spec's table
  - TS `Template.sections: string[]`
  - `applyTemplate(p, t, current: string[]): { next: ProjectInstructions; sections: string[]; undo: { instructions: ProjectInstructions; sections: string[] } }`

- [ ] **Step 1: Write the failing Rust tests**

In `instructions.rs` tests, change `a_missing_file_starts_with_the_starters` to expect `["feature-spec", "bugfix-spec", "refactor-spec", "handoff", "devils-advocate"]`, and add:

```rust
    #[test]
    fn starters_pick_catalogue_sections_per_the_spec() {
        let sets: Vec<(&str, Vec<&str>)> =
            starters().iter().map(|t| (t.id.as_str(), t.sections.iter().map(String::as_str).collect())).map(|(id, s)| (id, s)).collect::<Vec<_>>();
        let by_id = |id: &str| sets.iter().find(|(i, _)| *i == id).map(|(_, s)| s.clone()).unwrap();
        assert_eq!(by_id("feature-spec"), ["intent", "conflicts", "gaps", "ambiguities", "questions"]);
        assert_eq!(by_id("bugfix-spec"), ["intent", "conflicts", "gaps", "questions"]);
        assert_eq!(by_id("refactor-spec"), ["conflicts", "gaps", "ambiguities", "questions"]);
        assert_eq!(by_id("handoff"), ["ambiguities", "gaps"]);
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
```

Note `handoff` is `["ambiguities", "gaps"]` in the spec's table but catalogue order is gaps before ambiguities, so the stored list is `["gaps", "ambiguities"]`. Use `["gaps", "ambiguities"]` in the `handoff` assertion; the `normalize` loop pins it.

Run: `cargo test -p livebrainstorm-core instructions::` → compile error (no field `sections`).

- [ ] **Step 2: Add the field and rewrite the starters**

```rust
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
```

```rust
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
```

Fix the test `deleting_every_template_sticks_until_restore`: it edits `starters()[1]` and checks `t.id == "bug-hunt"`; change that id to `"bugfix-spec"`.

Run: `cargo test -p livebrainstorm-core instructions::` → all pass. Run: `cargo check -p livebrainstorm` → ok.

- [ ] **Step 3: Write the failing frontend test**

In `src/lib/instructions.test.ts`, change the `t` helper to `const t = (id, name, text, sections: string[] = []): Template => ({ id, name, text, sections });` and the "apply" test to:

```ts
  it("apply replaces the text, switches the tab's sections when the template has some, and keeps both for undo", () => {
    const before = p({ current: "mine", default: "d" });
    const plain = applyTemplate(before, bug, ["gaps"]);
    expect(plain.next).toEqual({ default: "d", current: bug.text, templateId: "bug-hunt" });
    expect(plain.sections).toEqual(["gaps"]); // a template without sections leaves the tab's alone
    expect(plain.undo).toEqual({ instructions: before, sections: ["gaps"] });
    const handoff = t("handoff", "Hand-off pass", "Hunt.", ["ambiguities", "gaps"]);
    expect(applyTemplate(before, handoff, ["gaps"]).sections).toEqual(["gaps", "ambiguities"]); // normalized to catalogue order
  });
```

Run: `npx vitest run src/lib/instructions.test.ts` → FAIL.

- [ ] **Step 4: Update `ipc.ts` and `instructions.ts`**

`ipc.ts`: add `/** Catalogue ids; empty leaves the tab's sections alone. */ sections: string[];` to `Template`.

`instructions.ts`: import `normalizeSections` from `./sections` and replace `applyTemplate`:

```ts
export interface AppliedTemplate {
  next: ProjectInstructions;
  /** The shown tab's sections after applying: the template's, or unchanged when it has none. */
  sections: string[];
  undo: { instructions: ProjectInstructions; sections: string[] };
}

/** Applying replaces the current text and, when the template has sections, the shown tab's sections; `undo` is exactly what was there. */
export function applyTemplate(p: ProjectInstructions, t: Template, current: string[]): AppliedTemplate {
  return {
    next: { ...p, current: t.text, templateId: t.id },
    sections: t.sections.length > 0 ? normalizeSections(t.sections) : current,
    undo: { instructions: p, sections: current },
  };
}
```

Run: `npx vitest run src/lib/instructions.test.ts` → PASS.

- [ ] **Step 5: The drawer and the template editor**

`InstructionsDrawer.tsx` props: add

```ts
  /** The shown tab's section choice (stored ids; empty is the default set) and how to change it. */
  sections: string[];
  onSections: (ids: string[]) => void;
```

Change the undo state to `useState<{ instructions: ProjectInstructions; sections: string[] } | null>(null)` and `apply`:

```ts
  const apply = () => {
    const t = p.templates.find((x) => x.id === pickedId);
    if (!t) return;
    const applied = applyTemplate(i, t, p.sections);
    p.onChange(applied.next);
    if (applied.sections !== p.sections) p.onSections(applied.sections);
    setUndo(applied.undo);
  };
```

and the Undo button's handler:

```ts
              onClick={() => {
                p.onChange(undo.instructions);
                if (undo.sections !== p.sections) p.onSections(undo.sections);
                setUndo(null);
              }}
```

Lead text: `These steer the notes for this project: what Live Spec looks for. Which sections appear is chosen per tab, above the notes.` Placeholder: `For example: focus Conflicts on error handling and race conditions.`

`TemplateList.tsx`: add prop `/** The shown tab's sections: a new template starts from them. */ sections: string[];`, make "+ New template" start with `{ id: crypto.randomUUID(), name: "", text: current, sections: effectiveSections(sections) }`, and give `TemplateEditor` toggles under the text:

```tsx
      <fieldset className="grid gap-1 text-xs text-muted">
        <legend className="mb-1">Sections</legend>
        {CATALOGUE.map((s) => (
          <label key={s.key} className="flex items-center gap-2 text-[13px] text-ink">
            <input
              type="checkbox"
              className="accent-accent"
              checked={v.sections.includes(s.key)}
              onChange={() => props.onChange({ ...v, sections: toggleSection(v.sections, s.key, true) })}
            />
            {s.title}
          </label>
        ))}
        {v.sections.length === 0 && <small>None picked: applying this template leaves the tab's sections as they are.</small>}
      </fieldset>
```

Import `CATALOGUE`, `effectiveSections`, `toggleSection` from `../lib/sections`. Note `toggleSection([], key, true)` on an empty template list returns the default set minus `key`; that is the intended behaviour for "start from all and remove one".

`App.tsx`: pass `sections={view.sections}` and `onSections={(ids) => void tabs.setSections(session.id, ids)}` to `InstructionsDrawer`, and `sections={view.sections}` to `TemplateList`. (`view` is non-null whenever `session` is, inside the `session && view` branch; the drawer sits outside it, so guard: `showInstructions && session && view`.)

- [ ] **Step 6: Typecheck, test, try it**

Run: `npm run typecheck && npm test` → clean. Manual: apply "Hand-off pass": the text changes and the notes re-run with Gaps and Ambiguities only; Undo restores both. Edit a template: the checkboxes show its set.

- [ ] **Step 7: Commit**

```bash
git add crates/livebrainstorm-core/src/instructions.rs src/lib/ipc.ts src/lib/instructions.ts src/lib/instructions.test.ts src/components/InstructionsDrawer.tsx src/components/TemplateList.tsx src/App.tsx
git commit -m "feat: templates carry a section set; five spec-oriented starters; applying a template switches the shown tab's sections

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 9: Copy spec and the spec-writing copy

**Files:**
- Modify: `src/components/DraftPane.tsx` (whole component)

**Interfaces:** none new.

- [ ] **Step 1: Rewrite `DraftPane`**

```tsx
import { useEffect, useState } from "react";
import { Button, cx } from "./ui";

interface Props {
  value: string;
  onChange: (v: string) => void;
  hasProject: boolean;
}

// Centres a ~40rem (about 68 characters) column inside the full-width pane, so
// the whole pane is the click and scroll target but lines stay readable.
// Percent padding resolves against the pane, so textarea and footer line up.
const GUTTER = "px-[max(2rem,calc((100%_-_40rem)/2))]";

const COPY_FAILED = "Couldn't copy. Select the text and copy it instead.";

export function DraftPane({ value, onChange, hasProject }: Props) {
  const words = value.trim() ? value.trim().split(/\s+/).length : 0;
  const [copy, setCopy] = useState<"idle" | "copied" | "failed">("idle");

  // "Copied" and the failure note clear themselves.
  useEffect(() => {
    if (copy === "idle") return;
    const t = setTimeout(() => setCopy("idle"), 2000);
    return () => clearTimeout(t);
  }, [copy]);

  const copySpec = () => {
    navigator.clipboard
      .writeText(value)
      .then(() => setCopy("copied"))
      .catch(() => setCopy("failed"));
  };

  return (
    <section className="flex min-h-0 flex-col bg-page" aria-label="Your spec">
      <textarea
        className={cx(
          "min-h-0 w-full flex-1 resize-none overflow-y-auto bg-transparent pt-10 pb-6 font-serif text-[17px] leading-[1.75] outline-none focus-visible:outline-none placeholder:text-faint",
          GUTTER,
        )}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        spellCheck
        autoFocus
        placeholder={
          hasProject
            ? "Write the spec here. Live Spec reads along and replies when you pause."
            : "Open the project the spec is for, so Live Spec can check it against the code, or just start writing."
        }
      />
      <footer className={cx("flex shrink-0 items-center gap-3 pt-1 pb-3 text-xs tabular-nums text-faint", GUTTER)}>
        <span>{words} words</span>
        <Button variant="link" onClick={copySpec} disabled={!value.trim()} title="Copy the spec to the clipboard">
          Copy spec
        </Button>
        {copy === "copied" && <span aria-live="polite">Copied</span>}
        {copy === "failed" && (
          <span className="text-fault" role="alert">
            {COPY_FAILED}
          </span>
        )}
      </footer>
    </section>
  );
}
```

- [ ] **Step 2: Typecheck and try it**

Run: `npm run typecheck` → clean. Manual: type a spec, click Copy spec, paste into a terminal: the whole draft arrives; "Copied" shows for two seconds. An empty draft greys the button.

- [ ] **Step 3: Commit**

```bash
git add src/components/DraftPane.tsx
git commit -m "feat(ui): Copy spec in the draft footer; spec-writing placeholders

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: The folder migration in core

**Files:**
- Create: `crates/livebrainstorm-core/src/migrate.rs`
- Modify: `crates/livebrainstorm-core/src/lib.rs` (add `pub mod migrate;`)

**Interfaces:**
- Produces:
  - `pub const OLD_IDENTIFIER: &str = "app.livebrainstorm.desktop";`
  - `pub fn old_path(new_dir: &Path, old_identifier: &str) -> Option<PathBuf>`
  - `pub enum Moved { Renamed, Copied, Nothing }`
  - `pub fn move_dir(old: &Path, new: &Path) -> std::io::Result<Moved>`
  - `pub fn move_app_dirs(new_dirs: &[PathBuf], old_identifier: &str, log: impl FnMut(String))`

- [ ] **Step 1: Write the failing tests**

Create `migrate.rs` with the doc comment and tests:

```rust
//! One-time move of the app's folders from the old bundle identifier's
//! location to the new one, on the first launch after the rename. Tauri
//! resolves the data and config folders as `<base>/<identifier>`, so the old
//! folder sits beside the new one. Nothing here deletes old data, and no
//! failure stops the launch: the caller logs and carries on.

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn seed(dir: &Path) {
        fs::create_dir_all(dir.join("sessions/tabs")).unwrap();
        fs::write(dir.join("sessions/tabs/t-1.json"), "{}").unwrap();
        fs::write(dir.join("settings.json"), "{}").unwrap();
    }

    #[test]
    fn the_old_path_swaps_the_final_component() {
        let new = Path::new("/Users/x/Library/Application Support/app.livespec.desktop");
        assert_eq!(old_path(new, OLD_IDENTIFIER), Some(PathBuf::from("/Users/x/Library/Application Support/app.livebrainstorm.desktop")));
        assert_eq!(old_path(Path::new("/"), OLD_IDENTIFIER), None);
    }

    #[test]
    fn a_missing_new_folder_takes_the_old_one_by_rename() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("old"), tmp.path().join("new"));
        seed(&old);
        assert_eq!(move_dir(&old, &new).unwrap(), Moved::Renamed);
        assert!(new.join("sessions/tabs/t-1.json").exists() && new.join("settings.json").exists());
        assert!(!old.exists());
    }

    #[test]
    fn an_empty_new_folder_is_treated_as_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("old"), tmp.path().join("new"));
        seed(&old);
        fs::create_dir_all(&new).unwrap(); // a plugin made it before setup ran
        assert_eq!(move_dir(&old, &new).unwrap(), Moved::Renamed);
        assert!(new.join("settings.json").exists());
    }

    #[test]
    fn an_existing_new_folder_or_a_missing_old_one_moves_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let (old, new) = (tmp.path().join("old"), tmp.path().join("new"));
        seed(&old);
        seed(&new);
        fs::write(new.join("settings.json"), r#"{"mine":true}"#).unwrap();
        assert_eq!(move_dir(&old, &new).unwrap(), Moved::Nothing);
        assert_eq!(fs::read_to_string(new.join("settings.json")).unwrap(), r#"{"mine":true}"#);
        assert!(old.exists(), "never deleted");
        assert_eq!(move_dir(&tmp.path().join("absent"), &tmp.path().join("new2")).unwrap(), Moved::Nothing);
        assert!(!tmp.path().join("new2").exists());
    }

    #[test]
    fn copy_dir_all_copies_nested_files() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        seed(&src);
        copy_dir_all(&src, &dst).unwrap();
        assert_eq!(fs::read_to_string(dst.join("sessions/tabs/t-1.json")).unwrap(), "{}");
        assert!(src.exists());
    }

    #[test]
    fn move_app_dirs_handles_each_distinct_folder_once_and_logs() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("base");
        let old = base.join(OLD_IDENTIFIER);
        seed(&old);
        let new = base.join("app.livespec.desktop");
        let mut log = Vec::new();
        move_app_dirs(&[new.clone(), new.clone()], OLD_IDENTIFIER, |m| log.push(m));
        assert!(new.join("settings.json").exists() && !old.exists());
        assert_eq!(log.len(), 1, "the same folder, given twice, is handled once: {log:?}");
        assert!(log[0].contains("moved"), "{log:?}");
        let mut again = Vec::new();
        move_app_dirs(&[new.clone()], OLD_IDENTIFIER, |m| again.push(m));
        assert!(again.is_empty(), "a second launch says nothing");
    }
}
```

Add `pub mod migrate;` to `lib.rs` between `pub mod menu;` and `pub mod project;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p livebrainstorm-core migrate::`
Expected: compile errors (nothing defined).

- [ ] **Step 3: Write the module**

Above the tests:

```rust
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The identifier the app shipped under before it became Live Spec.
pub const OLD_IDENTIFIER: &str = "app.livebrainstorm.desktop";

/// `<base>/<new identifier>` → `<base>/<old identifier>`. None for a path with no final component.
pub fn old_path(new_dir: &Path, old_identifier: &str) -> Option<PathBuf> {
    new_dir.file_name()?;
    Some(new_dir.with_file_name(old_identifier))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Moved {
    Renamed,
    /// The rename failed (another device, say): copied instead, the old folder left in place.
    Copied,
    Nothing,
}

/// Takes `old` as `new` when `new` is missing (or an empty folder a plugin made)
/// and `old` exists. A rename is one atomic move, so models in `local/` don't
/// get copied; when it fails, the folder is copied and the old one kept.
pub fn move_dir(old: &Path, new: &Path) -> io::Result<Moved> {
    if !old.is_dir() {
        return Ok(Moved::Nothing);
    }
    match fs::read_dir(new) {
        Ok(mut entries) if entries.next().is_some() => return Ok(Moved::Nothing),
        Ok(_) => fs::remove_dir(new)?, // empty: make room for the rename
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if fs::rename(old, new).is_ok() {
        return Ok(Moved::Renamed);
    }
    copy_dir_all(old, new)?;
    Ok(Moved::Copied)
}

fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let target = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// For each distinct folder in `new_dirs` (data and config are one folder on
/// macOS), moves the old identifier's folder in. Every outcome but "nothing to
/// do" is reported through `log`; errors are reported, never returned.
pub fn move_app_dirs(new_dirs: &[PathBuf], old_identifier: &str, mut log: impl FnMut(String)) {
    let mut seen: Vec<&PathBuf> = Vec::new();
    for new in new_dirs {
        if seen.contains(&new) {
            continue;
        }
        seen.push(new);
        let Some(old) = old_path(new, old_identifier) else { continue };
        match move_dir(&old, new) {
            Ok(Moved::Renamed) => log(format!("moved {} to {}", old.display(), new.display())),
            Ok(Moved::Copied) => log(format!("copied {} to {} (the old folder is still there)", old.display(), new.display())),
            Ok(Moved::Nothing) => {}
            Err(e) => log(format!("couldn't move {} to {}: {e}", old.display(), new.display())),
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p livebrainstorm-core migrate::`
Expected: 6 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/livebrainstorm-core/src/migrate.rs crates/livebrainstorm-core/src/lib.rs
git commit -m "feat(core): one-time move of the app folder from the old identifier's location

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 11: Rename to Live Spec, keychain migration, docs

**Files:**
- Rename: `crates/livebrainstorm-core/` → `crates/livespec-core/`
- Modify: `Cargo.toml`, `crates/livespec-core/Cargo.toml`, `src-tauri/Cargo.toml`, `src-tauri/src/main.rs`, every `.rs` under `src-tauri/src` and `crates/livespec-core` that says `livebrainstorm_core` or `LiveBrainstorm`
- Modify: `src-tauri/tauri.conf.json`, `package.json`, `package-lock.json` (via `npm install`), `index.html`
- Modify: `src-tauri/src/secrets.rs` (service, `migrate_from_old_service`), `src-tauri/src/lib.rs` (call both migrations in `setup`), `src-tauri/src/windows.rs:16` (title)
- Modify: `src/lib/tabList.ts:84,107`, `src/lib/tabList.test.ts`, `src/lib/windowTitle.test.ts`, `src/lib/tabs.test.ts` (any `LiveBrainstorm` expectations), `src/components/{BriefDrawer,SettingsDialog,TopBar}.tsx`, `src/styles.css:1-4`
- Modify: `README.md`, `AGENTS.md`, `CLAUDE.md`

**Interfaces:**
- Consumes: `migrate::{move_app_dirs, OLD_IDENTIFIER}` (Task 10).
- Produces: crate `livespec-core` (`use livespec_core::…`), package `livespec`, lib `livespec_lib`; `secrets::migrate_from_old_service()`.

- [ ] **Step 1: Rename the crate and the Rust identifiers**

```bash
git mv crates/livebrainstorm-core crates/livespec-core
perl -pi -e 's/crates\/livebrainstorm-core/crates\/livespec-core/g' Cargo.toml
perl -pi -e 's/^name = "livebrainstorm-core"/name = "livespec-core"/; s/for LiveBrainstorm/for Live Spec/' crates/livespec-core/Cargo.toml
perl -pi -e 's/^name = "livebrainstorm"$/name = "livespec"/; s/^name = "livebrainstorm_lib"/name = "livespec_lib"/; s/livebrainstorm-core = \{ path = "\.\.\/crates\/livebrainstorm-core" \}/livespec-core = { path = "..\/crates\/livespec-core" }/; s/Brainstorm alongside AI that understands your project/Write a spec beside an AI that has read your project/' src-tauri/Cargo.toml
perl -pi -e 's/livebrainstorm_lib/livespec_lib/' src-tauri/src/main.rs
grep -rl 'livebrainstorm_core' src-tauri/src crates/livespec-core | xargs perl -pi -e 's/livebrainstorm_core/livespec_core/g'
grep -rl 'LiveBrainstorm' src-tauri/src crates/livespec-core | xargs perl -pi -e 's/LiveBrainstorm/Live Spec/g'
grep -rl 'livebrainstorm-core' crates/livespec-core src-tauri/src | xargs perl -pi -e 's/livebrainstorm-core/livespec-core/g'
```

Then: `cargo test -p livespec-core` → all pass; `cargo check -p livespec` → ok. (`src-tauri/src/menu.rs` imports `livebrainstorm_core::menu` as `spec`; the perl pass covers it.)

- [ ] **Step 2: Product name, identifier, web package**

- `src-tauri/tauri.conf.json`: `"productName": "Live Spec"`, `"identifier": "app.livespec.desktop"`.
- `package.json`: `"name": "livespec"`, then `npm install` to update `package-lock.json`.
- `index.html`: `<title>Live Spec</title>`.
- `src-tauri/src/windows.rs:16`: `.title("Live Spec")`.
- `src/lib/tabList.ts`: `windowTitle` returns `"Live Spec"` / `` `Live Spec — ${title}` ``; `sidebarKey` returns `` `livespec.sidebar.${windowId}` ``. Update the expectations in `src/lib/tabList.test.ts` and `src/lib/windowTitle.test.ts` (`grep -n "LiveBrainstorm\|livebrainstorm.sidebar" src/lib/*.test.ts`).
- `src/styles.css` header comment: `/* Live Spec: a spec and the notes beside it. … Note colours carry meaning (conflicts red, gaps blue, ambiguities and questions amber). …`
- Components: `grep -rn "LiveBrainstorm" src/components` and replace each with `Live Spec` (TopBar brand, BriefDrawer two strings, SettingsDialog four strings).
- Leave `src/lib/legacyDrafts.ts` as it is: those keys name the pre-tabs build's storage.

Run: `grep -rn "LiveBrainstorm" src src-tauri crates index.html package.json` → no hits. `npm run typecheck && npm test` → clean.

- [ ] **Step 3: Keychain service and migration**

In `src-tauri/src/secrets.rs`:

```rust
const SERVICE: &str = "app.livespec.desktop";
/// The service the app saved under before it became Live Spec; read once to carry keys over.
const OLD_SERVICE: &str = "app.livebrainstorm.desktop";
```

Add after `set_secret`:

```rust
/// First launch after the rename: a key saved under the old service name is
/// copied to the new one. The old entry stays. macOS may ask once to allow
/// the new app to read the old item. Never fails the launch.
pub fn migrate_from_old_service() {
    for (account, what) in [(ACCOUNT, "API key"), (JEV_ACCOUNT, "TypeSafe key")] {
        if !matches!(keychain_get(account), Ok(None)) {
            continue; // already there, or the keychain is unavailable
        }
        let old = match keyring::Entry::new(OLD_SERVICE, account).and_then(|e| e.get_password()) {
            Ok(key) => key,
            Err(keyring::Error::NoEntry) => continue,
            Err(e) => {
                eprintln!("Live Spec: couldn't read the {what} saved by LiveBrainstorm: {e}");
                continue;
            }
        };
        match set_secret(account, &old, what) {
            Ok(()) => eprintln!("Live Spec: carried the {what} over from LiveBrainstorm"),
            Err(e) => eprintln!("Live Spec: {e}"),
        }
    }
}
```

In `src-tauri/src/lib.rs` `setup`, before `app.manage(AppState::new(&data_dir, &config_dir));`:

```rust
            // First launch after the rename: the old identifier's folder and keys come along.
            livespec_core::migrate::move_app_dirs(&[data_dir.clone(), config_dir.clone()], livespec_core::migrate::OLD_IDENTIFIER, |m| {
                eprintln!("Live Spec: {m}")
            });
            secrets::migrate_from_old_service();
```

Run: `cargo check -p livespec && cargo clippy --workspace --all-targets` → clean.

- [ ] **Step 4: README**

Replace the title and first paragraph:

```markdown
# Live Spec

Write a spec on the left; an AI that has read your project reviews it in the notes panel on the right. When you pause typing it tells you what the spec is asking for, where it conflicts with how the project actually works, what an implementing agent would need but can't find, which phrases it would have to guess at, and the questions that would most change the spec. The aim is a spec you can hand to Claude Code or any coding agent so it can read the project and see what to change without further discovery or guessing.
```

Replace the "Session instructions" section:

```markdown
### Sections and instructions

The notes come in up to five sections: **Intent**, **Conflicts with code** (fact-checked when Jev is on), **Gaps**, **Ambiguities** and **Questions for you**. Click **Sections ▾** above the notes to choose which ones a tab gets; each tab keeps its own choice, and a new tab opened from a project copies the shown tab's. **Copy spec** in the draft footer puts the spec on the clipboard, ready to paste to an agent.

Click **Instructions** in the top bar to steer the notes for the shown tab's project (every tab of a project shares them), for example "focus Conflicts on error handling". Start from a template (Feature spec, Bug fix spec, Refactor spec, Hand-off pass, Devil's advocate), edit it, or write your own. A template carries a section set as well as text: applying it also switches the shown tab's sections, and Undo restores both. Each project keeps its current instructions and a default you can reset to; new projects start with none. Add, edit and delete templates in the same drawer; *Restore starter templates* brings back any you deleted.
```

In the Jev paragraph, replace "checks every Fault in its notes" with "checks every Conflict in its notes". Then:

```bash
perl -pi -e 's/livebrainstorm-core/livespec-core/g; s/LiveBrainstorm/Live Spec/g' README.md
```

and add under "## Run it", after the provider table:

```markdown
Upgrading from LiveBrainstorm: the first launch of Live Spec moves the old app folder (tabs, briefs, settings, usage, local models) to the new one and copies your saved keys to the new keychain entry. macOS may ask once to allow Live Spec to read the old keychain item. Nothing is deleted. The sidebar's hidden/shown preference lives in the webview and starts over.
```

- [ ] **Step 5: AGENTS.md and CLAUDE.md**

Apply `perl -pi -e 's/livebrainstorm-core/livespec-core/g; s/livebrainstorm_lib/livespec_lib/g; s/package `livebrainstorm`/package `livespec`/g; s/LiveBrainstorm/Live Spec/g' AGENTS.md CLAUDE.md`, then edit these passages in `AGENTS.md`:

- **What this is:** replace the paragraph with: "Live Spec is a Tauri v2 desktop app for writing a spec against a codebase. The user types the spec on the left; when they pause, an LLM that has read the selected project streams review notes into the right-hand panel under the sections the tab has chosen from a five-entry catalogue: **Intent, Conflicts (with code), Gaps, Ambiguities, Questions**. The goal is a spec an AI coding agent can implement without further discovery or guessing. The model comes from one of three providers: …" (keep the provider sentence).
- **Live analysis flow, step 1:** after "Every `analyze` call carries the tab id, the current instructions and the context key", add "and the tab's section ids (`Tab.sections`; empty is the default set). A tab's key is its project's key plus `|<ids>` only when the sections differ from the default (`tabContextKey`), so tabs saved before the field existed keep their key."
- **Step 3/4:** replace "builds the prompt with `prompt::analysis_request` (which puts…" so it reads "resolves the tab's sections with `sections::resolve` (unknown ids dropped, nothing known → the whole catalogue), builds the prompt with `prompt::analysis_request` …". Replace the lane sentence "(`prompt::section_groups`: 1 → all four, 2 → …; always 1 for the Local provider)" with "(`prompt::section_groups(n, &chosen)`: at most `n` contiguous near-equal groups, never more than there are sections; always 1 for the Local provider)". Replace "The fact-check starts when the Faults lane finishes" with "The fact-check starts when the lane carrying Conflicts finishes (no `checking` event when Conflicts isn't chosen)".
- **Step 5:** "`src/lib/sections.ts` splits the streamed Markdown into the chosen sections (plus the old build's Directions, display-only), and `MiniMarkdown` renders them."
- **UI and styling, colours:** "The note colours carry meaning: conflicts red, gaps blue, ambiguities and questions amber."
- **Sidebar/notes:** add a bullet "**Sections picker** (`SectionsPicker` in `NotesPane.tsx`): `Sections ▾` opens the shared `Menu` with one checkbox item per catalogue entry (`MenuItem.checked`, `keepOpen`); the last checked one is disabled. `tabs.setSections` saves through `set_tab_sections`, then `TabSession.setSections` re-runs the shown tab after a pause (a hidden one when shown). `create_tab` takes the shown tab's sections when the new tab is on the same project."
- **Jev, fact-check bullet:** "takes the Conflicts section" instead of Faults; the key lookup etc. unchanged.
- **Cross-language contracts:** replace the "Section headings" row with "**Sections:** `sections::CATALOGUE` / `DEFAULT_SECTIONS` / `heading_id` in `crates/livespec-core/src/sections.rs` ↔ `SECTIONS` / `DEFAULT_SECTIONS` / the heading regex and `keyFor` in `src/lib/sections.ts`. Ids, order, the default set and the alias table (exact words: conflict(s) and fault(s) → conflicts, …) must match; both test suites pin them. The parser tolerates `**Conflicts:**`, `### Conflicts with code` and the old build's `## Faults`." In the Sessions row add "`Tab.sections`". In the Session instructions row add "`Template.sections` (empty: leave the tab's sections alone)". In the Saved checks row, "its `total` (all Conflicts)". In the Lanes row: "`prompt::section_groups` (Rust) decides which sections each `lane` carries; … `analysis_system(&resolve(&[]), "")` (no instructions) must stay byte-identical to the single-lane prompt (`all_five_sections_is_the_single_lane_prompt`)."
- **Persistence:** add "Identifier: `app.livespec.desktop` (was `app.livebrainstorm.desktop`). On launch, `migrate::move_app_dirs` renames the old folder into place when the new one is missing or empty (copy fallback, old kept) and `secrets::migrate_from_old_service` copies the two keychain entries. Neither deletes anything or fails the launch."
- **Deliberate decisions:** add "The old build's section names (`Faults`, `Directions`) stay in both parsers so saved notes render; never remove them."
- **Extension points → Starter templates:** "`STARTERS` in `crates/livespec-core/src/instructions.rs` (text plus a section set)." Add "Another section: add a `SectionSpec` to `sections::CATALOGUE` and a matching entry to `SECTIONS` in `src/lib/sections.ts`; the prompt, lanes, picker and parser follow."
- **Commands block:** `cargo test -p livespec-core`, `cargo check -p livespec` (package `livespec`, lib `livespec_lib`) are covered by the perl pass; verify with `grep -n livebrainstorm AGENTS.md CLAUDE.md` → no hits.

`CLAUDE.md` needs only the perl pass (it says "Live Spec itself drives Claude Code").

- [ ] **Step 6: Full verification**

```bash
cargo test -p livespec-core
cargo clippy --workspace --all-targets
cargo check -p livespec
npm run typecheck
npm test
grep -rn "LiveBrainstorm\|livebrainstorm" --exclude-dir=node_modules --exclude-dir=target --exclude-dir=docs --exclude-dir=gen --exclude-dir=.git --exclude=package-lock.json . | grep -v "legacyDrafts\|livebrainstorm\.draft\|livebrainstorm\.lastProject\|OLD_IDENTIFIER\|OLD_SERVICE\|app\.livebrainstorm\.desktop\|from LiveBrainstorm\|saved by LiveBrainstorm\|Upgrading from LiveBrainstorm\|old build"
```

Expected: every suite green, clippy silent, and the final grep prints nothing (the allowed survivors are the legacy-storage keys and the migration's own mentions of the old name).

Manual (needs a machine with an existing LiveBrainstorm install, Node 20.19+): `npm run tauri dev`. On first launch stderr shows `Live Spec: moved … app.livebrainstorm.desktop to … app.livespec.desktop` and, with a saved key, `Live Spec: carried the API key over from LiveBrainstorm`. Every tab, draft, history entry and brief is present; Settings shows the saved provider; the Usage panel shows past days. Open an old tab: its history shows the old notes with Faults as "Conflicts with code" and Directions; no run starts by itself. The window title reads "Live Spec — <tab>".

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: rename to Live Spec down to the identifier; move the old app folder and keychain keys on first launch; docs

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Self-review notes

- **Spec coverage:** Part 1 → Task 11 (names, folder move via Task 10, keychain). Part 2 → Tasks 1–3 (catalogue, guides, intro, rules, grouping, aliases, legacy Directions). Part 3 → Tasks 4–6 (Tab field, store, commands, analyze, context key, fact-check hook, create_tab copy). Part 4 → Tasks 7–9 (picker, Copy spec, templates with sections, starters, copy changes) and Task 11 (README, AGENTS). Testing section → each task's tests plus Task 11's manual run. Non-goals respected: no reordering, no export, no brief changes.
- **Type consistency:** `section_body(markdown, id)` takes ids everywhere (Tasks 2, 5). `section_groups(n, &chosen)` with `Vec<&'static SectionSpec>` (Tasks 2, 5, notes_live). `create_tab(window, root, sections)` (Tasks 4, 5, 6). `TabSessionDeps.analyze(…, instructions, sections, context, onEvent)` matches `api.analyze` (Tasks 5, 6). `applyTemplate(p, t, current)` returns `{ next, sections, undo: { instructions, sections } }` (Task 8 only).
- **Spec deviation recorded:** the `handoff` starter stores `["gaps", "ambiguities"]` because stored lists are normalized to catalogue order; the spec's table lists the same two sections.
