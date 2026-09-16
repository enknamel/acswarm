//! Offline sessions and the things tests put round them: a [`Client`]
//! with no server, with or without the game's archives, a character
//! standing somewhere, and the creatures, corpses and teammates a rule is
//! asked about.
//!
//! A test that reads the archives builds on [`game_data`] and is marked
//! `#[ignore = "needs AC_DATA_DIR"]`; `cargo test-data` runs those.

use std::rc::Rc;
use std::time::{Duration, Instant};

use ac_scene::Assets;
use ac_world::WorldObject;
use glam::{Quat, Vec3};

use crate::autoplay::{LootAction, Mate, TeamView, Turn, CREATURE_LEVEL};
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
