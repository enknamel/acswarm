//! Production files stay readable. Every `.rs` file a crate compiles is counted, and one over
//! the limit must be named in [`LONG_FILES`] with what keeps it one file, so a long file is a
//! decision on the record rather than a default. Reads files only and needs no game data.

mod gates;

/// Lines of production code one file may hold.
const LIMIT: usize = 1_500;

/// Every file over [`LIMIT`] today: its path, the lines it may not pass, and why it is one file.
/// The ceiling is today's count rounded up to the next hundred, so an exempt file may be edited
/// but not grown; a file that drops under [`LIMIT`] must lose its row, which keeps the list short.
const LONG_FILES: &[(&str, usize, &str)] = &[
    (
        "bins/acswarm/src/main.rs",
        3_100,
        "one `App`: the winit loop, the egui frame and every CLI mode share its fields",
    ),
    (
        "crates/ac-plugin/src/panels/fleet.rs",
        2_500,
        "the fleet window's every column, dialog and bulk action over all sessions at once",
    ),
    (
        "crates/ac-client/src/player.rs",
        2_100,
        "body physics in one frame of reference: floors, ledges, falls and jumps agree or fail",
    ),
    (
        "bins/acswarm/src/gpu.rs",
        2_000,
        "one wgpu device with the buffers, pipelines and passes whose layouts must match",
    ),
    (
        "crates/ac-client/src/travel.rs",
        1_800,
        "one journey end to end: trip, route, steering and the legs handed between them",
    ),
    (
        "crates/ac-plugin/src/panels/loot_profiles.rs",
        1_800,
        "the profile editor, one widget per rule field, over an ordered list that is the policy",
    ),
    (
        "crates/ac-plugin/src/panels/map.rs",
        1_700,
        "the map window: terrain, dots, hunting areas and the pointer, on shared pan and zoom",
    ),
    (
        "crates/ac-script/src/bridge.rs",
        1_700,
        "one Rhai registration per script call; the file is the API list",
    ),
];

/// Each source file with the production lines it holds, named from the workspace root.
fn measured() -> Vec<(String, usize)> {
    gates::sources()
        .into_iter()
        .map(|file| {
            let lines = gates::production_lines(&file.text).len();
            (file.shown, lines)
        })
        .collect()
}

#[test]
fn no_production_file_runs_past_the_limit() {
    let measured = measured();
    assert!(
        measured.len() > 300,
        "only {} source files found",
        measured.len()
    );
    let mut over = Vec::new();
    for (path, lines) in &measured {
        let ceiling = LONG_FILES
            .iter()
            .find(|(named, ..)| *named == path.as_str())
            .map(|&(_, ceiling, _)| ceiling)
            .unwrap_or(LIMIT);
        if *lines > ceiling {
            over.push(format!("{path}: {lines} production lines, over {ceiling}"));
        }
    }
    assert!(
        over.is_empty(),
        "files past their limit (split them, or add a row to LONG_FILES saying why not):\n{}",
        over.join("\n")
    );
}

#[test]
fn every_exempt_file_still_needs_its_row() {
    let measured = measured();
    let mut stale = Vec::new();
    for &(path, ceiling, _) in LONG_FILES {
        match measured.iter().find(|(shown, _)| shown.as_str() == path) {
            None => stale.push(format!("{path}: no such file")),
            Some((_, lines)) if *lines <= LIMIT => stale.push(format!(
                "{path}: down to {lines} lines, under the {LIMIT} limit"
            )),
            Some((_, lines)) if *lines + 100 <= ceiling => stale.push(format!(
                "{path}: {lines} lines, so its {ceiling} ceiling is a hundred too high"
            )),
            Some(_) => {}
        }
    }
    assert!(
        stale.is_empty(),
        "LONG_FILES rows to drop or tighten:\n{}",
        stale.join("\n")
    );
}

#[test]
fn the_count_leaves_test_modules_out() {
    let text = "\
one
#[cfg(test)]
mod tests {
    fn helper() {}
}
two
#[cfg(any(test, feature = \"testkit\"))]
pub mod testkit;
three
#[cfg(test)]
#[allow(clippy::all)]
impl Thing {
    fn hidden() {}
}
four
";
    let kept: Vec<&str> = gates::production_lines(text)
        .into_iter()
        .map(|(_, line)| line)
        .collect();
    assert_eq!(kept, ["one", "two", "three", "four"]);
    // The convention this leans on: the crate's longest file by raw lines is mostly tests.
    let shown = "crates/ac-loot/src/profile.rs";
    let profile = gates::sources()
        .into_iter()
        .find(|file| file.shown == shown)
        .unwrap_or_else(|| panic!("{shown} was not read"));
    assert!(
        gates::production_lines(&profile.text).len() + 500 < profile.text.lines().count(),
        "the test module of {shown} was not found"
    );
}
