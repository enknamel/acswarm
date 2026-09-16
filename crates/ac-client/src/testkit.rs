//! Offline sessions and the things tests put round them: a [`Client`]
//! with no server, with or without the game's archives, a character
//! standing somewhere, and the creatures, corpses and teammates a rule is
//! asked about.
//!
//! A test that reads the archives builds on [`game_data`] and is marked
//! `#[ignore = "needs AC_DATA_DIR"]`; `cargo test-data` runs those.

use std::rc::Rc;
use std::time::{Duration, Instant};

use ac_agent::recent::Recent;
use ac_scene::Assets;
use ac_world::{item_type, object_desc_flags, WorldObject};
use glam::{Quat, Vec3};

use crate::autoplay::growth::{Growth, Salable};
use crate::autoplay::{judge_loot, LootAction, Mate, TeamView, Turn, CREATURE_LEVEL};
use crate::items::ItemStats;
use crate::player::Player;
use crate::Client;

/// The character's own guid, where a test does not name another.
pub const ME: u32 = 0x5000_0001;
/// The setup and motion table of a human player character.
pub const HUMAN_SETUP: u32 = 0x0200_0001;
pub const HUMAN_MOTION_TABLE: u32 = 0x0900_0001;

/// No game data: every archive lookup is not found.
pub fn no_data() -> Rc<Assets> {
    Rc::new(Assets::empty())
}

/// The game's archives from `AC_DATA_DIR` (panics when it is unset).
pub fn game_data() -> Rc<Assets> {
    let dir = ac_dat::test_data_dir();
    let assets = Assets::open(&dir)
        .unwrap_or_else(|e| panic!("opening the archives in {}: {e}", dir.display()));
    Rc::new(assets)
}

/// A session with no server and no game data.
pub fn offline_client() -> Client {
    Client::offline(no_data())
}

/// A character of `level` with guid [`ME`] and nothing around it.
pub fn character_of_level(assets: Rc<Assets>, level: i32) -> Client {
    let mut c = Client::offline(assets);
    c.world.player_guid = Some(ME);
    c.world.stats.level = level;
    c
}

/// A human body at `local` in `cell`, moving the way a player does.
pub fn human(assets: &Assets, cell: u32, local: Vec3, rotation: Quat) -> Player {
    let mut pl = Player::new(assets, cell, local, rotation);
    pl.set_motion_table(assets, HUMAN_SETUP, HUMAN_MOTION_TABLE);
    pl
}

/// Put `c`'s character on its feet at `local` in `cell`, facing north.
pub fn stand(c: &mut Client, cell: u32, local: Vec3) {
    c.player = Some(human(&c.assets, cell, local, Quat::IDENTITY));
}

/// A session over the game's archives whose character stands at `local`
/// in `cell`, with no guid of its own yet.
pub fn standing_at(cell: u32, local: Vec3) -> Client {
    let mut c = Client::offline(game_data());
    stand(&mut c, cell, local);
    c
}

/// [`standing_at`], as a character of `level` with guid [`ME`].
pub fn standing_in_the_field(level: i32, cell: u32, local: Vec3) -> Client {
    let mut c = character_of_level(game_data(), level);
    stand(&mut c, cell, local);
    c
}

/// A living creature called `name`, nowhere in particular.
pub fn creature(guid: u32, name: &str) -> WorldObject {
    WorldObject {
        guid,
        name: name.into(),
        item_type: ac_world::item_type::CREATURE,
        health: Some(1.0),
        ..Default::default()
    }
}

/// A corpse called `name`, nowhere in particular.
pub fn corpse(guid: u32, name: &str) -> WorldObject {
    WorldObject {
        guid,
        name: name.into(),
        object_desc_flags: ac_world::object_desc_flags::CORPSE,
        ..Default::default()
    }
}

/// When each body came into sight, as `Autoplay::corpse_seen` holds it.
pub fn corpses_seen(rows: impl IntoIterator<Item = (u32, Instant)>) -> Recent<u32> {
    let mut seen = Recent::new();
    for (guid, when) in rows {
        seen.mark(guid, when);
    }
    seen
}

/// An object's position for the world point `at`, in `cell`'s landblock.
pub fn placed(cell: u32, at: Vec3) -> Option<ac_world::object::Position> {
    Some(ac_world::object::Position::new_flat(
        cell,
        at - ac_world::landblock_origin(cell),
    ))
}

/// A teammate called `name`, everything else at its default.
pub fn mate(guid: u32, name: &str) -> Mate {
    Mate {
        guid,
        name: name.into(),
        ..Default::default()
    }
}

pub fn item(name: &str, value: u32, armor: u32) -> ItemStats {
    ItemStats {
        name: name.into(),
        value,
        armor_level: armor,
        appraised: true,
        kind: if armor > 0 { "armor" } else { "misc" },
        ..Default::default()
    }
}

/// A rule that claims what `line` matches, in the inventory's own
/// search language.
pub fn asks(name: &str, line: &str, action: LootAction) -> crate::profile::Rule {
    crate::profile::Rule {
        name: name.into(),
        action,
        all: vec![crate::profile::Ask::Search(line.into())],
        ..Default::default()
    }
}

/// One of the others, standing at `at`, with `looting` in hand for
/// `held`.
pub fn looter(guid: u32, at: glam::Vec3, looting: Option<u32>, held: Duration) -> Mate {
    Mate {
        name: format!("Bryn{guid:02}"),
        guid,
        world: at,
        health: 1.0,
        autoplay: true,
        looting,
        looting_for: held,
        opens_bodies: true,
        ..Default::default()
    }
}

/// One profile for a whole party: a broken key for whoever can mend
/// it, and healing kits up to four.
pub fn a_party_profile() -> crate::profile::Profile {
    use crate::profile::{Ask, Mine, Profile, Rule};
    Profile {
        name: "party".into(),
        rules: vec![
            Rule {
                name: "broken keys, if I can mend them".into(),
                action: LootAction::Keep,
                all: vec![
                    Ask::Search("broken".into()),
                    Ask::Me(Mine::Skill {
                        skill: ac_world::stats::skill::LOCKPICK,
                        op: crate::items::Op::Ge,
                        level: 250,
                    }),
                ],
                ..Default::default()
            },
            Rule {
                name: "healing kits, a few".into(),
                action: LootAction::Keep,
                all: vec![Ask::Search("healing kit".into())],
                keep_up_to: Some(4),
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

/// A character as the turns read it: standing at `at`, free, with
/// room, having opened `opened` bodies first lately.
pub fn turn_at(guid: u32, at: glam::Vec3, opened: u16) -> Turn {
    Turn {
        guid,
        world: at,
        looting: false,
        fighting: false,
        room: true,
        opened,
    }
}

pub fn view_of(mates: Vec<Mate>) -> TeamView {
    TeamView {
        mates,
        ..Default::default()
    }
}

/// A creature of `wcid` standing in view, and a copy of it to ask
/// the fight rules about.
pub fn in_view(c: &mut Client, guid: u32, wcid: u32, name: &str) -> ac_world::WorldObject {
    let o = ac_world::WorldObject {
        weenie_class_id: wcid,
        ..creature(guid, name)
    };
    c.world.objects.insert(guid, o.clone());
    o
}

/// The GameEvent behind "You evade <name>'s attack.": the server's
/// word that `name` swung at the character and missed.
pub fn evaded(guid: u32, name: &str) -> Vec<u8> {
    let mut w = ac_net::wire::Writer::new();
    w.u32(guid)
        .u32(0)
        .u32(ac_net::messages::event::EVASION_DEFENDER_NOTIFICATION)
        .string16(name);
    w.finish()
}

/// An appraisal of `guid` saying it is `level`, with the health the
/// server sends whether or not the assessment succeeded.
pub fn appraised_as(c: &mut Client, guid: u32, level: Option<i32>, health: u32) {
    c.appraisals.insert(
        guid,
        ac_net::messages::Appraisal {
            guid,
            success: true,
            ints: level.map(|l| (CREATURE_LEVEL, l)).into_iter().collect(),
            creature: Some(ac_net::messages::CreatureProfile {
                health,
                health_max: health,
                ..Default::default()
            }),
            ..Default::default()
        },
    );
}

/// A weenie no table has heard of.
pub const STRANGER: u32 = 0x00FF_FFF0;

/// A weapon in the character's hand, or in its pack.
pub fn a_weapon(c: &mut Client, guid: u32, kind: u32, name: &str, in_hand: bool) {
    let me = c.world.player_guid;
    let locations = if kind == ac_world::item_type::CASTER {
        ac_world::equip::HELD
    } else {
        ac_world::equip::MELEE_WEAPON
    };
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            item_type: kind,
            value: 100,
            valid_locations: locations,
            container: if in_hand { None } else { me },
            wielder: if in_hand { me } else { None },
            ..Default::default()
        },
    );
}

/// A character with a mace in hand, a wand in the pack, and a
/// creature it is swinging at.
pub fn mid_fight(assets: std::rc::Rc<ac_scene::Assets>) -> (Client, u32) {
    const MACE: u32 = 0x8000_0101;
    const WAND: u32 = 0x8000_0102;
    const CREATURE: u32 = 0x8000_0103;
    let mut c = character_of_level(assets, 20);
    a_weapon(
        &mut c,
        MACE,
        ac_world::item_type::MELEE_WEAPON,
        "Mace",
        true,
    );
    a_weapon(
        &mut c,
        WAND,
        ac_world::item_type::CASTER,
        "Training Wand",
        false,
    );
    in_view(&mut c, CREATURE, 19257, "Drudge Skulker");
    c.combat = true;
    c.attack_target = Some(CREATURE);
    c.last_attack = Instant::now() - Duration::from_secs(5);
    (c, WAND)
}

pub fn salable(guid: u32, item_type: u32, value: u32, stack: u32) -> Salable {
    Salable {
        guid,
        item_type,
        value,
        stack,
    }
}

/// A Revenant called `name` standing `metres` east of the character:
/// a real fight at level 20, not a critter walked past for its own
/// sake.
pub fn standing_by(c: &mut Client, guid: u32, name: &str, metres: f32) -> ac_world::WorldObject {
    let pl = c.player.as_ref().unwrap();
    let (cell, me) = (pl.cell, pl.world_position());
    let o = ac_world::WorldObject {
        guid,
        weenie_class_id: 8592,
        name: name.into(),
        item_type: ac_world::item_type::CREATURE,
        object_desc_flags: ac_world::object_desc_flags::ATTACKABLE,
        health: Some(1.0),
        position: Some(ac_world::object::Position::new_flat(
            cell,
            me + glam::Vec3::new(metres, 0.0, 0.0) - ac_world::landblock_origin(cell),
        )),
        ..Default::default()
    };
    c.world.objects.insert(guid, o.clone());
    o
}

/// A vendor's window as the server sends it, with nothing on the
/// shelf.
pub fn window_of(vendor: u32) -> ac_world::object::ApproachVendor {
    ac_world::object::ApproachVendor {
        vendor,
        item_types: 0,
        min_value: 0,
        max_value: 0,
        magical: false,
        buy_rate: 1.0,
        sell_rate: 1.0,
        alt_currency: 0,
        alt_amount: 0,
        alt_name: String::new(),
        items: Vec::new(),
    }
}

/// A vendor standing `off` from the character, in view.
pub fn vendor_beside(c: &mut Client, guid: u32, name: &str, off: glam::Vec3) {
    let holtburg = 0xA9B4_0019;
    let me = c.player.as_ref().unwrap().world_position();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            item_type: ac_world::item_type::CREATURE,
            object_desc_flags: object_desc_flags::VENDOR,
            position: Some(ac_world::object::Position::new_flat(
                holtburg,
                me + off - ac_world::landblock_origin(holtburg),
            )),
            ..Default::default()
        },
    );
}

/// The character, with `capacity` slots in its main pack.
pub fn with_a_pack(c: &mut Client, capacity: u32) {
    let me = 0x5000_0001;
    c.world.player_guid = Some(me);
    c.world.objects.insert(
        me,
        ac_world::WorldObject {
            guid: me,
            name: "Verity".into(),
            is_player: true,
            items_capacity: capacity,
            ..Default::default()
        },
    );
}

/// A pea in the pack, taken to sell.
pub fn pea_in_the_pack(c: &mut Client, guid: u32, name: &str, wcid: u32, value: u32) {
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            weenie_class_id: wcid,
            item_type: item_type::SPELL_COMPONENTS,
            value,
            stack_size: 1,
            max_stack_size: 100,
            container: Some(me),
            ..Default::default()
        },
    );
    let stats = c.stats_of(guid).unwrap();
    c.autoplay.tag(&stats, LootAction::Sell);
}

/// A wand in hand, a bolt in the book and the Foci of Strife in the
/// pack: a war mage, whose every cast burns a scarab and a prismatic
/// taper.
pub fn as_a_war_mage(c: &mut Client) {
    const FLAME_BOLT_I: u32 = 27;
    const FOCI_OF_STRIFE: u32 = 15271;
    let me = c.world.player_guid.unwrap();
    c.world.stats.spells = vec![FLAME_BOLT_I];
    c.world.objects.insert(
        0x8000_0040,
        ac_world::WorldObject {
            guid: 0x8000_0040,
            name: "Wand".into(),
            item_type: item_type::CASTER,
            value: 100,
            wielder: Some(me),
            parent: Some(me),
            ..Default::default()
        },
    );
    c.world.objects.insert(
        0x8000_0041,
        ac_world::WorldObject {
            guid: 0x8000_0041,
            name: "Foci of Strife".into(),
            weenie_class_id: FOCI_OF_STRIFE,
            value: 100,
            container: Some(me),
            ..Default::default()
        },
    );
}

/// `stack` prismatic tapers in the pack.
pub fn tapers_in_the_pack(c: &mut Client, guid: u32, stack: u32) {
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: "Prismatic Taper".into(),
            weenie_class_id: 20631,
            item_type: item_type::SPELL_COMPONENTS,
            value: stack,
            stack_size: stack,
            max_stack_size: 1_000,
            container: Some(me),
            ..Default::default()
        },
    );
}

/// A stack of `stack` `name` in the pack, a spell component, with
/// nothing written down about it.
pub fn component_in_the_pack(c: &mut Client, guid: u32, name: &str, wcid: u32, stack: u32) {
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            weenie_class_id: wcid,
            item_type: item_type::SPELL_COMPONENTS,
            value: 5 * stack,
            stack_size: stack,
            max_stack_size: 1_000,
            container: Some(me),
            ..Default::default()
        },
    );
}

/// The weenie class the archives give a component of this name.
pub fn component_named(c: &Client, name: &str) -> u32 {
    let table = c.assets.spell_components().unwrap();
    let id = table.find_by_name(name).expect(name);
    c.assets
        .spell_component_ids()
        .unwrap()
        .component_wcid(id)
        .expect(name)
}

/// Write down what the profile makes of this item now, as the
/// arrival pass does: the item is judged by the rules once, when it
/// is taken, and the answer travels with it.
pub fn tagged_by_the_profile(c: &mut Client, guid: u32) -> LootAction {
    let stats = c.stats_of(guid).unwrap();
    let action = c.loot_action(&stats).expect("a rule claims it");
    c.autoplay.tag(&stats, action);
    action
}

/// The war mage in front of Cindrue's open window, which buys
/// components: what she is offered, and what the run does first.
pub fn at_cindrues_counter(c: &mut Client, cfg: &Growth) -> (Vec<u32>, ac_vendor::Next) {
    let cindrue = 0x8000_0002;
    vendor_beside(
        c,
        cindrue,
        "Archmage Cindrue",
        glam::Vec3::new(1.0, 0.0, 0.0),
    );
    let mut window = window_of(cindrue);
    window.item_types = item_type::SPELL_COMPONENTS;
    c.world.open_vendor = Some(window);
    let snap = c.vendor_snapshot(cfg);
    let mut offered: Vec<u32> = snap
        .items
        .iter()
        .filter(|i| snap.offers(i))
        .map(|i| i.guid)
        .collect();
    offered.sort_unstable();
    let next = ac_vendor::Run::new().step(&snap, Instant::now());
    (offered, next)
}

/// The character's buy list, and nothing else on it: `what`, `keep`
/// of them, urgent at `restock_at` or fewer. The starter's rules.
pub fn with_a_buy_list(c: &mut Client, lines: &[(&str, u32, u32)]) {
    with_a_profile(c, "wants", crate::profile::Profile::starter().rules, lines);
}

/// A profile of the character's own, `name`, with these `rules` in
/// this order and this buy list. A shelf of its own, so the one
/// every session shares is not touched.
pub fn with_a_profile(
    c: &mut Client,
    name: &str,
    rules: Vec<crate::profile::Rule>,
    lines: &[(&str, u32, u32)],
) {
    let dir = std::env::temp_dir().join("acswarm-test-growth-profiles");
    std::fs::create_dir_all(&dir).ok();
    let shelf = std::sync::Arc::new(crate::profile::Library::default());
    shelf.open(&dir);
    let mut p = crate::profile::Profile::starter();
    p.name = name.into();
    p.rules = rules;
    p.buy.clear();
    for (what, keep, restock_at) in lines {
        p.buy.push(crate::profile::Buy {
            what: (*what).into(),
            keep: *keep,
            restock_at: Some(*restock_at),
            on: true,
            ..Default::default()
        });
    }
    shelf.put(p).ok();
    c.profiles = shelf;
    c.autoplay.config.loot.profile = name.into();
}

/// One rule: `action` for anything whose name contains `word`.
pub fn word_rule(name: &str, word: &str, action: LootAction) -> crate::profile::Rule {
    crate::profile::Rule {
        name: name.into(),
        action,
        all: vec![crate::profile::Ask::Item(crate::items::Term::Word(
            word.into(),
        ))],
        ..Default::default()
    }
}

/// "The rest, to the counter": a rule that claims anything at all.
pub fn the_rest_to_the_counter() -> crate::profile::Rule {
    crate::profile::Rule {
        name: "the rest".into(),
        action: LootAction::Sell,
        all: vec![crate::profile::Ask::Item(crate::items::Term::Num(
            crate::items::NumKey::Value,
            crate::items::Op::Ge,
            0.0,
        ))],
        ..Default::default()
    }
}

/// Coin in the pack.
pub fn coin_in_the_pack(c: &mut Client, guid: u32, amount: u32) {
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: "Pyreal".into(),
            item_type: item_type::MONEY,
            value: 1,
            stack_size: amount,
            max_stack_size: 25_000,
            container: Some(me),
            ..Default::default()
        },
    );
}

/// The shop of this name.
pub fn shop_named(name: &str) -> &'static ac_world::shops::Shop {
    ac_world::shops::all()
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no shop {name}"))
}

/// A counter in view at `at`.
pub fn a_counter(c: &mut Client, guid: u32, name: &str, at: glam::Vec3) {
    let mut o = ac_world::WorldObject {
        guid,
        name: name.into(),
        item_type: item_type::CREATURE,
        object_desc_flags: object_desc_flags::VENDOR,
        ..Default::default()
    };
    o.position = Some(ac_world::object::Position {
        cell: 0xA9B4_0019,
        local: at,
        rotation: glam::Quat::IDENTITY,
    });
    c.world.objects.insert(guid, o);
}

/// A shelf of its own holding one starter profile, so the looting
/// has rules to carry out without touching the one every session
/// shares.
pub fn a_loot_profile(c: &mut Client, name: &str) {
    let dir = std::env::temp_dir().join("acswarm-test-loot-profiles");
    std::fs::create_dir_all(&dir).ok();
    let shelf = std::sync::Arc::new(crate::profile::Library::default());
    shelf.open(&dir);
    let mut p = crate::profile::Profile::starter();
    p.name = name.into();
    shelf.put(p).ok();
    c.profiles = shelf;
    c.autoplay.config.loot.profile = name.into();
    assert!(c.loot_profile().is_some(), "no rules to loot by");
}

/// A shelf holding one profile of `rules`, in a directory of its
/// own so that two tests never read each other's files.
pub fn shelf(named: &str, rules: Vec<crate::profile::Rule>) -> crate::profile::Library {
    let dir = std::env::temp_dir().join(format!("acswarm-{named}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let library = crate::profile::Library::default();
    library.open(&dir);
    library
        .put(crate::profile::Profile {
            name: "test".into(),
            rules,
            ..Default::default()
        })
        .expect("saved");
    library
}

pub fn judged(
    stats: &ItemStats,
    library: &crate::profile::Library,
    profile: &str,
) -> crate::profile::Verdict {
    judge_loot(
        stats,
        None,
        library.get(profile).as_deref(),
        &crate::weapons::Wielder::default(),
        "Aldric",
        0,
    )
}

/// Set the name lists on the test profile. They are the profile's
/// now, so they change for everyone reading it.
pub fn name_lists(library: &crate::profile::Library, always: &[&str], never: &[&str]) {
    let mut p = (*library.get("test").expect("the test profile")).clone();
    p.looting.always = always.iter().map(|s| s.to_string()).collect();
    p.looting.never = never.iter().map(|s| s.to_string()).collect();
    library.put(p).expect("put");
}

/// A corpse open at the character's feet, as the loot rules see it,
/// with one thing on it worth taking.
pub fn corpse_at_hand(guid: u32, item: u32) -> ac_loot::Open {
    ac_loot::Open {
        guid,
        name: "Corpse of a Drudge Skulker".into(),
        open: true,
        items: vec![ac_loot::Lying {
            guid: item,
            name: "Dagger".into(),
            burden: 10,
            verdict: ac_loot::Verdict::Take(LootAction::Keep),
            needs_no_slot: false,
        }],
        slots_free: 20,
        room_anywhere: 20,
        carry_room: 10_000,
        ..Default::default()
    }
}

/// A Sack hanging from the main pack, with `capacity` slots.
pub const SACK: u32 = 0x8000_0300;
/// A body at the character's feet.
pub const BODY: u32 = 0x8000_0400;

/// A character whose main pack has `main_slots` slots, `main_used`
/// of them taken by daggers, and a Sack of `sack_slots` slots with
/// `sack_used` daggers in it.
pub fn with_packs(
    assets: std::rc::Rc<ac_scene::Assets>,
    main_slots: u32,
    main_used: u32,
    sack_slots: u32,
    sack_used: u32,
) -> Client {
    let mut c = character_of_level(assets, 20);
    let me = c.world.player_guid.unwrap();
    c.world.objects.insert(
        me,
        ac_world::WorldObject {
            guid: me,
            name: "Verity".into(),
            is_player: true,
            items_capacity: main_slots,
            ..Default::default()
        },
    );
    c.world.objects.insert(
        SACK,
        ac_world::WorldObject {
            guid: SACK,
            name: "Sack".into(),
            weenie_class_id: 166,
            item_type: ac_world::item_type::CONTAINER,
            items_capacity: sack_slots,
            container: Some(me),
            ..Default::default()
        },
    );
    let mut next = 0x8000_0500;
    for (holder, n) in [(me, main_used), (SACK, sack_used)] {
        for _ in 0..n {
            c.world.objects.insert(
                next,
                ac_world::WorldObject {
                    guid: next,
                    name: "Dagger".into(),
                    container: Some(holder),
                    ..Default::default()
                },
            );
            next += 1;
        }
    }
    c.world.objects.insert(
        BODY,
        ac_world::WorldObject {
            guid: BODY,
            name: "Corpse of a Drudge Skulker".into(),
            object_desc_flags: ac_world::object_desc_flags::CORPSE,
            ..Default::default()
        },
    );
    // No slots kept back for a counter's money: these are about the
    // packs being full, not low.
    c.autoplay.config.team.restock.keep_slots = 0;
    assert!(!c.server_busy(Instant::now()));
    c
}

/// A thing of `wcid` lying on the body, `count` to the stack.
pub fn on_the_body(c: &mut Client, guid: u32, name: &str, wcid: u32, kind: u32, count: u32) {
    c.world.objects.insert(
        guid,
        ac_world::WorldObject {
            guid,
            name: name.into(),
            weenie_class_id: wcid,
            item_type: kind,
            stack_size: count,
            max_stack_size: if count > 1 { 25_000 } else { 1 },
            container: Some(BODY),
            ..Default::default()
        },
    );
    match &mut c.world.open_container {
        Some((body, items)) if *body == BODY => items.push(guid),
        _ => c.world.open_container = Some((BODY, vec![guid])),
    }
}
