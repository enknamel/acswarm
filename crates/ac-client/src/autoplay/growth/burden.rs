use super::Growth;
use crate::Client;

/// Twice its capacity is as much as burden can take from a character.
/// The server scales Melee and Missile Defense, and what the Run skill
/// adds to its speed, by two less its burden in multiples of capacity:
/// all of them at one, half at one and a half, none at all at two.
const DEFENSELESS_AT: f32 = 2.0;

/// How much more loot a character will take on before it has had
/// enough. `carried` is everything, gear and all; `loot` is the part of
/// it a counter would take.
///
/// Three things stop it, and the room is what the nearest of them
/// leaves:
///
/// * the working limit, `carry_up_to` times its capacity, measured on
///   the loot alone;
/// * twice its capacity, measured on everything, where it has no
///   defense left (or the working limit's own multiple, when the
///   player has set it higher than that);
/// * the server's wall at three times, measured on everything, past
///   which it hands over nothing at all.
///
/// The limit counts only loot because nothing else can be sold off. It
/// used to count everything, and that stopped Blargerton looting
/// altogether: Strength 60 gives a capacity of 9000 and a limit of
/// 13500 at one and a half times, and he carried 13866, nearly all of
/// it things he keeps -- his plate alone is 6540, then four foci and a
/// stack of Prismatic Tapers. No room at all, so every corpse he opened
/// was shut again at once and marked looted, and he took nothing; the
/// town run read the same zero as laden and could send him to sell with
/// nothing to sell. Leaving out only what he wore was not enough: at a
/// limit of 0.8 the foci and the tapers alone filled it.
///
/// Leaving what it keeps out of the limit is also what let a character
/// in plate hunt on past twice its capacity, with no defense, before it
/// was laden -- hence the second line. That line gives way when what
/// the character keeps weighs that much on its own: loot cannot slow it
/// any further, and a line no sale can bring it back under would leave
/// every corpse untouched again. The wall never gives way.
fn loot_room(carried: u32, loot: u32, capacity: u32, carry_up_to: f32) -> u32 {
    let up_to = carry_up_to.max(0.0);
    // The server's figure is the one to trust: a count of loot that has
    // got ahead of it is all loot, and nothing is kept.
    let loot = loot.min(carried);
    let kept = carried - loot;
    let limit = (capacity as f32 * up_to) as u32;
    let line = (capacity as f32 * up_to.max(DEFENSELESS_AT)) as u32;
    let wall = capacity.saturating_mul(3);
    let room = limit.saturating_sub(loot).min(wall.saturating_sub(carried));
    if kept < line {
        room.min(line.saturating_sub(carried))
    } else {
        room
    }
}

/// Whether a character with `room` for more loot has had enough and
/// should go and sell: no room at all, or less than the lightest thing
/// the looting left on a body for its weight (`left`) -- as long as
/// selling what it carries would make room for that thing (`sold` is the
/// room it would have then).
///
/// Room used to have to be exactly nothing, and it almost never is. The
/// loot rules take only what fits, so the room settles a little above
/// nothing and below whatever is still lying there: a character forty
/// short of a mace shut every body with one on it as too laden, hunted
/// on, and never went to sell. A thing no sale could make room for is no
/// reason to go.
fn had_enough(room: u32, left: Option<u32>, sold: u32) -> bool {
    room == 0 || left.is_some_and(|burden| room < burden && burden <= sold)
}

/// Whether a character carrying `carried` is past the point where the
/// server hands it anything at all: three times its `capacity`, and
/// weight has nothing to do with it there. A Pyreal weighs nothing, and
/// +Verity at 36462 of a 7500 capacity walked off her way to town for
/// one, was told "You are too encumbered to carry that!", and went back
/// for it twice more. With no capacity known yet nothing is past it.
pub(super) fn past_the_wall(carried: u32, capacity: u32) -> bool {
    capacity > 0 && carried > capacity.saturating_mul(3)
}

impl Client {
    /// What the character is carrying, in burden units, and the most
    /// it may carry.
    ///
    /// The capacity is a hundred and fifty times Strength, plus thirty
    /// more per rank of the carrying-capacity augmentation; the server
    /// refuses to hand over anything that would take the character past
    /// three times that. The comfortable place to work is well under
    /// it: a character at twice its capacity is slow and has no Melee or
    /// Missile Defense left, and a character at three times it cannot
    /// pick up what it kills.
    pub fn burden(&self) -> (u32, u32) {
        const ENCUMBRANCE_VAL: u32 = 5;
        const CARRY_AUGMENTATION: u32 = 230;
        let int_of = |k: u32| {
            self.world
                .stats
                .ints
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, v)| (*v).max(0) as u32)
        };
        // What the server says we are carrying, or the pack counted up
        // when it has not said.
        //
        // Counted the way the server counts it, which is not the
        // obvious way. A stack's weight is the whole stack's already,
        // so multiplying by its size counts it twice over; and a side
        // pack's weight is its own plus everything inside it, so adding
        // the contents as well counts those twice too. Only what hangs
        // directly off the character is added: the main pack's items,
        // side packs included as the single lumps they weigh, and
        // whatever is being worn or held.
        let now = int_of(ENCUMBRANCE_VAL).unwrap_or_else(|| {
            self.world
                .main_pack()
                .chain(self.world.wielded())
                .map(|o| o.burden)
                .sum()
        });
        let strength = self.wielder().attributes_current[0];
        let augmented = 150 + 30 * int_of(CARRY_AUGMENTATION).unwrap_or(0);
        (now, strength.saturating_mul(augmented))
    }

    /// How much more the character may be handed before the server
    /// starts refusing: three times its capacity, less what it carries.
    pub fn burden_room(&self) -> u32 {
        let (now, capacity) = self.burden();
        capacity.saturating_mul(3).saturating_sub(now)
    }

    /// How much more loot the character will take on before it stops
    /// hunting and goes to sell. Zero means it has had enough.
    ///
    /// The loot profile's `carry_up_to` is measured on the loot alone:
    /// what the selling rules would hand a counter. What the character
    /// wears and wields, and what the rules keep -- foci, the components
    /// its spells burn, what it keeps stocked, what it took to keep or
    /// to salvage -- is left out, because no trip to town takes it off.
    /// Counted, a character in heavy plate had no room before it picked
    /// up a thing, and left every corpse untouched. Everything still
    /// counts towards twice its capacity, where it has no defense left,
    /// and towards the server's wall at three times (`loot_room` has the
    /// arithmetic and the whole story).
    ///
    /// This is the working limit, not the server's. [`burden_room`](Self::burden_room) is
    /// the wall -- what the server will still accept -- and a character
    /// that hunts up to the wall cannot loot, cannot merge stacks and
    /// can barely walk.
    pub fn carry_room(&self, cfg: &Growth) -> u32 {
        let (now, capacity) = self.burden();
        let up_to = self
            .loot_profile()
            .map_or(crate::profile::Looting::default().carry_up_to, |p| {
                p.looting.carry_up_to
            });
        loot_room(now, self.loot_burden(cfg), capacity, up_to)
    }

    /// Carrying as much loot as it means to: time to go and sell.
    ///
    /// Not before its strength is known. With no capacity there is no
    /// room either, and a party told a member was laden the moment it
    /// logged in would turn round for town before a fight.
    ///
    /// Having had enough is not only having no room at all: it is having
    /// less room than the lightest thing the looting last left on a body
    /// still lying about (see `had_enough`).
    pub fn laden(&self, cfg: &Growth) -> bool {
        let (carried, capacity) = self.burden();
        if capacity == 0 {
            return false;
        }
        let up_to = self
            .loot_profile()
            .map_or(crate::profile::Looting::default().carry_up_to, |p| {
                p.looting.carry_up_to
            });
        let loot = self.loot_burden(cfg).min(carried);
        let room = loot_room(carried, loot, capacity, up_to);
        // Sold down to what it keeps: the room a trip to town would give.
        let sold = loot_room(carried - loot, 0, capacity, up_to);
        let objects = &self.world.objects;
        let left = self
            .autoplay
            .lightest_left_for_weight(|g| objects.contains_key(&g));
        had_enough(room, left, sold)
    }
}

#[cfg(test)]
mod tests;
