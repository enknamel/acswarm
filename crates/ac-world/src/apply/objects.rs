use ac_net::messages;
use ac_net::wire::Reader;

use crate::{material, object, object_desc_flags, Applied, ObjectCreate, World, WorldObject};

impl World {
    pub(super) fn apply_object_create(&mut self, body: &[u8]) -> Applied {
        match ObjectCreate::parse(body) {
            Ok(oc) => {
                let is_player = self.player_guid == Some(oc.guid);
                let previous = self
                    .objects
                    .remove(&oc.guid)
                    .or_else(|| self.left_behind.remove(&oc.guid));
                if oc.object_desc_flags & object_desc_flags::PLAYER != 0 {
                    tracing::debug!(
                        "player object {} ({:#010x}): position {:?} setup {:#010x} no_draw {} parent {:?}",
                        oc.name, oc.guid, oc.position.map(|p| p.cell), oc.setup_id, oc.no_draw, oc.parent
                    );
                }
                let obj = WorldObject {
                    guid: oc.guid,
                    // Salvage bags are sent as "Salvage"; the client
                    // names them by material, as retail did.
                    name: if oc.name.starts_with("Salvage") && oc.material != 0 {
                        format!("Salvaged {}", material::name(oc.material))
                    } else {
                        oc.name
                    },
                    weenie_class_id: oc.weenie_class_id,
                    setup_id: oc.setup_id,
                    motion_table_id: oc.motion_table_id,
                    sound_table_id: oc.sound_table_id,
                    scale: if oc.scale > 0.0 { oc.scale } else { 1.0 },
                    position: oc.position,
                    parent: oc.parent,
                    no_draw: oc.no_draw,
                    is_player,
                    object_desc_flags: oc.object_desc_flags,
                    item_type: oc.item_type,
                    icon_id: oc.icon_id,
                    icon_overlay: oc.icon_overlay,
                    icon_underlay: oc.icon_underlay,
                    stack_size: oc.stack_size,
                    value: oc.value,
                    spell_id: oc.spell_id,
                    material: oc.material,
                    workmanship: oc.workmanship,
                    structure: oc.structure,
                    max_structure: oc.max_structure,
                    ammo_type: oc.ammo_type,
                    combat_use: oc.combat_use,
                    max_stack_size: oc.max_stack_size,
                    usable: oc.usable,
                    burden: oc.burden,
                    items_capacity: oc.items_capacity,
                    containers_capacity: oc.containers_capacity,
                    container: oc.container,
                    wielder: oc.wielder,
                    valid_locations: oc.valid_locations,
                    wielded_location: oc.wielded_location,
                    health: previous.as_ref().and_then(|o| o.health),
                    palette_id: oc.palette_id,
                    sub_palettes: oc.sub_palettes,
                    texture_changes: oc.texture_changes,
                    anim_part_changes: oc.anim_part_changes,
                    motion: previous.as_ref().map(|o| o.motion).unwrap_or_default(),
                    commands: previous
                        .as_ref()
                        .map(|o| o.commands.clone())
                        .unwrap_or_default(),
                    display: oc.position,
                    target: None,
                    walked_at: None,
                    velocity: oc.velocity,
                    physics_state: oc.physics_state,
                    locked: previous.as_ref().and_then(|o| o.locked),
                    pk_status: previous.as_ref().map(|o| o.pk_status).unwrap_or(0),
                    mana: previous.as_ref().and_then(|o| o.mana),
                    pet_owner: oc.pet_owner,
                    cooldown_id: oc.cooldown_id,
                    cooldown_duration: oc.cooldown_duration,
                };
                let placed = obj.position;
                self.objects.insert(obj.guid, obj);
                self.generation += 1;
                // Our own description carries where we are: the first
                // one tells `arrived_in` which world we started in, so
                // that the next move out of it is seen as a move.
                if is_player {
                    if let Some(p) = placed {
                        self.arrived_in(p.cell);
                    }
                }
                Applied::Created
            }
            Err(e) => {
                tracing::warn!("ObjectCreate: {e}");
                Applied::Failed
            }
        }
    }

    pub(super) fn apply_player_create(&mut self, body: &[u8]) -> Applied {
        if body.len() >= 4 {
            let guid = u32::from_le_bytes(body[..4].try_into().unwrap());
            self.player_guid = Some(guid);
            let placed = self.objects.get_mut(&guid).map(|o| {
                o.is_player = true;
                o.position
            });
            if let Some(Some(p)) = placed {
                self.arrived_in(p.cell);
            }
            self.generation += 1;
            Applied::PlayerSet
        } else {
            Applied::Failed
        }
    }

    pub(super) fn apply_set_state(&mut self, body: &[u8]) -> Applied {
        let mut r = Reader::new(body);
        match (r.u32(), r.u32()) {
            (Ok(guid), Ok(state)) => {
                self.bring_back(guid);
                let Some(o) = self.objects.get_mut(&guid) else {
                    return Applied::Ignored;
                };
                let hidden =
                    state & (object::PHYSICS_STATE_NO_DRAW | object::PHYSICS_STATE_HIDDEN) != 0;
                o.physics_state = state;
                if o.no_draw != hidden {
                    o.no_draw = hidden;
                    self.generation += 1;
                    Applied::Created
                } else {
                    Applied::Ignored
                }
            }
            _ => Applied::Failed,
        }
    }

    pub(super) fn apply_object_delete(&mut self, body: &[u8]) -> Applied {
        // InventoryRemoveObject: a spent/given item is gone from
        // the pack; the server sends no DeleteObject for it.
        if body.len() >= 4 {
            let guid = u32::from_le_bytes(body[..4].try_into().unwrap());
            if self.forget(guid) {
                return Applied::Deleted;
            }
        }
        Applied::Ignored
    }

    pub(super) fn apply_update_health(&mut self, rest: &[u8]) -> Applied {
        match messages::parse_update_health(rest) {
            Ok((guid, h)) => {
                if let Some(o) = self.objects.get_mut(&guid) {
                    o.health = Some(h);
                }
                Applied::Health
            }
            Err(_) => Applied::Failed,
        }
    }

    pub(super) fn apply_player_killed(&mut self, body: &[u8]) -> Applied {
        match messages::parse_player_killed(body) {
            Ok((_, victim, _)) => {
                if let Some(o) = self.objects.get_mut(&victim) {
                    o.health = Some(0.0);
                }
                Applied::Health
            }
            Err(_) => Applied::Failed,
        }
    }

    pub(super) fn apply_play_effect(&mut self, body: &[u8]) -> Applied {
        match messages::parse_play_effect(body) {
            Ok((guid, script, _)) => Applied::Effect { guid, script },
            Err(_) => Applied::Failed,
        }
    }

    pub(super) fn apply_obj_desc(&mut self, body: &[u8]) -> Applied {
        match object::ObjDescEvent::parse(body) {
            Ok(ev) => {
                if let Some(o) = self.objects.get_mut(&ev.guid) {
                    o.palette_id = ev.desc.palette_id;
                    o.sub_palettes = ev.desc.sub_palettes;
                    o.texture_changes = ev.desc.texture_changes;
                    o.anim_part_changes = ev.desc.anim_part_changes;
                    self.generation += 1;
                    Applied::Appearance
                } else {
                    Applied::Ignored
                }
            }
            Err(e) => {
                tracing::warn!("ObjDescEvent: {e}");
                Applied::Failed
            }
        }
    }
}
