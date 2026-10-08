//! A static, budgeted view of the project for providers that can't explore
//! the repo themselves (API, local models). Priority: tree, then manifests and
//! docs, then the heads of the most "central" source files.

use super::{FileEntry, ProjectIndex};

const KEY_FILES: &[&str] = &[
    "readme.md", "readme", "claude.md", "agents.md", "architecture.md", "package.json",
    "cargo.toml", "pyproject.toml", "go.mod", "pom.xml", "build.gradle", "build.gradle.kts",
    "composer.json", "gemfile", "tauri.conf.json", "docker-compose.yml", "schema.prisma",
];
const CENTRAL_STEMS: &[&str] = &[
    "main", "lib", "index", "app", "mod", "server", "routes", "router", "schema", "types",
    "models", "api", "store", "config",
];
const HEAD_LINES: usize = 80;

pub fn build(index: &ProjectIndex, budget: usize) -> String {
    let mut out = String::new();
    push_tree(&mut out, index, budget / 5);

    let (key, rest): (Vec<&FileEntry>, Vec<&FileEntry>) =
        index.files.iter().partition(|f| KEY_FILES.contains(&file_name(&f.rel).to_lowercase().as_str()));

    out.push_str("\n## Key files\n");
    for f in key {
        if !push_file(&mut out, f, usize::MAX, budget / 8, budget) {
            break;
        }
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

fn push_tree(out: &mut String, index: &ProjectIndex, budget: usize) {
    out.push_str(&format!("## File tree ({} files)\n", index.tree.len()));
    let full: String = index.tree.iter().map(|p| format!("{p}\n")).collect();
    if full.len() <= budget {
        out.push_str(&full);
        return;
    }
    // Too big: summarise by top-level directory.
    let mut dirs: std::collections::BTreeMap<&str, usize> = Default::default();
    for p in &index.tree {
        let top = p.split_once('/').map_or(p.as_str(), |(d, _)| d);
        *dirs.entry(top).or_default() += 1;
    }
    for (d, n) in dirs {
        out.push_str(&format!("{d}{}\n", if n > 1 { format!("/ ({n} files)") } else { String::new() }));
    }
}

/// Returns false once the total budget is exhausted.
fn push_file(out: &mut String, f: &FileEntry, max_lines: usize, max_chars: usize, budget: usize) -> bool {
    let room = budget.saturating_sub(out.len());
    if room < 200 {
        return false;
    }
    let body: String = f.content.lines().take(max_lines).collect::<Vec<_>>().join("\n");
    let limit = max_chars.min(room - 100);
    let cut = (0..=limit.min(body.len())).rev().find(|i| body.is_char_boundary(*i)).unwrap_or(0);
    out.push_str(&format!("\n<file path=\"{}\">\n{}\n</file>\n", f.rel, &body[..cut]));
    true
}

fn file_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// Lower = more central: shallow paths and conventional entry-point names first.
fn centrality(rel: &str) -> (usize, usize, String) {
    let depth = rel.matches('/').count();
    let stem = file_name(rel).split('.').next().unwrap_or("").to_lowercase();
    let named = usize::from(!CENTRAL_STEMS.contains(&stem.as_str()));
    let is_test = usize::from(rel.contains("test") || rel.contains("spec"));
    (is_test * 10 + named, depth, rel.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::scan::{scan, tests::fixture};

    #[test]
    fn stays_within_budget_and_includes_readme() {
        let idx = scan(fixture().path()).unwrap();
        let d = build(&idx, 2_000);
        assert!(d.len() <= 2_000, "{}", d.len());
        assert!(d.contains("README.md") && d.contains("An online shop"));
    }
}
