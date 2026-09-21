//! Production fn names read as domain terms, not sentences: no articles, no pronouns, at most
//! four words. A name that still reads as one must be named in [`NARRATIVE`], a list that only
//! shrinks. Test names stay sentences and are not read here. Needs no game data.

use std::collections::BTreeSet;

mod gates;

/// Articles and pronouns, which a term does not carry in any position.
const NAMING_WORDS: &[&str] = &[
    "a", "an", "the", "it", "its", "we", "our", "us", "them", "their",
];

/// Copulas, read only after the first word: inside a name they join a sentence
/// (`corpse_is_ours`), while first they are the `is_` predicate the naming rule asks for.
const COPULAS: &[&str] = &["is", "are", "was"];

/// Words a name may hold.
const WORDS: usize = 4;

/// Every production fn whose name still reads as a sentence, as the wave-6 pass left them. The
/// list only shrinks: a rename drops its row, and nothing joins without an edit here. They are
/// carried, not blessed -- the glossary in `crates/ac-client/CLAUDE.md` is what a rename aims at.
const NARRATIVE: &[&str] = &[
    "a_critter",
    "a_few",
    "a_fight_in_sight",
    "a_mate_on_the_road_is_on",
    "a_pet_is_on",
    "answers_a_pour",
    "autoplay_watch_the_ground",
    "can_hurt_us",
    "carries_a_caster",
    "container_for_a_take",
    "corpse_for_us",
    "corpse_is_ours",
    "corpse_is_someone_elses",
    "covers_it",
    "drop_what_is_turned_off",
    "fall_is_lost",
    "for_a_take",
    "forget_the_departed",
    "forget_the_room",
    "frees_the_cast_slot",
    "has_an_errand",
    "held_by_an_errand",
    "in_a_fight",
    "in_the_way",
    "joins_the_team_on",
    "judged_at_a_shut",
    "killed_by_us",
    "left_behind_on_the_road",
    "load_the_quartermaster",
    "near_a_kill",
    "on_a_floor",
    "on_its_way",
    "on_the_way",
    "outranks_our_claim",
    "owes_a_corpse",
    "party_owed_a_body",
    "past_the_door",
    "past_the_wall",
    "plan_with_recalls_and_gems",
    "raises_a_maximum",
    "restocks_as_a_party",
    "sends_to_the_best",
    "set_aside_out_of_reach",
    "skills_its_rules_ask_about",
    "town_run_step_by_hand",
    "travel_about_the_ground",
    "unload_the_quartermaster",
    "wait_for_the_swing",
    "waits_for_a_corpse",
    "waits_for_them",
    "why_it_was_picked",
    "window_is_unwanted",
    "worth_a_sale_run",
    "xp_for_specialized_skill_point",
    "xp_for_specialized_skill_points",
    "xp_for_trained_skill_point",
    "xp_for_trained_skill_points",
];

/// What makes `name` a sentence rather than a term, in words a failure can print. Empty for a
/// term. A stoplist, not a dictionary: it reads the words a sentence needs, not the domain's.
fn sentence_words(name: &str) -> Vec<String> {
    let words: Vec<&str> = name.split('_').filter(|word| !word.is_empty()).collect();
    let mut found = Vec::new();
    for (at, word) in words.iter().enumerate() {
        if NAMING_WORDS.contains(word) || (at > 0 && COPULAS.contains(word)) {
            found.push(format!("`{word}`"));
        }
    }
    if words.len() > WORDS {
        found.push(format!("{} words", words.len()));
    }
    found
}

/// Every fn a file writes, with the line it sits on. Comments go first, so a name a doc line
/// quotes is not read as a definition.
fn fns_written(text: &str) -> Vec<(usize, String)> {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = Vec::new();
    for (number, line) in gates::production_lines(text) {
        let code = line.split_once("//").map_or(line, |(code, _)| code);
        for (at, _) in code.match_indices("fn ") {
            if code[..at].chars().next_back().is_some_and(ident) {
                continue;
            }
            let name: String = code[at + 3..].chars().take_while(|&c| ident(c)).collect();
            if name.starts_with(|c: char| c.is_ascii_lowercase() || c == '_') {
                out.push((number, name));
            }
        }
    }
    out
}

/// Every fn name the workspace's production files write.
fn names_written() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for file in gates::sources() {
        names.extend(fns_written(&file.text).into_iter().map(|(_, name)| name));
    }
    names
}

#[test]
fn no_production_fn_name_reads_as_a_sentence() {
    let listed: BTreeSet<&str> = NARRATIVE.iter().copied().collect();
    let mut read = 0;
    let mut sentences = Vec::new();
    for file in gates::sources() {
        for (number, name) in fns_written(&file.text) {
            read += 1;
            if listed.contains(name.as_str()) {
                continue;
            }
            let words = sentence_words(&name);
            if !words.is_empty() {
                let why = words.join(", ");
                sentences.push(format!("{}:{number}: `{name}` ({why})", file.shown));
            }
        }
    }
    assert!(read > 3_000, "only {read} production fns found");
    assert!(
        sentences.is_empty(),
        "fn names reading as sentences (rename them; a row in NARRATIVE is the last resort):\n{}",
        sentences.join("\n")
    );
}

#[test]
fn every_narrative_row_still_names_a_sentence() {
    let written = names_written();
    let mut stale = Vec::new();
    for name in NARRATIVE {
        if !written.contains(*name) {
            stale.push(format!("`{name}`: no production fn is named this"));
        } else if sentence_words(name).is_empty() {
            stale.push(format!("`{name}`: reads as a term now"));
        }
    }
    assert!(
        stale.is_empty(),
        "NARRATIVE rows to drop:\n{}",
        stale.join("\n")
    );
}

#[test]
fn the_stoplist_reads_sentences_and_spares_terms() {
    for sentence in [
        "a_critter",
        "corpse_is_ours",
        "why_it_was_picked",
        "plan_with_recalls_and_gems",
        "waits_for_them",
    ] {
        assert!(!sentence_words(sentence).is_empty(), "{sentence}");
    }
    // Predicates and the glossary's own terms, which the stoplist must leave alone.
    for term in [
        "is_dead",
        "has_errand",
        "can_cast",
        "pick_target",
        "worth_looting",
        "autoplay_keep_to_area",
        "hear_refusal",
        "offer_to_vendor",
    ] {
        assert!(sentence_words(term).is_empty(), "{term}");
    }
    assert!(
        NARRATIVE.windows(2).all(|pair| pair[0] < pair[1]),
        "NARRATIVE is sorted, one row per name"
    );
    let written = fns_written(
        "fn a_term() {}\n\
         /// see fn the_doc_one\n\
         pub(crate) fn the_other() {}\n",
    );
    let names: Vec<&str> = written.iter().map(|(_, name)| name.as_str()).collect();
    assert_eq!(names, ["a_term", "the_other"]);
    assert_eq!(written[1].0, 3, "the line a fn sits on");
}
