//! Prompt construction. Everything the model sees is built here, so tuning
//! LiveBrainstorm's behaviour is a one-file change.

use crate::llm::{ChatRequest, Provider};
use crate::project::{digest, ProjectIndex, Snippet};

/// Headings the UI splits the analysis on. Keep in sync with `src/lib/sections.ts`.
pub const SECTIONS: [&str; 4] = ["Intent", "Faults", "Directions", "Questions"];

const ANALYSIS_SYSTEM: &str = "\
You are LiveBrainstorm, a sharp brainstorming partner sitting beside a software engineer's draft. \
The draft is unfinished thinking typed in real time about the project described in <project>. \
Your job is to make their thinking better, not to finish their writing.

Respond in Markdown with exactly these four sections, in this order, and nothing else:
## Intent
What you think they are trying to do and why, in one or two bullets. If it's ambiguous, say what is ambiguous.
## Faults
Problems with the draft as it stands: conflicts with how the project actually works, things that already exist, \
wrong assumptions, missing edge cases, risky trade-offs. Cite file paths when the project context supports it. \
If nothing is worth raising, write \"- Nothing so far.\"
## Directions
Two or three concrete next moves or alternative approaches, each with its main trade-off.
## Questions
One to three questions whose answers would most change the design.

Rules: bullets only, at most 3 per section, each under 35 words. Ground claims in the project context; \
when the context doesn't cover something, say so instead of guessing. Never restate the draft. Do not use tools.";

const BRIEF_FORMAT: &str = "\
Write a project brief in Markdown, at most {WORDS} words, with these sections: \
## Purpose, ## Architecture (layers, runtime, key dependencies), \
## Modules (`path`: responsibility, one line each), ## Core flows (how data and requests move), \
## Conventions (patterns the code follows), ## Existing features, ## Gaps and TODOs, \
## Glossary (domain terms). State only what the code supports and cite paths. Output only the brief.";

const BRIEF_EXPLORE: &str = "\
You are mapping a codebase so a brainstorming assistant can reason about it later without reading the code. \
Explore the repository in the current working directory with read-only tools: start with the tree, READMEs, \
manifests and entry points, then follow the main flows. Don't read every file.";

const MAX_DRAFT_CHARS: usize = 12_000;

pub fn brief_request(index: &ProjectIndex, provider: &Provider) -> ChatRequest {
    let local = matches!(provider, Provider::OpenAiCompat(_));
    let format = BRIEF_FORMAT.replace("{WORDS}", if local { "600" } else { "1,200" });

    if provider.can_explore_repo() {
        return ChatRequest {
            system: format!("{BRIEF_EXPLORE}\n\n{format}"),
            user: format!("Map the project \"{}\" and write the brief.", index.name),
            max_tokens: 4_000,
            cwd: Some(index.root.clone()),
            allow_repo_tools: true,
        };
    }

    let budget = provider.context_budget_chars() * 3 / 4;
    ChatRequest {
        system: format!("You are mapping a codebase from the excerpts provided. {format}"),
        user: format!("<project name=\"{}\">\n{}\n</project>", index.name, digest::build(index, budget)),
        max_tokens: if local { 1_200 } else { 3_000 },
        cwd: None,
        allow_repo_tools: false,
    }
}

pub fn analysis_request(
    draft: &str,
    index: Option<&ProjectIndex>,
    brief: Option<&str>,
    snippets: &[Snippet],
    provider: &Provider,
) -> ChatRequest {
    let budget = provider.context_budget_chars();
    let mut project = String::new();

    if let Some(idx) = index {
        project.push_str(&format!("<project name=\"{}\">\n", idx.name));
        match brief {
            Some(b) => project.push_str(&format!("<brief>\n{}\n</brief>\n", clip(b, budget * 3 / 5))),
            None => project.push_str("<brief>Not generated yet. Rely on the code excerpts.</brief>\n"),
        }
        if !snippets.is_empty() {
            project.push_str("<relevant_code>\n");
            for s in snippets {
                project.push_str(&format!(
                    "<file path=\"{}\" start_line=\"{}\">\n{}\n</file>\n",
                    s.path, s.start_line, s.text
                ));
            }
            project.push_str("</relevant_code>\n");
        }
        project.push_str("</project>\n\n");
    } else {
        project.push_str("<project>No project selected: brainstorm on the draft alone.</project>\n\n");
    }

    ChatRequest {
        system: ANALYSIS_SYSTEM.to_owned(),
        user: format!("{project}<draft>\n{}\n</draft>", shape_draft(draft)),
        max_tokens: 900,
        // CWD lets Claude Code pick up the project's CLAUDE.md conventions.
        cwd: index.map(|i| i.root.clone()),
        allow_repo_tools: false,
    }
}

/// Which of `SECTIONS` a heading line opens, as tolerant as
/// `src/lib/sections.ts`: "## Faults", "### Questions for you", "**Faults:**".
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
    let word = rest.trim_start().split(|c: char| !c.is_ascii_alphabetic()).next()?.to_lowercase();
    SECTIONS.iter().copied().find(|s| {
        let s = s.to_lowercase();
        word == s || word == s.trim_end_matches('s')
    })
}

/// The body under one section heading (e.g. "Faults"), if present. Mirrors
/// `parseSections` in `src/lib/sections.ts`; keep the two in sync.
pub fn section_body(markdown: &str, name: &str) -> Option<String> {
    let mut body: Option<String> = None;
    let mut current = None;
    for line in markdown.lines() {
        if let Some(key) = heading_key(line) {
            current = Some(key);
            if key == name {
                body.get_or_insert_with(String::new);
            }
            continue;
        }
        if current == Some(name) {
            if let Some(b) = body.as_mut() {
                b.push_str(line);
                b.push('\n');
            }
        }
    }
    body.map(|b| b.trim().to_owned())
}

/// Very long drafts: keep the opening (the framing) and the tail (where the
/// user is typing now), drop the middle.
fn shape_draft(draft: &str) -> String {
    shape_draft_to(draft, MAX_DRAFT_CHARS)
}

/// `shape_draft` at any size: the first sixth is the opening, the rest the
/// tail. Jev uses it to keep its `state` small.
pub fn shape_draft_to(draft: &str, max_chars: usize) -> String {
    if draft.len() <= max_chars {
        return draft.to_owned();
    }
    let head_end = floor_boundary(draft, max_chars / 6);
    let tail_start = floor_boundary(draft, draft.len() - (max_chars - max_chars / 6));
    format!("{}\n[... middle of draft omitted ...]\n{}", &draft[..head_end], &draft[tail_start..])
}

fn clip(s: &str, max: usize) -> &str {
    &s[..floor_boundary(s, max.min(s.len()))]
}

fn floor_boundary(s: &str, i: usize) -> usize {
    (0..=i.min(s.len())).rev().find(|j| s.is_char_boundary(*j)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::OpenAiCompat;

    fn local() -> Provider {
        Provider::OpenAiCompat(OpenAiCompat {
            base_url: "http://x".into(),
            model: "m".into(),
            context_tokens: 4096,
            http: reqwest::Client::new(),
        })
    }

    #[test]
    fn long_drafts_keep_head_and_tail() {
        let draft = format!("HEAD{}TAIL", "é".repeat(20_000));
        let shaped = shape_draft(&draft);
        assert!(shaped.starts_with("HEAD") && shaped.ends_with("TAIL"));
        assert!(shaped.len() < MAX_DRAFT_CHARS + 100);
    }

    #[test]
    fn analysis_without_project_still_works() {
        let req = analysis_request("add caching to search", None, None, &[], &local());
        assert!(req.user.contains("No project selected") && req.cwd.is_none());
        for s in SECTIONS {
            assert!(req.system.contains(&format!("## {s}")));
        }
    }

    #[test]
    fn section_body_matches_the_ui_parser() {
        let md = "## Intent\n- add caching\n## Faults\n- `cache.ts` exists\n- second\n## Directions\n- reuse it";
        assert_eq!(section_body(md, "Faults").as_deref(), Some("- `cache.ts` exists\n- second"));
        assert_eq!(section_body("**Faults:**\n- x\n### Questions for you\n- y", "Faults").as_deref(), Some("- x"));
        assert_eq!(section_body("Sure, here you go\n## Intent\n- a", "Faults"), None);
    }

    #[test]
    fn drafts_shape_to_any_size() {
        let draft = format!("{}TAIL", "é".repeat(5_000));
        let shaped = shape_draft_to(&draft, 4_000);
        assert!(shaped.starts_with('é') && shaped.ends_with("TAIL"));
        assert!(shaped.len() < 4_100, "{}", shaped.len());
        assert_eq!(shape_draft_to("short", 4_000), "short");
    }
}
