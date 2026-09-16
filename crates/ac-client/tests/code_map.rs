//! The code maps read before the code: the root `CLAUDE.md` and this crate's. A map naming a
//! path, fn, step or module that is gone misleads more than no map, so these fail first.
//! They read files only and need no game data.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ac_client::steps::STEPS;

/// The root map is loaded into every session; this crate's only when working here.
const ROOT_LINES: usize = 121;
const CLIENT_LINES: usize = 200;

/// A backticked span ending in one of these names a file.
const EXTENSIONS: &[&str] = &[
    ".rs", ".md", ".sh", ".toml", ".rhai", ".json", ".txt", ".yml", ".csv",
];

/// How docs name binaries removed from the workspace. Retail's `acclient.exe` is not one.
const REMOVED_BINS: &[&str] = &["acbot", "bins/acclient", "-p acclient", "acclient --"];

fn client_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace() -> PathBuf {
    client_dir()
        .parent()
        .and_then(Path::parent)
        .expect("crates/ac-client sits two below the workspace")
        .to_path_buf()
}

struct Map {
    label: &'static str,
    /// Where the map sits: its bare names resolve against this and its `src/`.
    dir: PathBuf,
    text: String,
}

fn read_map(label: &'static str, dir: PathBuf) -> Map {
    let text = fs::read_to_string(dir.join("CLAUDE.md"))
        .unwrap_or_else(|e| panic!("reading {label}: {e}"));
    Map { label, dir, text }
}

fn root_map() -> Map {
    read_map("CLAUDE.md", workspace())
}

fn client_map() -> Map {
    read_map("crates/ac-client/CLAUDE.md", client_dir())
}

/// The text outside fenced code blocks.
fn prose(text: &str) -> String {
    let mut out = String::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if !fenced {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Every closed inline code span, line by line.
fn spans(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split('`').collect();
        for (i, part) in parts.iter().enumerate() {
            if i % 2 == 1 && i + 1 < parts.len() {
                out.push(part.to_string());
            }
        }
    }
    out
}

/// A span shaped like a file path: the path, and the `first-last` line range after a colon.
fn as_path(span: &str) -> Option<(&str, Option<&str>)> {
    const NOT_IN_A_PATH: &[char] = &['*', '<', '>', '{', '}', '$', '~', '=', '(', '[', ',', '"'];
    if span.is_empty()
        || span.contains(char::is_whitespace)
        || span.contains("::")
        || span.contains(NOT_IN_A_PATH)
        || span.starts_with(['-', '.', '/', '#', '@'])
    {
        return None;
    }
    let (path, range) = match span.split_once(':') {
        Some((path, range)) => (path, Some(range)),
        None => (span, None),
    };
    let looks = path.contains('/') || EXTENSIONS.iter().any(|e| path.ends_with(e));
    looks.then_some((path, range))
}

/// A map's path from the workspace root, the map's directory, or that directory's `src/`.
fn resolve(map_dir: &Path, path: &str) -> Option<PathBuf> {
    [workspace(), map_dir.to_path_buf(), map_dir.join("src")]
        .into_iter()
        .map(|base| base.join(path))
        .find(|p| p.exists())
}

/// Spans naming a file that is missing, or a line range past its end.
fn stale_paths(map_dir: &Path, text: &str) -> Vec<String> {
    let mut stale = Vec::new();
    for span in spans(&prose(text)) {
        let Some((path, range)) = as_path(&span) else {
            continue;
        };
        let Some(file) = resolve(map_dir, path) else {
            stale.push(format!("`{span}`: no such file"));
            continue;
        };
        let Some(range) = range else {
            continue;
        };
        let lines = fs::read_to_string(&file)
            .map(|t| t.lines().count())
            .unwrap_or(0);
        let parsed = match range.split_once('-') {
            Some((first, last)) => first.parse::<usize>().ok().zip(last.parse().ok()),
            None => range.parse::<usize>().ok().map(|n| (n, n)),
        };
        match parsed {
            Some((first, last)) if 1 <= first && first <= last && last <= lines => {}
            Some(_) => stale.push(format!("`{span}`: {path} has {lines} lines")),
            None => stale.push(format!("`{span}`: not a line range")),
        }
    }
    stale
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != "target") {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The name of every `fn` written under `crates/`.
fn defined_fns() -> BTreeSet<String> {
    let mut files = Vec::new();
    rust_files(&workspace().join("crates"), &mut files);
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut names = BTreeSet::new();
    for file in files {
        let text = fs::read_to_string(&file).unwrap_or_default();
        for (at, _) in text.match_indices("fn ") {
            if text[..at].chars().next_back().is_some_and(ident) {
                continue;
            }
            let name: String = text[at + 3..].chars().take_while(|&c| ident(c)).collect();
            if !name.is_empty() {
                names.insert(name);
            }
        }
    }
    names
}

/// The fn a span names as `name`, `name()` or `path::name()`, when it is shaped like one.
fn fn_name(span: &str) -> Option<&str> {
    let bare = span.strip_suffix("()").unwrap_or(span);
    let segments_ok = bare
        .split("::")
        .all(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    let name = bare.rsplit("::").next()?;
    let snake = name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    (segments_ok && snake).then_some(name)
}

/// The cells under `column` in the table whose header row starts with the cell `first`.
fn column(text: &str, first: &str, column: &str) -> Vec<String> {
    let cells = |line: &str| -> Vec<String> {
        line.trim()
            .trim_matches('|')
            .split('|')
            .map(|c| c.trim().to_string())
            .collect()
    };
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if !line.trim_start().starts_with('|') {
            continue;
        }
        let header = cells(line);
        if header.first().map(String::as_str) != Some(first) {
            continue;
        }
        let Some(at) = header.iter().position(|h| h == column) else {
            continue;
        };
        // The |---| row under the header.
        lines.next();
        return lines
            .by_ref()
            .take_while(|l| l.trim_start().starts_with('|'))
            .map(|l| cells(l).get(at).cloned().unwrap_or_default())
            .collect();
    }
    Vec::new()
}

/// Fns a map names that nothing under `crates/` defines: every span in a table's fn column
/// (entry fns, and the steps' worth and runs), and any span written as a call, `name()`.
fn undefined_fns(text: &str, defined: &BTreeSet<String>) -> Vec<String> {
    let prose = prose(text);
    let mut missing = Vec::new();
    for (first, name) in [("system", "entry fns"), ("#", "worth"), ("#", "runs")] {
        for cell in column(&prose, first, name) {
            for span in spans(&cell) {
                match fn_name(&span) {
                    Some(f) if defined.contains(f) => {}
                    Some(_) => missing.push(format!("`{span}` in {name}: no such fn")),
                    None => missing.push(format!("`{span}` in {name}: not a fn name")),
                }
            }
        }
    }
    for span in spans(&prose) {
        if !span.ends_with("()") {
            continue;
        }
        if let Some(f) = fn_name(&span).filter(|f| !defined.contains(*f)) {
            missing.push(format!("`{span}`: no fn {f}"));
        }
    }
    missing
}

fn with_label(label: &str, found: Vec<String>) -> Vec<String> {
    found.into_iter().map(|f| format!("{label}: {f}")).collect()
}

#[test]
fn every_path_the_maps_name_exists() {
    let mut stale = Vec::new();
    for map in [root_map(), client_map()] {
        stale.extend(with_label(map.label, stale_paths(&map.dir, &map.text)));
    }
    assert!(stale.is_empty(), "stale paths:\n{}", stale.join("\n"));
}

#[test]
fn every_fn_the_maps_name_is_defined() {
    let defined = defined_fns();
    assert!(
        defined.contains("tick_autoplay"),
        "the fn index found nothing"
    );
    let mut missing = Vec::new();
    for map in [root_map(), client_map()] {
        missing.extend(with_label(map.label, undefined_fns(&map.text, &defined)));
    }
    assert!(missing.is_empty(), "unknown fns:\n{}", missing.join("\n"));
}

#[test]
fn the_map_lists_the_steps_in_their_order() {
    let listed = column(&prose(&client_map().text), "#", "step");
    let steps: Vec<&str> = STEPS.iter().map(|s| s.name).collect();
    assert_eq!(
        listed, steps,
        "the Steps table in crates/ac-client/CLAUDE.md"
    );
}

#[test]
fn every_client_module_has_a_row() {
    let lib = fs::read_to_string(client_dir().join("src").join("lib.rs")).expect("lib.rs");
    let modules: Vec<&str> = lib
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l
                .strip_prefix("pub mod ")
                .or_else(|| l.strip_prefix("pub(crate) mod "))
                .or_else(|| l.strip_prefix("mod "))?;
            rest.strip_suffix(';')
        })
        .collect();
    assert!(modules.contains(&"autoplay"), "no modules read from lib.rs");
    let map = client_map();
    let named: BTreeSet<String> = spans(&prose(&map.text))
        .iter()
        .filter_map(|s| as_path(s).map(|(path, _)| path.to_string()))
        .collect();
    let missing: Vec<&str> = modules
        .into_iter()
        .filter(|m| {
            // A module is either `name.rs` or the `name/mod.rs` of a directory.
            [format!("{m}.rs"), format!("{m}/mod.rs")]
                .iter()
                .all(|file| {
                    ![
                        file.clone(),
                        format!("src/{file}"),
                        format!("crates/ac-client/src/{file}"),
                    ]
                    .iter()
                    .any(|f| named.contains(f))
                })
        })
        .collect();
    assert!(
        missing.is_empty(),
        "modules with no row in {}: {missing:?}",
        map.label
    );
}

#[test]
fn every_crate_has_a_row() {
    let map = root_map();
    let rows: BTreeSet<String> = prose(&map.text)
        .lines()
        .filter(|l| l.trim_start().starts_with('|'))
        .filter_map(|l| l.trim().trim_matches('|').split('|').next())
        .map(|c| c.trim().to_string())
        .collect();
    let mut missing = Vec::new();
    for dir in ["crates", "bins", "examples"] {
        let entries = fs::read_dir(workspace().join(dir)).expect(dir);
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if entry.path().join("Cargo.toml").exists() && !rows.contains(&name) {
                missing.push(format!("{dir}/{name}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "crates with no row in {}: {missing:?}",
        map.label
    );
}

#[test]
fn the_maps_stay_short() {
    let root = root_map().text.lines().count();
    let client = client_map().text.lines().count();
    assert!(root <= ROOT_LINES, "CLAUDE.md is {root} lines");
    assert!(
        client <= CLIENT_LINES,
        "crates/ac-client/CLAUDE.md is {client} lines"
    );
}

fn markdown_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            markdown_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

#[test]
fn the_docs_name_no_removed_binary() {
    let root = workspace();
    let mut files = vec![
        root.join("README.md"),
        root.join("examples")
            .join("plugin-template")
            .join("README.md"),
    ];
    markdown_files(&root.join("docs"), &mut files);
    let mut hits = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        let shown = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .display()
            .to_string();
        for (n, line) in text.lines().enumerate() {
            for word in REMOVED_BINS.iter().filter(|w| line.contains(**w)) {
                hits.push(format!("{shown}:{}: {word}", n + 1));
            }
        }
    }
    assert!(files.len() > 3, "no docs found");
    assert!(
        hits.is_empty(),
        "removed binaries named:\n{}",
        hits.join("\n")
    );
}

#[test]
fn the_checks_catch_a_stale_map() {
    let defined: BTreeSet<String> = ["real_fn".to_string()].into();
    let text = "\
| system | entry fns |
|---|---|
| one | `real_fn`, `gone_fn`, `Not A Fn` |

Calls `real_fn()` and `path::other_gone()`; see `no/such/file.rs` and `CLAUDE.md:1-999999`.

```
`fenced_gone()` in `fenced/file.rs` is not read
```
";
    let missing = undefined_fns(text, &defined);
    assert_eq!(missing.len(), 3, "{missing:?}");
    let stale = stale_paths(&workspace(), text);
    assert_eq!(stale.len(), 2, "{stale:?}");
    assert_eq!(fn_name("steps::weigh()"), Some("weigh"));
    assert_eq!(
        as_path("autoplay.rs:10-20"),
        Some(("autoplay.rs", Some("10-20")))
    );
    assert_eq!(as_path("ac_client::travel"), None);
}
