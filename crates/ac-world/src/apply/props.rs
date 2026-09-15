use ac_net::messages;
use ac_net::wire::Reader;

use crate::{object, object_desc_flags, Applied, World};

impl World {
    pub(super) fn apply_set_stack_size(&mut self, body: &[u8]) -> Applied {
        match messages::parse_set_stack_size(body) {
            Ok((guid, stack, value)) => {
                if let Some(o) = self.objects.get_mut(&guid) {
                    o.stack_size = stack;
                    o.value = value;
                    self.generation += 1;
                    Applied::Inventory
                } else {
                    Applied::Ignored
                }
            }
            Err(_) => Applied::Failed,
        }
    }

    pub(super) fn apply_public_int(&mut self, body: &[u8]) -> Applied {
        let Ok(u) = object::PropertyUpdate::parse_public_int(body) else {
            return Applied::Failed;
        };
        let Some(o) = self.objects.get_mut(&u.guid) else {
            return Applied::Ignored;
        };
        let value = u.value.max(0) as u32;
        let applied = match u.key {
            messages::property_int::STACK_SIZE => {
                o.stack_size = value;
                Applied::Inventory
            }
            messages::property_int::VALUE => {
                o.value = value;
                Applied::Inventory
            }
            // Uses left on a healing kit or a lockpick.
            object::property_int::STRUCTURE => {
                o.structure = value;
                Applied::Inventory
            }
            object::property_int::MAX_STRUCTURE => {
                o.max_structure = value;
                Applied::Inventory
            }
            // Sent with the wield and unwield events; the
            // location is what tells a two-handed weapon from a
            // shield hand.
            object::property_int::CURRENT_WIELDED_LOCATION => {
                o.wielded_location = value;
                Applied::Inventory
            }
            // Broadcast when a player (or the projectile they
            // fired) changes PK status: the description flags
            // follow ACE's UpdateObjectDescriptionFlag.
            object::property_int::PLAYER_KILLER_STATUS => {
                o.pk_status = value;
                let mut f = o.object_desc_flags
                    & !(object_desc_flags::PLAYER_KILLER
                        | object_desc_flags::FREE_PK_STATUS
                        | object_desc_flags::PK_LITE_STATUS);
                if value == object::pk_status::PK {
                    f |= object_desc_flags::PLAYER_KILLER;
                } else if value == object::pk_status::FREE {
                    f |= object_desc_flags::FREE_PK_STATUS;
                } else if value == object::pk_status::PK_LITE {
                    f |= object_desc_flags::PK_LITE_STATUS;
                }
                o.object_desc_flags = f;
                Applied::Appearance
            }
            _ => return Applied::Ignored,
        };
        self.generation += 1;
        applied
    }

    pub(super) fn apply_public_bool(&mut self, body: &[u8]) -> Applied {
        let Ok(u) = object::PropertyUpdate::parse_public_bool(body) else {
            return Applied::Failed;
        };
        let Some(o) = self.objects.get_mut(&u.guid) else {
            return Applied::Ignored;
        };
        match u.key {
            object::property_bool::LOCKED => {
                o.locked = Some(u.value);
                if u.value {
                    o.object_desc_flags &= !object_desc_flags::OPENABLE;
                } else {
                    o.object_desc_flags |= object_desc_flags::OPENABLE;
                }
                self.generation += 1;
                Applied::Inventory
            }
            _ => Applied::Ignored,
        }
    }

    pub(super) fn apply_public_string(&mut self, body: &[u8]) -> Applied {
        let Ok(u) = object::PropertyUpdate::parse_public_string(body) else {
            return Applied::Failed;
        };
        let Some(o) = self.objects.get_mut(&u.guid) else {
            return Applied::Ignored;
        };
        if u.key == messages::property_string::NAME {
            o.name = u.value;
            self.generation += 1;
            Applied::Appearance
        } else {
            Applied::Ignored
        }
    }

    pub(super) fn apply_public_did(&mut self, body: &[u8]) -> Applied {
        let Ok(u) = object::PropertyUpdate::parse_public_did(body) else {
            return Applied::Failed;
        };
        let Some(o) = self.objects.get_mut(&u.guid) else {
            return Applied::Ignored;
        };
        match u.key {
            object::property_did::SETUP => o.setup_id = u.value,
            object::property_did::MOTION_TABLE => o.motion_table_id = u.value,
            object::property_did::ICON => o.icon_id = u.value,
            _ => return Applied::Ignored,
        }
        self.generation += 1;
        Applied::Appearance
    }

    pub(super) fn apply_public_float(&mut self, body: &[u8]) -> Applied {
        match object::PropertyUpdate::parse_public_float(body) {
            Ok(_) => Applied::Ignored,
            Err(_) => Applied::Failed,
        }
    }

    pub(super) fn apply_public_int64(&mut self, body: &[u8]) -> Applied {
        match object::PropertyUpdate::parse_public_int64(body) {
            Ok(_) => Applied::Ignored,
            Err(_) => Applied::Failed,
        }
    }

    pub(super) fn apply_private_did(&mut self, op: u32, body: &[u8]) -> Applied {
        if let Ok(u) = object::PropertyUpdate::parse_private_u32(body) {
            if u.key == object::property_did::MOTION_TABLE {
                if let Some(o) = self.player_mut() {
                    o.motion_table_id = u.value;
                    self.generation += 1;
                }
            }
        }
        self.apply_stats(op, body)
    }

    pub(super) fn apply_instance_id(&mut self, body: &[u8]) -> Applied {
        let mut r = Reader::new(body);
        let _seq = r.u8();
        match (r.u32(), r.u32(), r.u32()) {
            (Ok(guid), Ok(prop), Ok(value)) => {
                if let Some(o) = self.objects.get_mut(&guid) {
                    let v = (value != 0).then_some(value);
                    match prop {
                        2 => o.container = v,
                        3 => o.wielder = v,
                        _ => {}
                    }
                    o.parent = o.wielder.or(o.container);
                    self.generation += 1;
                }
                Applied::Inventory
            }
            _ => Applied::Failed,
        }
    }
}
