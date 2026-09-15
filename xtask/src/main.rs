//! Repository chores, run as `cargo run -p xtask -- <cmd>`.
//!
//! - `same-code <base-rev> [--allow-new]`: exits 1 when a `.rs` file changed since
//!   `<base-rev>` differs in anything but comments. Proves a comment-only commit.
//! - `facts <base-rev>`: lists removed comment lines that hold facts, for a reviewer.

mod facts;
mod git;
mod same_code;

use std::process::ExitCode;

use anyhow::{anyhow, bail, Result};

const USAGE: &str = "usage: cargo run -p xtask -- same-code <base-rev> [--allow-new]
       cargo run -p xtask -- facts <base-rev>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("xtask: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode> {
    let (command, rest) = args.split_first().ok_or_else(|| anyhow!(USAGE))?;
    match (command.as_str(), rest) {
        ("same-code", [base]) if base != "--allow-new" => same_code::run(base, false),
        ("same-code", [base, flag] | [flag, base]) if flag == "--allow-new" => {
            same_code::run(base, true)
        }
        ("facts", [base]) => facts::run(base),
        _ => bail!(USAGE),
    }
}
