//! `facts`: lists removed comment lines that hold facts, so a reviewer can confirm each
//! one survives a comment rewrite, in a shorter comment, a commit message or docs.

use std::fmt::Write as _;
use std::process::ExitCode;
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;

use crate::git::Repo;

/// A fact: an ACE source line (`.cs:NN`), a hex id, a number with a unit, or a word
/// that states a rule or an order.
static FACT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\.cs:\d+",
        r"|0x[0-9a-fA-F]+",
        r"|\b\d+(?:\.\d+)?\s?(?:(?:m/s|ms|km|GB|s|m)\b|%)",
        r"|(?i:\b(?:never|must|always|only|before|after)\b)",
    ))
    .expect("the fact pattern compiles")
});

pub fn run(base: &str) -> Result<ExitCode> {
    let repo = Repo::here()?;
    repo.check_rev(base)?;
    let diff = repo.rs_diff_u0(base)?;
    let facts: Vec<Removed> = removed_comment_lines(&diff)
        .into_iter()
        .filter(|removed| holds_fact(&removed.text))
        .collect();
    print!("{}", report(&facts, base));
    Ok(ExitCode::SUCCESS)
}

/// A comment line a diff removes, numbered as in the base revision.
#[derive(Debug, PartialEq, Eq)]
pub struct Removed {
    pub path: String,
    pub line: usize,
    pub text: String,
}

pub fn holds_fact(text: &str) -> bool {
    FACT.is_match(text)
}

/// The removed lines of a `git diff -U0` that are `//`, `///` or `//!` comments.
pub fn removed_comment_lines(diff: &str) -> Vec<Removed> {
    let mut found = Vec::new();
    // The base-side path of the file being read; `None` for a new file.
    let mut path: Option<String> = None;
    let mut old_line = 0;
    // Removed lines still to come in the current hunk. Counting them keeps a removed
    // line that starts with `--` from being read as a file header.
    let mut old_left = 0;
    for line in diff.lines() {
        if old_left > 0 {
            if let Some(text) = line.strip_prefix('-') {
                if let (Some(path), true) = (&path, text.trim_start().starts_with("//")) {
                    found.push(Removed {
                        path: path.clone(),
                        line: old_line,
                        text: text.to_owned(),
                    });
                }
                old_line += 1;
                old_left -= 1;
                continue;
            }
        }
        if line.starts_with("diff --git ") {
            path = None;
            old_left = 0;
        } else if let Some(name) = line.strip_prefix("--- ") {
            // Git ends a name holding a space with a tab.
            path = name
                .trim_end_matches('\t')
                .strip_prefix("a/")
                .map(str::to_owned);
        } else if let Some((start, count)) = hunk_old_range(line) {
            old_line = start;
            old_left = count;
        }
    }
    found
}

/// The base-side start line and line count of a hunk header `@@ -12,3 +12,0 @@`.
fn hunk_old_range(line: &str) -> Option<(usize, usize)> {
    let old = line.strip_prefix("@@ -")?.split_whitespace().next()?;
    match old.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((old.parse().ok()?, 1)),
    }
}

/// `path:line: text` for each fact, under a header line per file.
pub fn report(facts: &[Removed], base: &str) -> String {
    if facts.is_empty() {
        return format!("no removed comment line since {base} holds a fact\n");
    }
    let mut out = String::new();
    let mut file: Option<&str> = None;
    for fact in facts {
        if file != Some(fact.path.as_str()) {
            if file.is_some() {
                out.push('\n');
            }
            let _ = writeln!(out, "{}", fact.path);
            file = Some(&fact.path);
        }
        let _ = writeln!(out, "{}:{}: {}", fact.path, fact.line, fact.text.trim());
    }
    let _ = writeln!(
        out,
        "\n{} removed comment line(s) hold facts; lines are numbered as in {base}",
        facts.len()
    );
    out
}

#[cfg(test)]
mod tests;
