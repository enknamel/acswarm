use super::*;

#[test]
fn facts_are_sources_ids_measures_and_rule_words() {
    for text in [
        "// ACE says so in Player_Move.cs:212",
        "/// opcode 0xF7B1",
        "// waits 300 ms",
        "// waits 300ms",
        "// every 1.5 s",
        "// within 5 m of it",
        "// shockwave flies at 15 m/s",
        "//! a 2 km road",
        "// 50% of the time",
        "// needs 15 GB free",
        "// Never opens a door",
        "// the leader must hold",
        "// always the nearest",
        "// only when idle",
        "// before the fight",
        "// after it lands",
    ] {
        assert!(holds_fact(text), "{text}");
    }
}

#[test]
fn ordinary_words_and_bare_numbers_are_not_facts() {
    for text in [
        "// walks to the corpse",
        "// three sessions",
        "// module 3 of the plan",
        "// 3 more fights",
        "// 10 sessions",
        "// afterwards it rests",
        "// mustard and onions",
        "// x2s",
    ] {
        assert!(!holds_fact(text), "{text}");
    }
}

const DIFF: &str = "\
diff --git a/crates/a/src/lib.rs b/crates/a/src/lib.rs
index 1111111..2222222 100644
--- a/crates/a/src/lib.rs
+++ b/crates/a/src/lib.rs
@@ -3,2 +3 @@ fn one() {
-    /// Waits 300 ms.
-    let x = 1;
+    let x = 2;
@@ -10 +9,0 @@ fn two() {
-// ---- a section rule
@@ -20,4 +18,0 @@
-    // plain
--- a/not/a/header.rs
-    code();
-    //! inner
diff --git a/crates/b/src/gone.rs b/crates/b/src/gone.rs
deleted file mode 100644
index 3333333..0000000
--- a/crates/b/src/gone.rs
+++ /dev/null
@@ -1,2 +0,0 @@
-//! Gone, never to return.
-pub fn gone() {}
\\ No newline at end of file
diff --git a/crates/c/src/new.rs b/crates/c/src/new.rs
new file mode 100644
index 0000000..4444444
--- /dev/null
+++ b/crates/c/src/new.rs
@@ -0,0 +1 @@
+// only added, never removed
";

fn removed(path: &str, line: usize, text: &str) -> Removed {
    Removed {
        path: path.to_owned(),
        line,
        text: text.to_owned(),
    }
}

#[test]
fn removed_comment_lines_are_numbered_as_in_the_base() {
    assert_eq!(
        removed_comment_lines(DIFF),
        vec![
            removed("crates/a/src/lib.rs", 3, "    /// Waits 300 ms."),
            removed("crates/a/src/lib.rs", 10, "// ---- a section rule"),
            removed("crates/a/src/lib.rs", 20, "    // plain"),
            removed("crates/a/src/lib.rs", 23, "    //! inner"),
            removed("crates/b/src/gone.rs", 1, "//! Gone, never to return."),
        ]
    );
}

#[test]
fn hunk_headers_give_the_base_range() {
    assert_eq!(hunk_old_range("@@ -12,3 +12,0 @@ fn x() {"), Some((12, 3)));
    assert_eq!(hunk_old_range("@@ -7 +7 @@"), Some((7, 1)));
    assert_eq!(hunk_old_range("@@ -0,0 +1 @@"), Some((0, 0)));
    assert_eq!(hunk_old_range("diff --git a/x.rs b/x.rs"), None);
}

#[test]
fn the_report_groups_facts_by_file() {
    let facts = vec![
        removed("a.rs", 3, "    /// Waits 300 ms."),
        removed("a.rs", 9, "// never"),
        removed("b.rs", 1, "//! 0x10"),
    ];
    assert_eq!(
        report(&facts, "main"),
        "a.rs\n\
         a.rs:3: /// Waits 300 ms.\n\
         a.rs:9: // never\n\
         \n\
         b.rs\n\
         b.rs:1: //! 0x10\n\
         \n\
         3 removed comment line(s) hold facts; lines are numbered as in main\n"
    );
    assert_eq!(
        report(&[], "main"),
        "no removed comment line since main holds a fact\n"
    );
}

#[test]
fn only_fact_lines_survive_the_filter() {
    let texts: Vec<String> = removed_comment_lines(DIFF)
        .into_iter()
        .filter(|line| holds_fact(&line.text))
        .map(|line| line.text)
        .collect();
    assert_eq!(
        texts,
        ["    /// Waits 300 ms.", "//! Gone, never to return."]
    );
}
