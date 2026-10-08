use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use ignore::WalkBuilder;

use super::{terms, Fnv};
use crate::{Error, Result};

pub const MAX_FILE_BYTES: u64 = 256 * 1024;
pub const MAX_INDEXED_FILES: usize = 6_000;
const MAX_TREE_PATHS: usize = 10_000;

/// Directories that are never source, even without a .gitignore.
const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "dist", "build", "out", ".next", ".nuxt", "vendor",
    "coverage", "__pycache__", ".venv", "venv", ".gradle", "Pods", "DerivedData",
];
const SKIP_FILES: &[&str] = &[
    "package-lock.json", "yarn.lock", "pnpm-lock.yaml", "Cargo.lock", "poetry.lock",
    "composer.lock", "Gemfile.lock", "bun.lockb", "go.sum",
];
const BINARY_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "icns", "bmp", "svgz", "pdf", "zip", "gz",
    "tgz", "xz", "7z", "rar", "jar", "wasm", "so", "dylib", "dll", "exe", "bin", "o", "a",
    "class", "woff", "woff2", "ttf", "otf", "eot", "mp3", "mp4", "mov", "webm", "wav",
    "gguf", "onnx", "pt", "sqlite", "db", "lockb",
];

#[derive(Debug)]
pub struct FileEntry {
    /// Forward-slash path relative to the project root.
    pub rel: String,
    pub size: u64,
    pub content: String,
    pub(crate) terms: HashMap<String, u32>,
}

#[derive(Debug)]
pub struct ProjectIndex {
    pub root: PathBuf,
    pub name: String,
    /// Indexed text files, sorted by path.
    pub files: Vec<FileEntry>,
    /// Every non-ignored path (including binaries/large files) for the tree view.
    pub tree: Vec<String>,
    /// Changes whenever any file's path, size or mtime changes.
    pub fingerprint: String,
    pub(crate) doc_freq: HashMap<String, u32>,
}

/// Walks the project respecting .gitignore/.ignore. Blocking: run it on a
/// blocking thread.
pub fn scan(root: &Path) -> Result<ProjectIndex> {
    if !root.is_dir() {
        return Err(Error::Precondition(format!("{} is not a folder", root.display())));
    }
    let root = root.canonicalize()?;
    let walker = WalkBuilder::new(&root)
        .hidden(true)
        .require_git(false) // honour .gitignore even outside a git repo
        .filter_entry(|e| {
            !(e.file_type().is_some_and(|t| t.is_dir())
                && SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
        })
        .build();

    let mut files = Vec::new();
    let mut tree = Vec::new();
    let mut stamps: Vec<(String, u64, u64)> = Vec::new();

    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(&root) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let Ok(meta) = entry.metadata() else { continue };
        let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
        stamps.push((rel.clone(), meta.len(), mtime));
        if tree.len() < MAX_TREE_PATHS {
            tree.push(rel.clone());
        }

        if files.len() >= MAX_INDEXED_FILES || !is_indexable(path, meta.len()) {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else { continue };
        if bytes.iter().take(8_000).any(|b| *b == 0) {
            continue; // binary without a telltale extension
        }
        let Ok(content) = String::from_utf8(bytes) else { continue };
        let terms = count_terms(&rel, &content);
        files.push(FileEntry { rel, size: meta.len(), content, terms });
    }

    files.sort_by(|a, b| a.rel.cmp(&b.rel));
    tree.sort();
    stamps.sort();

    let mut fp = Fnv::new();
    for (rel, size, mtime) in &stamps {
        fp.write(rel.as_bytes());
        fp.write(&size.to_le_bytes());
        fp.write(&mtime.to_le_bytes());
    }

    let mut doc_freq: HashMap<String, u32> = HashMap::new();
    for f in &files {
        for term in f.terms.keys() {
            *doc_freq.entry(term.clone()).or_default() += 1;
        }
    }

    let name = root.file_name().map_or_else(|| root.display().to_string(), |n| n.to_string_lossy().into_owned());
    Ok(ProjectIndex { root, name, files, tree, fingerprint: fp.hex(), doc_freq })
}

fn is_indexable(path: &Path, size: u64) -> bool {
    if size == 0 || size > MAX_FILE_BYTES {
        return false;
    }
    let name = path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
    if SKIP_FILES.contains(&name.as_ref()) || name.ends_with(".min.js") || name.ends_with(".map") {
        return false;
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    !BINARY_EXTS.contains(&ext.as_str())
}

fn count_terms(rel: &str, content: &str) -> HashMap<String, u32> {
    let mut map = HashMap::new();
    // Path terms count too: `billing/invoice.ts` is about invoices.
    for t in terms::tokens(rel).into_iter().chain(terms::tokens(content)) {
        *map.entry(t).or_default() += 1;
    }
    map
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let w = |p: &str, s: &str| {
            let path = dir.path().join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, s).unwrap();
        };
        w(".gitignore", "secret.txt\n");
        w("secret.txt", "do not index");
        w("README.md", "# Shop\nAn online shop with carts and invoices.");
        w("src/billing/invoice.ts", "export function createInvoice(cartId: string) {\n  // invoice totals\n}\n");
        w("src/cart/cart.ts", "export class CartStore { addItem() {} }\n");
        w("node_modules/x/index.js", "ignored");
        w("logo.png", "\u{89}PNG");
        dir
    }

    #[test]
    fn respects_ignores_and_skips_binaries() {
        let dir = fixture();
        let idx = scan(dir.path()).unwrap();
        let rels: Vec<_> = idx.files.iter().map(|f| f.rel.as_str()).collect();
        assert_eq!(rels, ["README.md", "src/billing/invoice.ts", "src/cart/cart.ts"]);
        assert!(idx.tree.contains(&"logo.png".to_string()));
        assert!(!idx.tree.iter().any(|p| p.contains("node_modules") || p == "secret.txt"));
    }

    #[test]
    fn fingerprint_tracks_changes() {
        let dir = fixture();
        let a = scan(dir.path()).unwrap().fingerprint;
        assert_eq!(a, scan(dir.path()).unwrap().fingerprint);
        std::fs::write(dir.path().join("src/new.ts"), "x").unwrap();
        assert_ne!(a, scan(dir.path()).unwrap().fingerprint);
    }
}
