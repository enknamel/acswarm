use ac_net::messages;
use ac_net::wire::Reader;

use crate::{object, Applied, World};

impl World {
    pub(super) fn apply_put_in_container(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        match (r.u32(), r.u32(), r.u32()) {
            (Ok(item), Ok(container), Ok(_placement)) => {
                if let Some(o) = self.objects.get_mut(&item) {
                    o.container = Some(container);
                    o.wielder = None;
                    o.wielded_location = 0;
                    o.position = None;
                    o.display = None;
                    o.parent = Some(container);
                    self.generation += 1;
                }
                if let Some((c, items)) = &mut self.open_container {
                    if *c != container {
                        items.retain(|g| *g != item);
                    } else if !items.contains(&item) {
                        // Stored into the chest (or hook) we are
                        // looking into: it shows up there.
                        items.push(item);
                    }
                }
                Applied::Inventory
            }
            _ => Applied::Failed,
        }
    }

    pub(super) fn apply_wield_object(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        match (r.u32(), r.u32()) {
            (Ok(item), Ok(location)) => {
                if let Some(o) = self.objects.get_mut(&item) {
                    o.wielder = self.player_guid;
                    o.wielded_location = location;
                    o.container = None;
                    o.position = None;
                    o.display = None;
                    o.parent = self.player_guid;
                    self.generation += 1;
                }
                Applied::Inventory
            }
            _ => Applied::Failed,
        }
    }

    pub(super) fn apply_view_contents(&mut self, rest: &[u8]) -> Applied {
        match messages::parse_view_contents(rest) {
            Ok((c, items)) => {
                for (g, _) in &items {
                    if let Some(o) = self.objects.get_mut(g) {
                        o.container = Some(c);
                        o.parent = Some(c);
                    }
                }
                // Our own side packs are listed the same way at
                // login; only a container in the world opens the
                // loot window.
                let me = self.player_guid;
                let carried =
                    me.is_some() && self.objects.get(&c).is_some_and(|o| o.container == me);
                if !carried {
                    self.open_container = Some((c, items.into_iter().map(|(g, _)| g).collect()));
                }
                Applied::Inventory
            }
            Err(_) => Applied::Failed,
        }
    }

    pub(super) fn apply_approach_vendor(&mut self, rest: &[u8]) -> Applied {
        match object::ApproachVendor::parse(rest) {
            Ok(v) => {
                self.open_vendor = Some(v);
                Applied::Vendor
            }
            Err(e) => {
                tracing::warn!("ApproachVendor: {e}");
                tracing::debug!(
                    "ApproachVendor payload: {}",
                    rest.iter().map(|b| format!("{b:02x}")).collect::<String>()
                );
                Applied::Failed
            }
        }
    }

    pub(super) fn apply_close_ground_container(&mut self) -> Applied {
        self.open_container = None;
        Applied::Inventory
    }

    pub(super) fn apply_item_mana(&mut self, rest: &[u8]) -> Applied {
        let mut r = Reader::new(rest);
        match (r.u32(), r.f32(), r.u32()) {
            (Ok(item), Ok(mana), Ok(success)) => {
                if let Some(o) = self.objects.get_mut(&item) {
                    o.mana = (success != 0).then_some(mana);
                }
                Applied::Inventory
            }
            _ => Applied::Failed,
        }
    }

    pub(super) fn apply_put_in_3d(&mut self, rest: &[u8]) -> Applied {
        if let Ok(item) = Reader::new(rest).u32() {
            if let Some(o) = self.objects.get_mut(&item) {
                o.container = None;
                o.wielder = None;
                o.parent = None;
                self.generation += 1;
            }
        }
        Applied::Inventory
    }

    pub(super) fn apply_parent_event(&mut self, body: &[u8]) -> Applied {
        match object::ParentEvent::parse(body) {
            Ok(ev) => {
                let Some(o) = self.objects.get_mut(&ev.child) else {
                    return Applied::Ignored;
                };
                o.wielder = Some(ev.parent);
                o.parent = Some(ev.parent);
                o.container = None;
                o.position = None;
                o.display = None;
                self.generation += 1;
                Applied::Inventory
            }
            Err(e) => {
                tracing::warn!("ParentEvent: {e}");
                Applied::Failed
            }
        }
    }

    pub(super) fn apply_pickup_event(&mut self, body: &[u8]) -> Applied {
        match object::parse_pickup_event(body) {
            Ok((guid, _, _)) => {
                let Some(o) = self.objects.get_mut(&guid) else {
                    return Applied::Ignored;
                };
                o.position = None;
                o.display = None;
                o.target = None;
                self.generation += 1;
                Applied::Inventory
            }
            Err(_) => Applied::Failed,
        }
    }
}
