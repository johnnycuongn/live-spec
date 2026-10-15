//! Building the brief with parallel explorers: split the indexed files into
//! balanced folder slices, one per agent; collect each agent's notes; name
//! what nobody mapped. The shell runs the agents; this module stays pure.

use std::collections::BTreeMap;

use super::digest::is_key_file;
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

/// Up to `n` slices of roughly equal size. Root-level README and manifest
/// files belong to none of them (the merge reads them in the overview);
/// other root files are explored like folders. Fewer than
/// `MIN_FILES_TO_SPLIT` files, or `n <= 1`, gives a single slice.
pub fn slices(index: &ProjectIndex, n: usize) -> Vec<Slice> {
    // (prefix -> files, bytes) for every first-level folder.
    let mut groups: BTreeMap<String, (usize, u64)> = BTreeMap::new();
    for f in &index.files {
        let key = match f.rel.split_once('/') {
            Some((top, _)) => format!("{top}/"),
            // README and manifests reach the merge through the overview.
            None if is_key_file(&f.rel) => continue,
            // Other root files (main.go, app.py) are explored like a folder.
            None => f.rel.clone(),
        };
        let entry = groups.entry(key).or_default();
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

    #[test]
    fn root_source_files_are_explored_not_dropped() {
        // A Go service: 30 .go files at the root, plus internal/, cmd/, docs/ and a README.
        let mut files: Vec<(String, usize)> = vec![("README.md".into(), 500), ("go.mod".into(), 100)];
        for i in 0..30 {
            files.push((format!("server{i}.go"), 2_000));
        }
        for i in 0..8 {
            files.push((format!("internal/store{i}.go"), 1_000));
        }
        files.push(("cmd/main.go".into(), 500));
        files.push(("cmd/cli.go".into(), 500));
        for i in 0..5 {
            files.push((format!("docs/d{i}.md"), 300));
        }
        let refs: Vec<(&str, usize)> = files.iter().map(|(p, s)| (p.as_str(), *s)).collect();
        let idx = scan(repo(&refs).path()).unwrap();
        let s = slices(&idx, 4);
        assert!(s.len() > 1);
        for i in 0..30 {
            let rel = format!("server{i}.go");
            assert_eq!(s.iter().filter(|slice| in_slice(&rel, &slice.prefixes)).count(), 1, "{rel}");
        }
        for key in ["README.md", "go.mod"] {
            assert!(s.iter().all(|slice| !in_slice(key, &slice.prefixes)), "{key} stays with the merge's overview");
        }
    }
}
