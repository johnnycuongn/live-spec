//! "Checked in code": verifies the Faults the notes raise against the
//! project's own code, after the notes have streamed. Plain code finds the
//! evidence; Jev only judges whether it supports the claim (adapted from
//! TypeSafe's citation-check cookbook). Verdicts are signals, not proof:
//! Jev reads literally and struggles with behaviour spread over many files.

use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Jev, Options, Question};
use crate::project::retrieve::{best_line, file_score, query_weights, window};
use crate::project::{terms, ProjectIndex, Snippet};

pub const MAX_CLAIMS: usize = 6;
pub const CHECK_MIN_CONFIDENCE: f64 = 0.8;
/// Jev requests in flight at once; independent of the max-agents setting.
pub const JEV_CONCURRENCY: usize = 8;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
const EVIDENCE_CHARS: usize = 1_500;
const MIN_SPAN_CHARS: usize = 3;
/// A name found in more files than this never counts as specific, however
/// large the repo.
const MAX_SPECIFIC_FILES: usize = 8;
/// Extensions that make a backticked span a file path: `cart.ts` is a file,
/// `state.local` is code.
const SOURCE_EXTS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "json", "toml", "md", "css", "html", "py", "go", "java", "kt",
    "swift", "c", "h", "cpp", "rb", "php", "yml", "yaml", "sh", "sql",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Confirmed,
    Contradicted,
    Unclear,
    NotShown,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// A backticked span that reads as a file path: has a `/` or a known source
/// extension, and no spaces, quotes or parentheses (`app.get('/')` is code).
fn is_path(span: &str) -> bool {
    let plain = !span.contains(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | '(' | ')'));
    plain && (span.contains('/') || span.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && SOURCE_EXTS.contains(&ext)))
}

/// `rel` is `span` or ends with `/span`: whole path segments, so `cart.ts`
/// doesn't match `minicart.ts`.
fn path_matches(rel: &str, span: &str) -> bool {
    rel == span || rel.strip_suffix(span).is_some_and(|head| head.ends_with('/'))
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `span` occurs in `content` as a whole identifier: `user` is not found in
/// `userId` or `username`.
fn contains_ident(content: &str, span: &str) -> bool {
    content.match_indices(span).any(|(at, _)| {
        let before = content[..at].chars().next_back();
        let after = content[at + span.len()..].chars().next();
        !before.is_some_and(is_ident_char) && !after.is_some_and(is_ident_char)
    })
}

/// One backticked name from a claim and the files it was found in.
struct Span {
    text: String,
    hits: Vec<usize>,
    /// Matched as a file path, rather than as a name in file contents.
    is_file: bool,
}

/// Finds the code a claim refers to, with no model involved.
pub fn locate_evidence(index: &ProjectIndex, claim: &str) -> Evidence {
    // A name found in more files than this is too generic to point anywhere:
    // a quarter of the repo, but never more than MAX_SPECIFIC_FILES.
    let generic_above = (index.files.len() / 4).clamp(2, MAX_SPECIFIC_FILES);
    let mut spans: Vec<Span> = Vec::new();
    for text in code_spans(claim) {
        if is_path(&text) {
            let hits: Vec<usize> = (0..index.files.len()).filter(|&i| path_matches(&index.files[i].rel, &text)).collect();
            if !hits.is_empty() {
                spans.push(Span { text, hits, is_file: true });
                continue;
            }
        }
        // Not a file (or no such file): `res.json` and `state.local` are code.
        let hits: Vec<usize> = (0..index.files.len()).filter(|&i| contains_ident(&index.files[i].content, &text)).collect();
        if hits.len() <= generic_above {
            spans.push(Span { text, hits, is_file: false });
        }
    }
    if spans.is_empty() {
        return Evidence::Uncheckable;
    }
    let mut candidates: Vec<usize> = spans.iter().flat_map(|s| s.hits.iter().copied()).collect();
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
    let names: Vec<&str> = spans.iter().filter(|s| !s.is_file).map(|s| s.text.as_str()).collect();
    let (hits, line) = lines
        .iter()
        .enumerate()
        .map(|(n, l)| (names.iter().filter(|s| contains_ident(l, s)).count(), n))
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

/// Checks each Fault and reports through `on_event` as verdicts arrive.
/// Emits nothing when no Fault names code; otherwise `Checking` first, then
/// one `Result` per checkable claim, or a single `Notice` if Jev fails.
pub async fn run(jev: &Jev, index: &ProjectIndex, faults: &str, mut on_event: impl FnMut(CheckEvent)) {
    let (claims, total) = extract_claims(faults);
    let located: Vec<(String, Evidence)> = claims
        .into_iter()
        .map(|claim| {
            let evidence = locate_evidence(index, &claim);
            (claim, evidence)
        })
        .filter(|(_, evidence)| *evidence != Evidence::Uncheckable)
        .collect();
    if located.is_empty() {
        return;
    }
    on_event(CheckEvent::Checking { count: located.len(), total });

    let mut to_judge = Vec::new();
    for (claim, evidence) in located {
        match evidence {
            Evidence::Found(code, line) => to_judge.push((claim, code, line)),
            _ => on_event(CheckEvent::Result(CheckResult {
                claim,
                verdict: Verdict::NotFound,
                confidence: None,
                path: None,
                line: None,
            })),
        }
    }

    let questions = BTreeMap::from([("relation".to_owned(), question())]);
    let questions = &questions;
    let mut answers = std::pin::pin!(stream::iter(to_judge)
        .map(|(claim, code, line)| async move {
            let answer = jev.ask(&state(&claim, &code), questions, REQUEST_TIMEOUT).await;
            (claim, code, line, answer)
        })
        .buffer_unordered(JEV_CONCURRENCY));
    while let Some((claim, code, line, answer)) = answers.next().await {
        let picked = match answer {
            Ok(response) => response.choice("relation").map(|(choice, confidence)| (verdict(choice, confidence), confidence)),
            Err(e) => return on_event(CheckEvent::Notice(format!("Couldn't check the notes against the code: {e}"))),
        };
        let Some((verdict, confidence)) = picked else {
            return on_event(CheckEvent::Notice(
                "Couldn't check the notes against the code: Jev returned an unexpected answer.".into(),
            ));
        };
        on_event(CheckEvent::Result(CheckResult {
            claim,
            verdict,
            confidence: Some(confidence),
            path: Some(code.path),
            line: Some(line),
        }));
    }
}

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
    fn check_results_round_trip_through_json() {
        let r = CheckResult {
            claim: "`x` is wrong".into(),
            verdict: Verdict::NotShown,
            confidence: Some(0.9),
            path: Some("src/x.rs".into()),
            line: Some(3),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert_eq!(json, r#"{"claim":"`x` is wrong","verdict":"notShown","confidence":0.9,"path":"src/x.rs","line":3}"#);
        assert_eq!(serde_json::from_str::<CheckResult>(&json).unwrap(), r, "saved checks read back unchanged");
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

    use crate::jev::test_server::{serve, Canned};
    use crate::jev::{Jev, DEFAULT_MODEL};

    const SUPPORTS: &str = r#"{"model":"jev-1.13.0","answers":{"relation":{"type":"choice","choice":"supports","probabilities":{"supports":0.95,"contradicts":0.03,"says_nothing":0.02},"confidence":0.93}},"usage":{"input_tokens":300,"output_tokens":20}}"#;

    fn judge(base_url: String) -> Jev {
        Jev { base_url, ..Jev::new("test-key".into(), DEFAULT_MODEL.into(), reqwest::Client::new()) }
    }

    async fn events(jev: &Jev, faults: &str) -> Vec<CheckEvent> {
        let idx = scan(fixture().path()).unwrap();
        let mut out = Vec::new();
        run(jev, &idx, faults, |e| out.push(e)).await;
        out
    }

    #[tokio::test]
    async fn reports_verdicts_as_they_arrive() {
        let (url, _) = serve(vec![Canned::new(200, SUPPORTS)]).await;
        let faults = "- `createInvoice` never applies discounts\n- `applyCoupon` is called twice\n- Totals look fine";
        assert_eq!(
            events(&judge(url), faults).await,
            vec![
                CheckEvent::Checking { count: 2, total: 3 },
                CheckEvent::Result(CheckResult {
                    claim: "`applyCoupon` is called twice".into(),
                    verdict: Verdict::NotFound,
                    confidence: None,
                    path: None,
                    line: None,
                }),
                CheckEvent::Result(CheckResult {
                    claim: "`createInvoice` never applies discounts".into(),
                    verdict: Verdict::Confirmed,
                    confidence: Some(0.93),
                    path: Some("src/billing/invoice.ts".into()),
                    line: Some(1),
                }),
            ]
        );
    }

    #[tokio::test]
    async fn a_jev_failure_becomes_one_notice() {
        let (url, _) = serve(vec![Canned::new(401, "{}")]).await;
        let got = events(&judge(url), "- `createInvoice` never applies discounts").await;
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0], CheckEvent::Checking { count: 1, total: 1 });
        assert!(matches!(&got[1], CheckEvent::Notice(m) if m.contains("TypeSafe key rejected")), "{got:?}");
    }

    #[tokio::test]
    async fn nothing_to_check_emits_nothing() {
        let (url, hits) = serve(vec![]).await;
        assert!(events(&judge(url.clone()), "- Nothing so far.").await.is_empty());
        assert!(events(&judge(url), "- Totals look fine").await.is_empty());
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn a_name_in_a_few_files_of_a_small_repo_is_still_too_generic() {
        let dir = repo(&[
            ("a.ts", "const data = 1;\n"),
            ("b.ts", "const data = 2;\n"),
            ("c.ts", "const data = 3;\n"),
            ("d.ts", "export const other = 4;\n"),
        ]);
        let idx = scan(dir.path()).unwrap();
        assert_eq!(locate_evidence(&idx, "`data` is never validated"), Evidence::Uncheckable);
    }

    #[test]
    fn a_name_in_many_files_of_a_big_repo_is_too_generic() {
        let paths: Vec<String> = (0..40).map(|i| format!("src/m{i}.ts")).collect();
        let bodies: Vec<String> = (0..40)
            .map(|i| if i < 12 { "const config = load();\n".to_owned() } else { format!("export const v{i} = {i};\n") })
            .collect();
        let files: Vec<(&str, &str)> = paths.iter().zip(&bodies).map(|(p, b)| (p.as_str(), b.as_str())).collect();
        let idx = scan(repo(&files).path()).unwrap();
        assert_eq!(locate_evidence(&idx, "`config` is read twice"), Evidence::Uncheckable);
    }

    #[test]
    fn names_match_whole_identifiers_only() {
        let dir = repo(&[("a.ts", "const userId = 1;\n"), ("b.ts", "export const other = 2;\n")]);
        let idx = scan(dir.path()).unwrap();
        assert_eq!(locate_evidence(&idx, "`user` is never validated"), Evidence::NotFound);
    }

    #[test]
    fn code_that_looks_like_a_file_name_is_searched_as_code() {
        let dir = repo(&[("src/api.ts", "app.get('/'); // res.json({ ok: true })\n"), ("src/b.ts", "export const b = 1;\n")]);
        let idx = scan(dir.path()).unwrap();
        for claim in ["`res.json` is called without a status", "`app.get('/')` has no auth"] {
            let found = locate_evidence(&idx, claim);
            assert!(matches!(found, Evidence::Found(ref code, 1) if code.path == "src/api.ts"), "{claim}: {found:?}");
        }
    }

    #[test]
    fn a_file_name_matches_whole_path_segments_only() {
        let dir = repo(&[("src/minicart.ts", "export const mini = 1;\n")]);
        let idx = scan(dir.path()).unwrap();
        assert_eq!(locate_evidence(&idx, "`cart.ts` has no removeItem"), Evidence::NotFound);
        let real = repo(&[("src/cart.ts", "export const cart = 1;\n"), ("src/minicart.ts", "export const mini = 1;\n")]);
        let idx = scan(real.path()).unwrap();
        assert!(matches!(locate_evidence(&idx, "`cart.ts` has no removeItem"), Evidence::Found(ref c, _) if c.path == "src/cart.ts"));
    }
}
