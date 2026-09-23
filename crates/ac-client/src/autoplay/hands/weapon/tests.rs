use super::*;
use crate::testkit::{a_weapon, game_data, mid_fight, no_data};

#[test]
fn a_buff_pass_that_wants_a_wand_waits_rather_than_disarming_mid_charge() {
    use crate::Stance;
    // +Verity's buffing reached for her Training Wand every second
    // and a half while she was charging a Drudge Servant, and every
    // reach put her mace away and cancelled the charge with it.
    assert!(change_of_hands_waits(Stance::Melee, Stance::Magic, true));
    assert!(change_of_hands_waits(Stance::Missile, Stance::Magic, true));

    // Answered, the gap between two swings is hers: the wand goes in
    // then, and nothing is cancelled.
    assert!(!change_of_hands_waits(Stance::Melee, Stance::Magic, false));

    // A pass that already holds a wand changes nothing, so there is
    // nothing to wait for and the buff goes up mid-fight as before.
    assert!(!change_of_hands_waits(Stance::Magic, Stance::Magic, true));

    // The rule is about hands, not about wands: a character told to
    // fight with a bow waits for the swing just the same.
    assert!(change_of_hands_waits(Stance::Melee, Stance::Missile, true));
}

#[test]
fn a_refused_wield_is_left_alone_for_longer_every_time() {
    use crate::did::Patience;
    // ACE refuses a wield it will not make with no error code at all
    // -- a caster cannot go in while a shield is up, and it says so
    // with WeenieError.None -- so there is nothing to read and
    // nothing to do but wait. +Verity asked 150 times in a minute.
    const WAND: u32 = 0x8000_00C3;
    let t0 = Instant::now();
    let mut held: Patience<u32> = Patience::new();

    held.hold(WAND, WIELD_AGAIN, t0);
    assert!(held.held(&WAND, t0), "not asked for again at once");
    assert!(
        held.held(&WAND, t0 + WIELD_AGAIN - Duration::from_millis(1)),
        "nor a moment before the wait is up"
    );
    assert!(
        !held.held(&WAND, t0 + WIELD_AGAIN),
        "asked again once the wait is up"
    );

    // Refused again: the wait doubles, so an item the server will
    // never wield in this state costs a handful of asks rather than
    // one every buff pass.
    let second = t0 + WIELD_AGAIN;
    held.hold(WAND, WIELD_AGAIN, second);
    assert_eq!(held.waited(&WAND), Some(WIELD_AGAIN * 2));
    assert!(held.held(&WAND, second + WIELD_AGAIN));
    assert!(!held.held(&WAND, second + WIELD_AGAIN * 2));

    // A wield that lands forgets the wait: the hands have changed,
    // so whatever the server was objecting to has gone.
    held.forget(&WAND);
    assert!(!held.held(&WAND, second));
    assert_eq!(held.waited(&WAND), None);
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_swing_waits_for_a_swap_the_buff_pass_started() {
    let (mut c, wand) = mid_fight(game_data());
    // The in-fight buffing reaches for the wand. The mace goes back
    // in the pack and the wand waits on empty hands.
    assert!(c.wield_for(Stance::Magic), "the swap went out");
    assert_eq!(c.autoplay.pending_wield, Some(wand));
    // Which the fight can now see. Before this, only the arming
    // code stamped the swap clock, so a swap the buff pass or the
    // softening started was invisible and the next swing went out
    // into the empty hands: +Verity put her Flaming Takuba away for
    // a wand and punched a Spikey Armoredillo.
    assert!(c.hands_changing(Instant::now()), "the swap is under way");
    // The hands still say melee -- the server has not answered the
    // put yet -- so nothing but this holds the swing back.
    assert_eq!(c.combat_stance(), Stance::Melee);
    c.tick_combat();
    assert!(!c.attack_pending, "no swing into the empty hands");
    // And the wait is bounded: a swap the server never finishes
    // cannot stop the character fighting.
    c.autoplay.last_rewield = Some(Instant::now() - SWAP_SETTLES);
    c.tick_combat();
    assert!(c.attack_pending, "swinging again once the swap is stale");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn the_weapon_comes_back_out_of_the_pack_after_a_buff() {
    let (mut c, wand) = mid_fight(game_data());
    const MACE: u32 = 0x8000_0101;
    // The buff pass put the mace down and took the wand up.
    a_weapon(
        &mut c,
        MACE,
        ac_world::item_type::MELEE_WEAPON,
        "Mace",
        false,
    );
    a_weapon(
        &mut c,
        wand,
        ac_world::item_type::CASTER,
        "Training Wand",
        true,
    );
    c.autoplay.put_down = Some(MACE);
    c.autoplay_rearm(Instant::now());
    // ACE will not put a mace in a hand that holds a caster -- it
    // refuses the wield outright, with no error to read -- so the
    // wand goes back in the pack first and the mace waits on empty
    // hands. Asking straight out was refused every single time.
    assert_eq!(c.autoplay.pending_wield, Some(MACE));
    assert_eq!(c.autoplay.wield_asked, None, "nothing was asked for yet");
    assert_eq!(c.autoplay.put_down, None, "the errand passed on");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn an_errand_the_server_is_refusing_is_kept_rather_than_dropped() {
    let (mut c, _) = mid_fight(game_data());
    const MACE: u32 = 0x8000_0101;
    const SHIELD: u32 = 0x8000_0104;
    let me = c.world.player_guid;
    a_weapon(
        &mut c,
        MACE,
        ac_world::item_type::MELEE_WEAPON,
        "Mace",
        false,
    );
    c.autoplay.put_down = Some(MACE);
    c.hold_off_wield(MACE, Instant::now());
    c.autoplay_rearm(Instant::now());
    // Asking now sends nothing, so clearing the errand would leave
    // the mace in the pack with nothing left to ask again.
    assert_eq!(c.autoplay.put_down, Some(MACE), "still owed the weapon");
    assert_eq!(c.autoplay.pending_wield, None);

    // The shield keeps its errand the same way.
    c.world.objects.insert(
        SHIELD,
        ac_world::WorldObject {
            guid: SHIELD,
            name: "Buckler".into(),
            valid_locations: ac_world::equip::SHIELD,
            container: me,
            ..Default::default()
        },
    );
    c.autoplay.wanted_shield = Some(SHIELD);
    c.hold_off_wield(SHIELD, Instant::now());
    c.autoplay_shield(Instant::now());
    assert_eq!(c.autoplay.wanted_shield, Some(SHIELD), "still owed it");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_weapon_put_down_waits_only_on_a_buff_the_urgent_pass_would_put_back() {
    use ac_world::item_type::{CASTER, MELEE_WEAPON};
    const MACE: u32 = 0x8000_0101;
    const WAND: u32 = 0x8000_0102;
    let now = Instant::now();
    let (mut c, spell) = crate::testkit::a_caster_with_a_buff_due(now);
    a_weapon(&mut c, MACE, MELEE_WEAPON, "Mace", false);
    c.attack_target = Some(0x8000_0103);
    // Half a minute left: under never_below, not run out.
    let sp = c.assets.spell_table().unwrap().get(spell).cloned().unwrap();
    c.world
        .stats
        .enchantments
        .push(ac_world::stats::Enchantment {
            spell_id: spell as u16,
            category: sp.category as u16,
            power: sp.power,
            duration: 30.0,
            received: c.session.server_time(),
            ..Default::default()
        });
    c.autoplay.put_down = Some(MACE);
    // With the wand in hand the urgent pass puts it back mid-fight.
    c.autoplay_rearm(now);
    assert_eq!(c.autoplay.put_down, Some(MACE), "rearmed with a buff due");
    // The fight took the mace up for its next target. With
    // out_of_combat_only on, the urgent pass leaves that buff for the
    // fight's end: asked by never_below, the errand waited on it every
    // tick until it ran out or the fight ended.
    a_weapon(&mut c, WAND, CASTER, "Training Wand", false);
    a_weapon(&mut c, MACE, MELEE_WEAPON, "Mace", true);
    let never_below = c.autoplay.config.buffs.never_below;
    assert!(c.due_buff(never_below, now).is_some(), "the buff went up");
    c.autoplay_rearm(now);
    assert_eq!(c.autoplay.put_down, None, "waiting on a buff left down");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_weapon_put_down_is_still_owed_while_the_server_moves_it() {
    use ac_world::item_type::{CASTER, MELEE_WEAPON};
    const MACE: u32 = 0x8000_0101;
    const WAND: u32 = 0x8000_0102;
    let now = Instant::now();
    // The urgent pass has just sent the mace to the pack and asked for
    // the wand, for a protection that has run out; the server has
    // answered neither, so the mace is still in hand.
    let (mut c, _) = crate::testkit::a_caster_with_a_buff_due(now);
    a_weapon(&mut c, WAND, CASTER, "Training Wand", false);
    a_weapon(&mut c, MACE, MELEE_WEAPON, "Mace", true);
    c.attack_target = Some(0x8000_0103);
    c.autoplay.put_down = Some(MACE);
    c.autoplay_rearm(now);
    // Looking in the pack first reads the mace in hand as an errand
    // already done, and the fight goes on with the wand.
    assert_eq!(c.autoplay.put_down, Some(MACE), "the errand was dropped");
}

#[test]
fn the_shield_goes_on_between_swings_and_not_during_one() {
    let (mut c, _) = mid_fight(no_data());
    const SHIELD: u32 = 0x8000_0104;
    let me = c.world.player_guid;
    c.world.objects.insert(
        SHIELD,
        ac_world::WorldObject {
            guid: SHIELD,
            name: "Buckler".into(),
            valid_locations: ac_world::equip::SHIELD,
            container: me,
            ..Default::default()
        },
    );
    c.autoplay.wanted_shield = Some(SHIELD);
    // A swing is in the air. ACE shuffles the stance on every
    // successful equip, a shield included, and a combat-mode change
    // cancels the attack -- so the shield waits for the gap.
    c.attack_pending = true;
    c.last_attack = Instant::now();
    assert!(c.mid_attack());
    c.autoplay_shield(Instant::now());
    assert_eq!(c.autoplay.wanted_shield, Some(SHIELD), "still to go on");
    assert_eq!(c.autoplay.wield_asked, None, "nothing sent mid-swing");
    assert!(c.wants_the_hands, "the gap after the swing is booked");
}

#[test]
#[ignore = "needs AC_DATA_DIR"]
fn a_weapon_choice_put_off_for_a_swing_is_made_once_the_fight_is_joined() {
    let (mut c, _) = mid_fight(game_data());
    let cfg = Fight::default();
    // The choice was put off: a swing was in the air when the target
    // was picked, so `arm_for` booked the gap and returned without
    // recording what it had armed for. The attack went out anyway --
    // the target that owned the swing had left the area or been
    // given up on, which clears `attack_target` and leaves
    // `attack_pending` set.
    assert_eq!(c.autoplay.armed_for, None);
    assert!(c.appraise_queue.is_empty());
    // Nothing in the air now, and the fight is joined.
    assert!(!c.mid_attack());
    assert!(c.autoplay_fight_as(Instant::now(), &cfg), "fighting");
    // Only this branch runs from here on: the picker is never
    // reached again while the target is alive. Without it the
    // character fought the whole creature with whatever the buff
    // pass had left in its hands, and a bow with no arrows chosen.
    assert!(
        !c.appraise_queue.is_empty(),
        "the weapons are being weighed for the choice"
    );
}

/// A character with a mace in hand and a wand in the pack, and
/// nothing the server is waiting on: no swing out, no spell in the
/// air.
fn hands_free(assets: std::rc::Rc<ac_scene::Assets>) -> Client {
    let (mut c, _) = mid_fight(assets);
    c.attack_target = None;
    c.attack_pending = false;
    c.autoplay.cast_sent = None;
    assert!(!c.server_busy(Instant::now()), "nothing owed to the server");
    c
}

#[test]
fn the_weapon_already_in_the_hand_is_not_asked_for_again() {
    // All 71 "You must remove your X to wield Y" lines of a
    // ten-minute run had X and Y the same item: the client asking
    // the server to wield what it was already holding.
    let mut c = hands_free(no_data());
    const MACE: u32 = 0x8000_0101;
    assert!(c.wield_guid(MACE), "the mace is in hand, so the ask stands");
    assert_eq!(c.autoplay.wield_asked, None, "and nothing was sent");
}

#[test]
fn one_wield_goes_out_once_however_often_it_is_asked_for() {
    let mut c = hands_free(no_data());
    const WAND: u32 = 0x8000_0102;
    assert!(c.wield_guid(WAND), "the wand is asked for");
    let first = c.autoplay.wield_asked;
    assert!(matches!(first, Some((WAND, _))));
    // The client thinks at 8 Hz and the answer takes a few hundred
    // milliseconds. Three ticks of asking used to be three sends,
    // and the server refused the last two for the first having
    // worked.
    assert!(c.wield_guid(WAND), "the ask already stands");
    assert!(c.wield_guid(WAND));
    assert_eq!(c.autoplay.wield_asked, first, "nothing else was sent");
    // A wield the server never answers at all is asked for again.
    c.autoplay.wield_asked = Some((WAND, Instant::now() - WIELD_ANSWERS_IN));
    assert!(c.wield_guid(WAND));
    assert_ne!(c.autoplay.wield_asked, first, "asked again once stale");
}

#[test]
fn a_refused_wield_that_worked_forgets_the_wait_rather_than_doubling_it() {
    let mut c = hands_free(no_data());
    const WAND: u32 = 0x8000_0102;
    let now = Instant::now();
    // The wand was asked for twice and taken up once. The second
    // ask comes back refused, and the world already shows the wand
    // in hand.
    a_weapon(&mut c, WAND, ac_world::item_type::CASTER, "Wand", true);
    c.autoplay.wield_asked = Some((WAND, now));
    c.wield_refused(WAND, now);
    assert!(
        !c.wield_held_off(WAND),
        "the wield worked; there is nothing to wait for"
    );
    assert_eq!(c.autoplay.wield_asked, None, "and the ask is answered");

    // A refusal for something still in the pack is a real refusal
    // and still earns its wait.
    a_weapon(&mut c, WAND, ac_world::item_type::CASTER, "Wand", false);
    c.autoplay.wield_asked = Some((WAND, now));
    c.wield_refused(WAND, now);
    assert!(c.wield_held_off(WAND), "left alone for a while");
}

#[test]
fn nothing_is_sent_to_move_an_item_while_the_server_has_us_busy() {
    // ACE refuses a take made while the character is busy and spends
    // two messages saying so -- YoureTooBusy and an
    // InventoryServerSaveFailed with nothing in it. Nine characters
    // bought 89 of those pairs in ten minutes, taking from a body
    // with a spell in the air.
    let mut c = hands_free(no_data());
    const WAND: u32 = 0x8000_0102;
    const LOOT: u32 = 0x8000_0105;
    let now = Instant::now();
    c.autoplay.cast_sent = Some(now);
    assert!(c.server_busy(now));

    assert!(!c.wield_guid(WAND), "the wand waits for a free tick");
    assert_eq!(c.autoplay.wield_asked, None, "nothing sent mid-cast");

    c.loot_queue.push_back(LOOT);
    c.tick_loot(now);
    assert!(c.loot_inflight.is_none(), "the take waits too");
    assert_eq!(
        c.loot_queue.front(),
        Some(&LOOT),
        "and keeps its place in the queue"
    );

    // A swing in the air is a different matter. ACE sets no IsBusy
    // for one -- nothing in its melee or missile path does, and the
    // wield handler does not read it at all -- so the take goes out.
    // The wield still waits, because a wield mid-swing lands and
    // cancels the swing doing it.
    c.autoplay.cast_sent = None;
    c.attack_pending = true;
    c.last_attack = now;
    assert!(!c.server_busy(now), "a swing is not the server being busy");
    assert!(c.wield_must_wait(WAND, now), "not worth a cancelled swing");
    assert!(!c.wield_guid(WAND));
    c.tick_loot(now);
    assert_eq!(
        c.loot_inflight.map(|(g, _)| g),
        Some(LOOT),
        "the take was held back for a rule the server does not have"
    );
    assert!(c.loot_queue.is_empty());

    // The swing lands, and the wand goes out too.
    c.attack_pending = false;
    assert!(c.wield_guid(WAND));
}
