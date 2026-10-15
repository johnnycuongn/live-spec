//! Runs Claude Code for real (uses the subscription): times complete notes
//! for this repo with 1 lane and with 4 lanes, alternating, twice each.
//!   cargo test -p livespec-core --test notes_live -- --ignored --nocapture

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use livespec_core::llm::{Chunk, ClaudeCli, Provider};
use livespec_core::project::{retrieve, scan};
use livespec_core::{agents, analysis, prompt, shell_env};

#[tokio::test]
#[ignore = "runs Claude Code and uses the subscription"]
async fn four_lanes_vs_one() {
    let path_env = shell_env::login_shell_path();
    let Some(binary) = shell_env::find_executable("claude", path_env.as_deref()) else {
        eprintln!("skipped: claude not found");
        return;
    };
    let provider = Provider::ClaudeCli(ClaudeCli::new(binary, "haiku", path_env));
    let index = scan(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))).unwrap();
    let draft = "Every pause spawns a fresh claude -p process from ClaudeCli::stream. I want to keep one process alive, reuse it across analyze calls, and kill it when replace_task aborts the old analysis.";
    let snippets = retrieve(&index, draft, 5, 30_000);

    for n in [1, 4, 1, 4] {
        let chosen = prompt::resolve(&[]);
        let groups = prompt::section_groups(n, &chosen);
        let first_text: Mutex<Option<Duration>> = Mutex::new(None);
        let started = Instant::now();
        let mut lanes = Vec::new();
        for sections in &groups {
            let req = prompt::analysis_request(draft, Some(&index), None, &snippets, &provider, sections, "");
            let (provider, first_text) = (&provider, &first_text);
            lanes.push(async move {
                let mut sink = |chunk: Chunk| {
                    if matches!(chunk, Chunk::Text(_)) {
                        first_text.lock().unwrap().get_or_insert(started.elapsed());
                    }
                };
                let result = provider.stream(&req, &mut sink).await;
                (started.elapsed(), result.map(|text| (text.len(), text)))
            });
        }
        let done = agents::run_limited(n, lanes).await;
        let per_lane: Vec<String> = done
            .iter()
            .zip(&groups)
            .map(|((at, r), g)| format!("{}={}ms/{}ch", g.iter().map(|s| s.id).collect::<Vec<_>>().join("+"), at.as_millis(), r.as_ref().map_or(0, |(len, _)| *len)))
            .collect();
        let results = done.into_iter().map(|(_, r)| r.map(|(_, text)| text)).collect();
        let (text, notices) = analysis::merge_lanes(&groups, results).expect("notes");
        let complete = started.elapsed();
        let missing: Vec<_> = chosen.iter().filter(|s| prompt::section_body(&text, s.id).is_none()).map(|s| s.heading).collect();
        eprintln!(
            "lanes={n}  first text {:>5} ms  complete {:>5} ms  missing={missing:?}  notices={notices:?}\n          per lane: {}",
            first_text.lock().unwrap().map_or(0, |d| d.as_millis()),
            complete.as_millis(),
            per_lane.join("  ")
        );
    }
}
