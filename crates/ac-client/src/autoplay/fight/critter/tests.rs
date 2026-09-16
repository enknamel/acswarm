use super::*;

/// A table row made up for a test.
fn row(tolerance: u32, health: u32, level: Option<u32>) -> ac_world::elements::Creature {
    ac_world::elements::Creature {
        wcid: 0,
        name: String::new(),
        health,
        takes: [None; 8],
        tolerance,
        level,
    }
}

/// The table's row for this very weenie.
fn weenie(row: &ac_world::elements::Creature) -> Option<Hint<'_>> {
    Some(Hint::Weenie(row))
}

#[test]
fn a_creature_the_table_knows_is_a_critter_when_it_is_quiet_outgrown_and_small() {
    use ac_world::elements::{creature_by_id, tolerance};
    let quiet = Seen::default();
    let rabbit = row(tolerance::RETALIATE, 5, Some(4));
    // A level 4 Rabbit with its five health: fought at 7, walked
    // past from 8 up.
    for mine in [5, 7] {
        assert_eq!(
            critter(quiet, None, None, weenie(&rabbit), mine),
            Critter::Fight
        );
    }
    for mine in [8, 20] {
        assert_eq!(
            critter(quiet, None, None, weenie(&rabbit), mine),
            Critter::WalkPast
        );
    }
    // A level 61 Revenant is passive and nobody outgrows it: 122
    // is past the level a character can reach.
    let revenant = creature_by_id(8592).expect("in the table");
    assert_eq!(revenant.level, Some(61));
    for mine in [100, 121] {
        assert_eq!(
            critter(quiet, None, None, weenie(revenant), mine),
            Critter::Fight
        );
    }
    // Something the table says attacks on sight is fought however
    // small, and before it has noticed the character.
    assert_eq!(
        critter(quiet, None, None, weenie(&row(0, 3, Some(1))), 275),
        Critter::Fight
    );
    // Every flag that means it leaves a passer-by alone counts.
    for flag in [
        tolerance::NO_ATTACK,
        tolerance::APPRAISE,
        tolerance::PROVOKE,
        tolerance::RETALIATE,
        tolerance::MONSTER,
    ] {
        assert_eq!(
            critter(quiet, None, None, weenie(&row(flag, 5, Some(4))), 20),
            Critter::WalkPast,
            "{flag}"
        );
    }
    // Something that cannot fight back at all is scenery with a
    // health bar, and hitting it is a chore whatever it can take: a
    // Portal Pillar has two thousand health and never swings.
    assert_eq!(
        critter(
            quiet,
            None,
            None,
            weenie(&row(tolerance::NO_ATTACK, 2001, Some(4))),
            50
        ),
        Critter::WalkPast
    );
    // The rest of the flags do fight back once hit, so what they
    // can take is what decides: a Drudge Skulker's forty-two is a
    // fight and so is a Mite Snippet's twenty, a Rabbit's five is
    // not. The table's line, not the stranger's: a Mite Snippet
    // has a Cow's health, and only the table tells them apart.
    for health in [42, 20, CRITTER_HEALTH + 1] {
        assert_eq!(
            critter(
                quiet,
                None,
                None,
                weenie(&row(tolerance::RETALIATE, health, Some(8))),
                50
            ),
            Critter::Fight,
            "{health}"
        );
    }
    // The ones that do not: "only fight back at whoever started it"
    // still starts fights with everyone else.
    assert_eq!(
        critter(quiet, None, None, weenie(&row(32, 5, Some(4))), 20),
        Critter::Fight
    );
    // A character with no level yet fights everything.
    assert_eq!(
        critter(quiet, None, None, weenie(&rabbit), 0),
        Critter::Fight
    );
    // Nothing is walked past on a guess, nor attacked on one: a
    // row with no level, or no health, is a question for the
    // server.
    assert_eq!(
        critter(
            quiet,
            None,
            None,
            weenie(&row(tolerance::RETALIATE, 5, None)),
            275
        ),
        Critter::Appraise
    );
    assert_eq!(
        critter(
            quiet,
            None,
            None,
            weenie(&row(tolerance::RETALIATE, 0, Some(4))),
            275
        ),
        Critter::Appraise
    );
    // And the answer stands in for the table's figure.
    assert_eq!(
        critter(
            quiet,
            Some(4),
            None,
            weenie(&row(tolerance::RETALIATE, 5, None)),
            275
        ),
        Critter::WalkPast
    );
    assert_eq!(
        critter(
            quiet,
            None,
            Some(5),
            weenie(&row(tolerance::RETALIATE, 0, Some(4))),
            275
        ),
        Critter::WalkPast
    );
    // Or beats it: a Rabbit by name that is level 40 by appraisal
    // is no Rabbit, and one with sixty health is no Rabbit either.
    assert_eq!(
        critter(quiet, Some(40), None, weenie(&rabbit), 50),
        Critter::Fight
    );
    assert_eq!(
        critter(quiet, None, Some(60), weenie(&rabbit), 50),
        Critter::Fight
    );
    // A row found by the end of the name is a guess at the kind,
    // and its figures are some other weenie's: a "Dire Brown Rabbit" is
    // asked about, not walked past on the Rabbit's four and five,
    // and its own answer decides. Its temper is still believed,
    // both ways.
    let by_name = Some(Hint::Name(&rabbit));
    assert_eq!(critter(quiet, None, None, by_name, 50), Critter::Appraise);
    assert_eq!(
        critter(quiet, Some(4), None, by_name, 50),
        Critter::Appraise
    );
    assert_eq!(
        critter(quiet, Some(4), Some(5), by_name, 50),
        Critter::WalkPast
    );
    assert_eq!(
        critter(quiet, Some(40), Some(300), by_name, 50),
        Critter::Fight
    );
    assert_eq!(
        critter(
            quiet,
            None,
            None,
            Some(Hint::Name(&row(0, 3, Some(1)))),
            275
        ),
        Critter::Fight,
        "the kind starts fights"
    );
    assert_eq!(
        critter(
            quiet,
            Some(4),
            None,
            Some(Hint::Name(&row(tolerance::NO_ATTACK, 2001, Some(4)))),
            50
        ),
        Critter::WalkPast,
        "the kind never swings"
    );
}

#[test]
fn a_creature_the_table_does_not_know_is_judged_by_what_it_does() {
    use ac_world::elements::tolerance;
    let quiet = Seen::default();
    // A Cow: in no table, level 8 with twenty health by appraisal,
    // docile until attacked. Outgrown from 16 up, and a level 15
    // still fights it. Not in the table is not a reason to fight.
    for mine in [16, 20, 275] {
        assert_eq!(
            critter(quiet, Some(8), Some(20), None, mine),
            Critter::WalkPast
        );
    }
    assert_eq!(critter(quiet, Some(8), Some(20), None, 15), Critter::Fight);
    // A level 3 one, likewise.
    assert_eq!(
        critter(quiet, Some(3), Some(20), None, 20),
        Critter::WalkPast
    );
    // Nothing known about it yet: asked about, neither attacked nor
    // walked past. An appraisal brings the level and the health
    // together, so half an answer is no answer.
    assert_eq!(critter(quiet, None, None, None, 20), Critter::Appraise);
    assert_eq!(critter(quiet, Some(8), None, None, 20), Critter::Appraise);
    assert_eq!(critter(quiet, None, Some(20), None, 20), Critter::Appraise);
    // Too big to be a critter: an Auroch Yearling is level 8 with
    // sixty-five health, and worth the fight.
    assert_eq!(critter(quiet, Some(8), Some(65), None, 50), Critter::Fight);
    // The stranger's line is wider than the table's, and it is a
    // line.
    assert_eq!(
        critter(quiet, Some(8), Some(STRANGER_HEALTH), None, 50),
        Critter::WalkPast
    );
    assert_eq!(
        critter(quiet, Some(8), Some(STRANGER_HEALTH + 1), None, 50),
        Critter::Fight
    );
    // What it does outranks everything: attacking the character,
    // walking at it or a mate, or fighting anyone at all is a fight
    // already, whatever is or is not known about it.
    let rabbit = row(tolerance::RETALIATE, 5, Some(4));
    for seen in [
        Seen {
            attacked_us: true,
            ..Seen::default()
        },
        Seen {
            targets_us_or_mate: true,
            ..Seen::default()
        },
        Seen {
            fighting_anyone: true,
            ..Seen::default()
        },
    ] {
        assert!(!seen.quiet());
        assert_eq!(
            critter(seen, Some(8), Some(20), None, 20),
            Critter::Fight,
            "{seen:?}"
        );
        assert_eq!(
            critter(seen, None, None, None, 20),
            Critter::Fight,
            "{seen:?}"
        );
        assert_eq!(
            critter(seen, None, None, weenie(&rabbit), 20),
            Critter::Fight,
            "{seen:?}"
        );
    }
}

#[test]
fn the_holtburg_fields_are_still_a_hunting_ground_at_any_level() {
    use ac_world::elements::creature_by_id;
    let quiet = Seen::default();
    // Every creature the two encounter generators around Holtburg
    // put out (2007 newbietownaluviangen, 5150 harmlessaluviangen),
    // by weenie. ACE gives the first eight Retaliate so they do not
    // come at a new player, and they are all level 8: by level
    // alone a level 16 character had nothing left to attack
    // anywhere in Holtburg. None of them ever attacks first, so
    // what they have been seen doing says nothing, and the table
    // is what keeps them a hunting ground.
    let field = [
        19257, // Drudge Skulker
        19258, // Drudge Slinker
        19263, // Gnawer Shreth
        19261, // Creeper Mosswart
        19262, // Young Mosswart
        19256, // Young Banderling
        19260, // Mite Snippet
        19259, // Mite Scion
    ];
    for wcid in field {
        let c = creature_by_id(wcid).expect("in the table");
        assert!(c.passive(), "{} is what the test is about", c.name);
        for mine in [16, 20, 50, 275] {
            assert_eq!(
                critter(quiet, None, None, weenie(c), mine),
                Critter::Fight,
                "{} is what the fields are for, at level {mine}",
                c.name
            );
        }
    }
    // The two that really are critters are still walked past.
    for wcid in [2566, 24937] {
        let c = creature_by_id(wcid).expect("in the table");
        assert_eq!(
            critter(quiet, None, None, weenie(c), 20),
            Critter::WalkPast,
            "{} is worth nothing to a level 20 character",
            c.name
        );
    }
}
