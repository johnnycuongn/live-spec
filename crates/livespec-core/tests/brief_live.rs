//! Builds this repo's brief for real with Claude Code (sonnet; uses the
//! subscription, several minutes): 1 explorer vs 4 explorers plus merge.
//!   cargo test -p livespec-core --test brief_live -- --ignored --nocapture

use std::path::Path;
use std::time::Instant;

use livespec_core::llm::{Chunk, ClaudeCli, Provider};
use livespec_core::project::{explore, scan};
use livespec_core::{agents, prompt, shell_env};

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
    let provider = Provider::ClaudeCli(ClaudeCli::new(binary, "sonnet", path_env));
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
