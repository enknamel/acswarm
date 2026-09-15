//! Offline sessions and the things tests put round them: a [`Client`]
//! with no server, with or without the game's archives, a character
//! standing somewhere, and the creatures, corpses and teammates a rule is
//! asked about.
//!
//! A test that reads the archives builds on [`game_data`] and is marked
//! `#[ignore = "needs AC_DATA_DIR"]`; `cargo test-data` runs those.

use std::rc::Rc;

use ac_scene::Assets;
use ac_world::WorldObject;
use glam::{Quat, Vec3};

use crate::autoplay::Mate;
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
