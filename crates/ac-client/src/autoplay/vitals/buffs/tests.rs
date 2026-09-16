use super::*;

#[test]
fn an_urgent_buff_waits_for_the_fight_only_when_it_costs_the_weapon() {
    // The urgent pass runs as a reflex, ahead of loot and ahead of
    // the fight, and ignored `out_of_combat_only` outright: a buff
    // with a minute still on it was enough to put the sword away
    // mid-swing. Under god mode that cost throughput; for a mortal
    // character it is a fight fought bare-handed.
    let (sword, wand) = (false, true);
    let mut cfg = Buffs {
        never_below: 60.0,
        top_up_within: 300.0,
        out_of_combat_only: true,
        ..Buffs::default()
    };
    assert_eq!(
        buff_within(&cfg, true, true, sword),
        0.0,
        "only what has lapsed"
    );
    // A buff that is not up at all reads as nought seconds left, so
    // it still goes back up in the middle of a fight. That is what
    // "never below" is for.
    assert!(0.0 <= buff_within(&cfg, true, true, sword));
    // With a wand already in hand the recast costs a cast and
    // nothing else, so `never_below` holds. Answering nought here
    // meant a mortal caster's protections were put back only after
    // they had lapsed -- the very window the setting names.
    assert_eq!(
        buff_within(&cfg, true, true, wand),
        60.0,
        "a free recast was still made to wait for the fight"
    );
    // Out of the fight the urgent pass is unchanged, and so is the
    // quiet one either way.
    assert_eq!(buff_within(&cfg, true, false, sword), 60.0);
    assert_eq!(buff_within(&cfg, false, true, sword), 300.0);
    // And a player who has not asked for the restraint keeps the
    // old behaviour: buffs go back up mid-fight.
    cfg.out_of_combat_only = false;
    assert_eq!(buff_within(&cfg, true, true, sword), 60.0);
}
