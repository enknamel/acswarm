mod containers;
mod events;
mod movement;
mod objects;
mod people;
mod props;
mod trading;

use ac_net::messages::{self, opcode};

use crate::{stats, Applied, World};

impl World {
    pub fn apply(&mut self, msg: &[u8]) -> Applied {
        let Some((op, body)) = messages::split(msg) else {
            return Applied::Ignored;
        };
        match op {
            // UpdateObject carries the same description as CreateObject:
            // the object re-described (a hook that now shows the item hung
            // on it, a changed appearance). Runtime state stays.
            opcode::OBJECT_CREATE | opcode::UPDATE_OBJECT => self.apply_object_create(body),
            opcode::PLAYER_CREATE => self.apply_player_create(body),
            opcode::UPDATE_POSITION => self.apply_update_position(body),
            // SetState (0xF74B): guid, physics state, two sequences. A
            // player logs in hidden and is shown this way; NoDraw and
            // Hidden objects stay off the scene.
            opcode::SET_STATE => self.apply_set_state(body),
            opcode::MOVEMENT_EVENT => self.apply_movement_event(body),
            opcode::OBJECT_DELETE | opcode::INVENTORY_REMOVE_OBJECT => {
                self.apply_object_delete(body)
            }
            opcode::GAME_EVENT => self.apply_game_event(op, body),
            opcode::SET_STACK_SIZE => self.apply_set_stack_size(body),
            opcode::PUBLIC_UPDATE_PROPERTY_INT => self.apply_public_int(body),
            // Locked/unlocked chests and doors, and UiHidden. The
            // Openable description flag follows Locked as ACE computes
            // it (`openable = !IsLocked`).
            opcode::PUBLIC_UPDATE_PROPERTY_BOOL => self.apply_public_bool(body),
            // A renamed object (a pet, a corpse that gained a suffix).
            opcode::PUBLIC_UPDATE_PROPERTY_STRING => self.apply_public_string(body),
            // A model, motion table or icon swap on an object in view.
            opcode::PUBLIC_UPDATE_PROPERTY_DATA_ID => self.apply_public_did(body),
            // Parsed so a bad body is reported; nothing in the world
            // reads an object's floats or 64-bit ints (emote-driven
            // stat changes on other players).
            opcode::PUBLIC_UPDATE_PROPERTY_FLOAT => self.apply_public_float(body),
            opcode::PUBLIC_UPDATE_PROPERTY_INT64 => self.apply_public_int64(body),
            // Our own motion table changed (a mount, a transformation):
            // the player object animates from the new one.
            opcode::PRIVATE_UPDATE_PROPERTY_DATA_ID => self.apply_private_did(op, body),
            // A player in view died: their health is gone until the
            // server describes them again. The message itself goes to
            // the chat log.
            opcode::PLAYER_KILLED => self.apply_player_killed(body),
            // A velocity without a position: prediction takes it.
            opcode::VECTOR_UPDATE => self.apply_vector_update(body),
            // Something in view took an item in hand: another player's
            // weapon, an arrow nocked before a shot, a creature's ammo.
            opcode::PARENT_EVENT => self.apply_parent_event(body),
            // An item left the ground (picked up by someone, or a
            // wielded item being introduced); its owner follows in an
            // instance id update or a ParentEvent.
            opcode::PICKUP_EVENT => self.apply_pickup_event(body),
            opcode::PLAY_EFFECT => self.apply_play_effect(body),
            opcode::OBJ_DESC_EVENT => self.apply_obj_desc(body),
            opcode::PUBLIC_UPDATE_INSTANCE_ID => self.apply_instance_id(body),
            _ => self.apply_stats(op, body),
        }
    }

    /// Hand a message to the character sheet and report what it changed.
    fn apply_stats(&mut self, op: u32, body: &[u8]) -> Applied {
        use stats::StatsApplied;
        match self.stats.apply(op, body) {
            Some(StatsApplied::Stats) => Applied::Stats,
            Some(StatsApplied::Spellbook { spell, known }) => {
                self.generation += 1;
                Applied::Spellbook { spell, known }
            }
            Some(StatsApplied::Enchantments) => {
                self.generation += 1;
                Applied::Enchantments
            }
            None => Applied::Ignored,
        }
    }
}
