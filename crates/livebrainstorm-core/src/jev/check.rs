//! "Checked in code": verifies the Faults the notes raise against the
//! project's own code, after the notes have streamed. Plain code finds the
//! evidence; Jev only judges whether it supports the claim (adapted from
//! TypeSafe's citation-check cookbook). Verdicts are signals, not proof:
//! Jev reads literally and struggles with behaviour spread over many files.

use serde::Serialize;
use serde_json::{json, Value};

use super::{Options, Question};
use crate::project::retrieve::{best_line, file_score, query_weights, window};
use crate::project::{terms, ProjectIndex, Snippet};

pub const MAX_CLAIMS: usize = 6;
pub const CHECK_MIN_CONFIDENCE: f64 = 0.8;
/// Jev requests in flight at once; independent of the max-agents setting.
pub const JEV_CONCURRENCY: usize = 8;
const EVIDENCE_CHARS: usize = 1_500;
const MIN_SPAN_CHARS: usize = 3;
/// Extensions that make a backticked span a file path: `cart.ts` is a file,
/// `state.local` is code.
const SOURCE_EXTS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "json", "toml", "md", "css", "html", "py", "go", "java", "kt",
    "swift", "c", "h", "cpp", "rb", "php", "yml", "yaml", "sh", "sql",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Confirmed,
    Contradicted,
    Unclear,
    NotShown,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub claim: String,
    pub verdict: Verdict,
    pub confidence: Option<f64>,
    pub path: Option<String>,
    /// 1-based line the evidence centres on.
    pub line: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CheckEvent {
    /// `count` of the `total` Faults name code that can be checked.
    Checking { count: usize, total: usize },
    Result(CheckResult),
    /// Jev failed; the remaining claims are dropped. At most one per run.
    Notice(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Evidence {
    /// The claim names no code, or only names too generic to locate.
    Uncheckable,
    /// It names code that isn't in the indexed files.
    NotFound,
    /// The window to judge, and the 1-based line it centres on.
    Found(Snippet, usize),
}

/// The Faults bullets (up to MAX_CLAIMS) and how many bullets there were,
/// skipping the prompt's "Nothing so far."
pub fn extract_claims(faults: &str) -> (Vec<String>, usize) {
    let bullets: Vec<String> = faults
        .lines()
        .filter_map(bullet_text)
        .filter(|t| !t.to_lowercase().starts_with("nothing so far"))
        .map(str::to_owned)
        .collect();
    let total = bullets.len();
    (bullets.into_iter().take(MAX_CLAIMS).collect(), total)
}

fn bullet_text(line: &str) -> Option<&str> {
    let t = line.trim_start();
    let rest = ["- ", "* ", "• "].iter().find_map(|m| t.strip_prefix(m)).or_else(|| {
        let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        (digits > 0).then(|| &t[digits..]).and_then(|r| r.strip_prefix(". ").or_else(|| r.strip_prefix(") ")))
    })?;
    Some(rest.trim()).filter(|r| !r.is_empty())
}

/// Backticked names and paths in a claim, minus ones too short or generic
/// to locate. A trailing `:42` line number is dropped.
pub fn code_spans(claim: &str) -> Vec<String> {
    claim
        .split('`')
        .skip(1)
        .step_by(2)
        .map(|s| strip_line_suffix(s.trim()))
        .filter(|s| s.chars().count() >= MIN_SPAN_CHARS && !terms::is_stopword(&s.to_lowercase()))
        .map(str::to_owned)
        .collect()
}

fn strip_line_suffix(s: &str) -> &str {
    match s.rsplit_once(':') {
        Some((head, line)) if !head.is_empty() && !line.is_empty() && line.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => s,
    }
}

fn is_path(span: &str) -> bool {
    span.contains('/') || span.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && SOURCE_EXTS.contains(&ext))
}

/// Finds the code a claim refers to, with no model involved.
pub fn locate_evidence(index: &ProjectIndex, claim: &str) -> Evidence {
    // A name found in more files than this is too generic to point anywhere.
    let generic_above = (index.files.len() / 4).max(3);
    let mut spans: Vec<(String, Vec<usize>)> = Vec::new();
    for span in code_spans(claim) {
        let path = is_path(&span);
        let hits: Vec<usize> = (0..index.files.len())
            .filter(|&i| {
                let f = &index.files[i];
                if path { f.rel.ends_with(span.as_str()) } else { f.content.contains(span.as_str()) }
            })
            .collect();
        if path || hits.len() <= generic_above {
            spans.push((span, hits));
        }
    }
    if spans.is_empty() {
        return Evidence::Uncheckable;
    }
    let mut candidates: Vec<usize> = spans.iter().flat_map(|(_, hits)| hits.iter().copied()).collect();
    candidates.sort_unstable();
    candidates.dedup();

    // Best TF-IDF match for the claim; ties keep path order.
    let weights = query_weights(claim);
    let mut best: Option<(f64, usize)> = None;
    for i in candidates {
        let score = file_score(index, i, &weights);
        if best.is_none_or(|(top, _)| score > top) {
            best = Some((score, i));
        }
    }
    let Some((_, i)) = best else { return Evidence::NotFound };

    let file = &index.files[i];
    let lines: Vec<&str> = file.content.lines().collect();
    let names: Vec<&str> = spans.iter().filter(|(s, _)| !is_path(s)).map(|(s, _)| s.as_str()).collect();
    let (hits, line) = lines
        .iter()
        .enumerate()
        .map(|(n, l)| (names.iter().filter(|s| l.contains(**s)).count(), n))
        .fold((0, 0), |acc, x| if x.0 > acc.0 { x } else { acc });
    let center = if hits > 0 { line } else { best_line(&lines, &weights) };
    Evidence::Found(window(&file.rel, &lines, center, EVIDENCE_CHARS), center + 1)
}

/// The question, worded as in TypeSafe's citation-check cookbook.
pub fn question() -> Question {
    Question::Choice {
        instructions: json!("How does `code` relate to `claim`?"),
        criteria: Options(vec![
            ("supports".into(), json!("The code shows the claim is true, or directly implies it.")),
            ("contradicts".into(), json!("The code shows the claim is false, or directly implies the opposite.")),
            ("says_nothing".into(), json!("The code does not address the claim either way.")),
        ]),
    }
}

pub fn state(claim: &str, code: &Snippet) -> Value {
    json!({ "claim": claim, "code": { "path": code.path, "start_line": code.start_line, "text": code.text } })
}

pub fn verdict(choice: &str, confidence: f64) -> Verdict {
    match choice {
        "says_nothing" => Verdict::NotShown,
        "supports" if confidence >= CHECK_MIN_CONFIDENCE => Verdict::Confirmed,
        "contradicts" if confidence >= CHECK_MIN_CONFIDENCE => Verdict::Contradicted,
        _ => Verdict::Unclear,
    }
}

#[cfg(test)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::scan;
    use crate::project::scan::tests::fixture;

    fn repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, body) in files {
            let p = dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        dir
    }

    #[test]
    fn extracts_bullets_and_skips_nothing_so_far() {
        let (claims, total) = extract_claims("- `a` is wrong\n- Nothing so far.\n* second\n1. third\nprose");
        assert_eq!(claims, ["`a` is wrong", "second", "third"]);
        assert_eq!(total, 3);
    }

    #[test]
    fn spans_drop_short_generic_names_and_line_numbers() {
        assert_eq!(code_spans("`createInvoice` uses `id`, `self` and `src/cart/cart.ts:12`"), ["createInvoice", "src/cart/cart.ts"]);
        assert_eq!(code_spans("`Error::Provider` is returned"), ["Error::Provider"]);
    }

    #[test]
    fn finds_the_named_function() {
        let idx = scan(fixture().path()).unwrap();
        match locate_evidence(&idx, "`createInvoice` never applies discounts") {
            Evidence::Found(code, line) => {
                assert_eq!(code.path, "src/billing/invoice.ts");
                assert_eq!(line, 1);
                assert!(code.text.contains("createInvoice"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn finds_a_named_file() {
        let idx = scan(fixture().path()).unwrap();
        let found = locate_evidence(&idx, "`src/cart/cart.ts:3` has no removeItem");
        assert!(matches!(found, Evidence::Found(ref code, _) if code.path == "src/cart/cart.ts"), "{found:?}");
    }

    #[test]
    fn unknown_names_are_not_found_and_prose_is_uncheckable() {
        let idx = scan(fixture().path()).unwrap();
        assert_eq!(locate_evidence(&idx, "`applyCoupon` is called twice"), Evidence::NotFound);
        assert_eq!(locate_evidence(&idx, "Discounts are never applied"), Evidence::Uncheckable);
    }

    #[test]
    fn names_found_almost_everywhere_are_too_generic() {
        let paths: Vec<String> = (0..8).map(|i| format!("src/f{i}.ts")).collect();
        let files: Vec<(&str, &str)> = paths.iter().map(|p| (p.as_str(), "const data = load();\n")).collect();
        let idx = scan(repo(&files).path()).unwrap();
        assert_eq!(locate_evidence(&idx, "`data` is never validated"), Evidence::Uncheckable);
    }

    #[test]
    fn dotted_names_are_code_not_paths() {
        let dir = repo(&[("src/state.rs", "let guard = state.local.lock();\n"), ("src/other.rs", "fn main() {}\n")]);
        let idx = scan(dir.path()).unwrap();
        let found = locate_evidence(&idx, "`state.local` is held across an await");
        assert!(matches!(found, Evidence::Found(ref code, 1) if code.path == "src/state.rs"), "{found:?}");
    }

    #[test]
    fn maps_answers_to_verdicts() {
        assert_eq!(verdict("supports", 0.93), Verdict::Confirmed);
        assert_eq!(verdict("supports", 0.6), Verdict::Unclear);
        assert_eq!(verdict("contradicts", 0.85), Verdict::Contradicted);
        assert_eq!(verdict("says_nothing", 0.99), Verdict::NotShown);
        assert_eq!(verdict("unexpected", 0.99), Verdict::Unclear);
    }

    #[test]
    fn question_lists_supports_first() {
        let text = serde_json::to_string(&question()).unwrap();
        let pos = |s: &str| text.find(s).unwrap();
        assert!(pos("\"supports\"") < pos("\"contradicts\"") && pos("\"contradicts\"") < pos("\"says_nothing\""));
    }
}
