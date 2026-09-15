//! The git queries the chores need, run at the repository's top level.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

pub struct Repo {
    root: PathBuf,
}

impl Repo {
    /// The repository holding the current directory.
    pub fn here() -> Result<Self> {
        let top = git(None, &["rev-parse", "--show-toplevel"])?;
        Ok(Self {
            root: PathBuf::from(top.trim_end()),
        })
    }

    /// Fails unless `rev` names a commit.
    pub fn check_rev(&self, rev: &str) -> Result<()> {
        let commit = format!("{rev}^{{commit}}");
        git(
            Some(&self.root),
            &["rev-parse", "--verify", "--quiet", &commit],
        )
        .map(drop)
        .with_context(|| format!("{rev} is not a commit"))
    }

    /// `.rs` paths, from the top level, that differ between `rev` and the working tree.
    /// A rename is listed as a deletion and an addition.
    pub fn changed_rs_files(&self, rev: &str) -> Result<Vec<String>> {
        let names = git(
            Some(&self.root),
            &[
                "diff",
                "--name-only",
                "-z",
                "--no-renames",
                rev,
                "--",
                "*.rs",
            ],
        )?;
        Ok(names
            .split('\0')
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// `git diff -U0` of the `.rs` files from `rev` to the working tree.
    pub fn rs_diff_u0(&self, rev: &str) -> Result<String> {
        git(
            Some(&self.root),
            &[
                "diff",
                "-U0",
                "--no-renames",
                "--no-color",
                "--no-ext-diff",
                "--src-prefix=a/",
                "--dst-prefix=b/",
                rev,
                "--",
                "*.rs",
            ],
        )
    }

    /// The file at `path` in `rev`, or `None` when `rev` has no such file.
    pub fn show(&self, rev: &str, path: &str) -> Result<Option<String>> {
        let spec = format!("{rev}:{path}");
        if git(Some(&self.root), &["cat-file", "-e", &spec]).is_err() {
            return Ok(None);
        }
        git(Some(&self.root), &["cat-file", "blob", &spec]).map(Some)
    }

    /// The working-tree file at `path`, or `None` when it is gone.
    pub fn read(&self, path: &str) -> Result<Option<String>> {
        let full = self.root.join(path);
        match std::fs::read_to_string(&full) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", full.display())),
        }
    }
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<String> {
    let mut command = Command::new("git");
    if let Some(dir) = dir {
        command.arg("-C").arg(dir);
    }
    // Paths print unquoted, so they join onto the top level as they are.
    command.args(["-c", "core.quotepath=off"]).args(args);
    let out = command.output().context("running git")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    String::from_utf8(out.stdout).context("git printed text that is not UTF-8")
}
