use super::*;

fn outcome(old: &str, new: &str) -> Outcome {
    compare_sources(Some(old), Some(new))
}

fn differs(old: &str, new: &str) -> Difference {
    match outcome(old, new) {
        Outcome::Differs(difference) => difference,
        other => panic!("expected a difference, got {other:?}"),
    }
}

fn changed(place: &str, line: usize) -> Difference {
    Difference {
        place: place.to_owned(),
        change: Change::Changed,
        line,
    }
}

#[test]
fn doc_comments_and_plain_comments_are_not_code() {
    let old = concat!(
        "//! The module, at length.\n",
        "\n",
        "/// Adds a number to itself.\n",
        "/// Twice over.\n",
        "fn add(a: u8) -> u8 { a + a }\n",
    );
    let new = concat!(
        "//! The module.\n",
        "// A plain comment.\n",
        "fn add(a: u8) -> u8 {\n",
        "    /* Why it doubles. */\n",
        "    a + a\n",
        "}\n",
    );
    assert_eq!(outcome(old, new), Outcome::Same);
}

#[test]
fn docs_are_stripped_at_every_depth() {
    let old = concat!(
        "/** A block doc. */\n",
        "pub struct Pack {\n",
        "    /// Slots.\n",
        "    pub slots: u8,\n",
        "}\n",
        "#[doc = \"An explicit doc.\"]\n",
        "enum Kind {\n",
        "    /// A weapon.\n",
        "    Weapon,\n",
        "}\n",
        "impl Pack {\n",
        "    /// Empties it.\n",
        "    fn empty(&mut self) {\n",
        "        /// Nested.\n",
        "        fn inner() {}\n",
        "        /// On a statement.\n",
        "        let _ = 1;\n",
        "        match self.slots {\n",
        "            /// On an arm.\n",
        "            0 => {}\n",
        "            _ => {}\n",
        "        }\n",
        "    }\n",
        "}\n",
        "trait Walk {\n",
        "    /// Steps.\n",
        "    fn step(&self);\n",
        "}\n",
        "mod inner {\n",
        "    //! Inner module doc.\n",
        "    #![allow(dead_code)]\n",
        "}\n",
        "macro_rules! make {\n",
        "    ($name:ident) => {\n",
        "        /// Made by the macro.\n",
        "        pub struct $name;\n",
        "    };\n",
        "}\n",
        "make! {\n",
        "    /// Inside an invocation.\n",
        "    Made\n",
        "}\n",
    );
    let new = concat!(
        "pub struct Pack { pub slots: u8, }\n",
        "enum Kind { Weapon, }\n",
        "impl Pack {\n",
        "    fn empty(&mut self) {\n",
        "        fn inner() {}\n",
        "        let _ = 1;\n",
        "        match self.slots { 0 => {} _ => {} }\n",
        "    }\n",
        "}\n",
        "trait Walk { fn step(&self); }\n",
        "mod inner { #![allow(dead_code)] }\n",
        "macro_rules! make { ($name:ident) => { pub struct $name; }; }\n",
        "make! { Made }\n",
    );
    assert_eq!(outcome(old, new), Outcome::Same);
}

#[test]
fn doc_lists_such_as_hidden_are_code() {
    assert_eq!(
        differs("#[doc(hidden)]\npub fn f() {}\n", "pub fn f() {}\n"),
        changed("fn f", 1)
    );
}

#[test]
fn a_changed_body_names_its_function_at_its_first_code_line() {
    let old = concat!(
        "const LIMIT: u8 = 3;\n",
        "\n",
        "/// Adds.\n",
        "fn add(a: u8) -> u8 {\n",
        "    a + a\n",
        "}\n",
    );
    let new = concat!(
        "const LIMIT: u8 = 3;\n",
        "\n",
        "/// Adds.\n",
        "/// More.\n",
        "fn add(a: u8) -> u8 {\n",
        "    a * 2\n",
        "}\n",
    );
    assert_eq!(differs(old, new), changed("fn add", 5));
}

#[test]
fn a_difference_inside_an_impl_names_the_impl_and_the_item() {
    let old = concat!(
        "struct Client;\n",
        "impl Client {\n",
        "    /// Ticks.\n",
        "    fn tick(&self) {}\n",
        "    fn walk(&self) { let _ = 1; }\n",
        "}\n",
    );
    let new = concat!(
        "struct Client;\n",
        "impl Client {\n",
        "    fn tick(&self) {}\n",
        "    /// Walks.\n",
        "    fn walk(&self) { let _ = 2; }\n",
        "}\n",
    );
    assert_eq!(differs(old, new), changed("impl Client > fn walk", 5));
}

#[test]
fn a_difference_nests_through_modules_and_traits() {
    let old = concat!(
        "mod tests {\n",
        "    trait Walk {\n",
        "        fn step(&self) { let _ = 1; }\n",
        "    }\n",
        "}\n",
    );
    let new = old.replace("let _ = 1;", "let _ = 2;");
    assert_eq!(
        differs(old, &new),
        changed("mod tests > trait Walk > fn step", 3)
    );
}

#[test]
fn a_changed_impl_header_is_reported_on_the_impl() {
    let old = "impl Client {\n    fn a() {}\n}\n";
    let new = "impl Walk for Client {\n    fn a() {}\n}\n";
    assert_eq!(
        differs(old, new),
        changed("impl Walk for Client (was impl Client)", 1)
    );
}

#[test]
fn labels_print_types_as_written() {
    let old = "impl<'a> std::fmt::Display for Name<'a, u8> {\n    fn fmt() { one() }\n}\n";
    let new = old.replace("one()", "two()");
    assert_eq!(
        differs(old, &new),
        changed("impl std::fmt::Display for Name<'a, u8> > fn fmt", 2)
    );
    assert_eq!(
        differs("use a::{b, c};\n", "use a::{b, d};\n"),
        changed("use a::{b, d} (was use a::{b, c})", 1)
    );
}

#[test]
fn added_removed_and_inserted_items() {
    assert_eq!(
        differs("fn a() {}\n", "fn a() {}\nfn b() {}\n"),
        Difference {
            place: "fn b".to_owned(),
            change: Change::Added,
            line: 2,
        }
    );
    assert_eq!(
        differs("fn a() {}\nfn b() {}\n", "fn a() {}\n"),
        Difference {
            place: "fn b".to_owned(),
            change: Change::Removed,
            line: 2,
        }
    );
    assert_eq!(
        differs(
            "fn a() {}\nfn c() {}\n",
            "fn a() {}\nfn b() {}\nfn c() {}\n"
        ),
        changed("fn b (was fn c)", 2)
    );
}

#[test]
fn file_inner_attributes_are_code() {
    assert_eq!(
        differs(
            "#![allow(dead_code)]\nfn a() {}\n",
            "#![allow(unused)]\nfn a() {}\n"
        ),
        changed("the file's inner attributes", 1)
    );
}

#[test]
fn a_file_that_does_not_parse_says_which_side() {
    let outcome = compare_sources(Some("fn a() {}\n"), Some("fn a() {}\nfn b( {}\n"));
    assert!(
        matches!(
            outcome,
            Outcome::Unparsable {
                side: Side::WorkingTree,
                line: 2,
                ..
            }
        ),
        "{outcome:?}"
    );
}

#[test]
fn new_and_deleted_files_differ_unless_allowed() {
    assert_eq!(compare_sources(None, Some("fn a() {}\n")), Outcome::Added);
    assert_eq!(compare_sources(Some("fn a() {}\n"), None), Outcome::Deleted);
    for outcome in [Outcome::Added, Outcome::Deleted] {
        assert_eq!(describe("x.rs", "main", &outcome, true), None);
    }
    assert_eq!(
        describe("x.rs", "main", &Outcome::Added, false).as_deref(),
        Some("x.rs: new file (--allow-new allows it)")
    );
    assert_eq!(
        describe("x.rs", "main", &Outcome::Deleted, false).as_deref(),
        Some("x.rs: deleted (--allow-new allows it)")
    );
}

#[test]
fn report_lines_point_at_the_difference() {
    let removed = Outcome::Differs(Difference {
        place: "fn b".to_owned(),
        change: Change::Removed,
        line: 2,
    });
    assert_eq!(
        describe("x.rs", "main", &removed, false).as_deref(),
        Some("x.rs: code differs, first at removed fn b (line 2 in main)")
    );
    let inside = Outcome::Differs(changed("impl Client > fn walk", 5));
    assert_eq!(
        describe("x.rs", "main", &inside, true).as_deref(),
        Some("x.rs:5: code differs, first in impl Client > fn walk")
    );
    assert_eq!(describe("x.rs", "main", &Outcome::Same, false), None);
}

#[test]
fn stripping_keeps_other_attributes_and_macro_patterns() {
    let tokens: TokenStream =
        "#[derive(Debug)] #[doc = \"x\"] #![doc = \"y\"] struct A; $(#[$meta:meta])* #[doc(hidden)]"
            .parse()
            .unwrap();
    let expected: TokenStream = "#[derive(Debug)] struct A; $(#[$meta:meta])* #[doc(hidden)]"
        .parse()
        .unwrap();
    assert_eq!(strip_docs(tokens).to_string(), expected.to_string());
}
