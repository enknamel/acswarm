//! What the gates beside this file read: every `.rs` file a crate compiles, with its
//! `#[cfg(test)]` items cut out. One answer to "which code is production", so the length gate
//! and the naming gate measure the same thing. Reads files only and needs no game data.

use std::fs;
use std::path::{Path, PathBuf};

/// Test code, not production: a split-out `#[cfg(test)] mod tests;` and the offline builders
/// tests put round a `Client`, which the crate compiles only for `test` or the `testkit` feature.
const TEST_FILES: &[&str] = &["tests.rs", "testkit.rs"];

/// Directories holding tests and benchmarks rather than the crate's own code.
const TEST_DIRS: &[&str] = &["target", "tests", "benches"];

/// One file a crate compiles.
pub struct Source {
    /// Its path from the workspace root, as a failure should name it.
    pub shown: String,
    pub text: String,
}

fn workspace() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/ac-client sits two below the workspace")
        .to_path_buf()
}

fn under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !TEST_DIRS.contains(&name.as_str()) {
                under(&path, out);
            }
        } else if name.ends_with(".rs") && !TEST_FILES.contains(&name.as_str()) {
            out.push(path);
        }
    }
}

/// Every file a crate compiles, under `crates/`, `bins/`, `examples/` and `xtask/`, by path.
pub fn sources() -> Vec<Source> {
    let root = workspace();
    let mut files = Vec::new();
    for base in ["crates", "bins", "examples", "xtask"] {
        under(&root.join(base), &mut files);
    }
    let mut out: Vec<Source> = files
        .iter()
        .map(|file| Source {
            shown: file
                .strip_prefix(&root)
                .unwrap_or(file)
                .to_string_lossy()
                .to_string(),
            text: fs::read_to_string(file).unwrap_or_default(),
        })
        .collect();
    out.sort_by(|a, b| a.shown.cmp(&b.shown));
    out
}

/// The lines outside every top-level `#[cfg(test)]` item, each with its 1-based number, so a file
/// reads the same whether its tests sit in it or in a `tests.rs` beside it. rustfmt leaves a
/// top-level item's closing brace alone at column 0, which ends a braced one; an unbraced one
/// ends at its `;`.
pub fn production_lines(text: &str) -> Vec<(usize, &str)> {
    let lines: Vec<&str> = text.lines().collect();
    let gated = |line: &str| line.starts_with("#[cfg(test)]") || line.starts_with("#[cfg(any(test");
    let mut kept = Vec::new();
    let mut at = 0;
    while at < lines.len() {
        if !gated(lines[at]) {
            kept.push((at + 1, lines[at]));
            at += 1;
            continue;
        }
        at += 1;
        while lines.get(at).is_some_and(|l| l.starts_with("#[")) {
            at += 1;
        }
        if lines.get(at).is_some_and(|l| l.trim_end().ends_with('{')) {
            at += 1;
            while at < lines.len() && lines[at] != "}" {
                at += 1;
            }
        } else {
            while at < lines.len() && !lines[at].trim_end().ends_with(';') {
                at += 1;
            }
        }
        at += 1;
    }
    kept
}
