//! The team rules a character keeps while its player steers it: keep up
//! with the leader, and hit what the leader is hitting. Nothing here
//! takes the legs or the hands for an errand of its own -- no town run,
//! no ground to hunt, no experience spent, no weapon changed -- because
//! with autoplay off the player is the one playing.

use std::time::Instant;

use crate::autoplay::Doing;
use crate::{Client, Stance};

impl Client {
    /// The team rules that run with autoplay off. True when one of them
    /// acted, which is what keeps the status line it wrote (see
    /// [`Client::tick_autoplay`]).
    ///
    /// They run in the order the table gives the same three rows: catch
    /// up (120) before the fight (80), and keeping up (50) after it (see
    /// `crate::steps`). The hands before the legs within a tick, because
    /// an attack ends a journey under way ([`Client::interrupt_travel`])
    /// and the walk is laid down after it, not before.
    ///
    /// No switch of its own. `Team::follow` and `Team::focus_fire`, under
    /// `Team::enabled`, are what a player already sets to say "come with
    /// me" and "hit what I hit", and neither says anything about who is
    /// at the keyboard; a character that wants one without the other
    /// turns the other off, as it always could.
    pub(crate) fn autoplay_by_hand(&mut self, now: Instant) -> bool {
        if self.autoplay.config.enabled
            || self.world.player_guid.is_none()
            || !self.autoplay.config.team.enabled
        {
            return false;
        }
        // A leader that has got well away is caught up with and nothing
        // else: a swing sent after that is a swing at something standing
        // where the leader no longer is.
        if self.autoplay_follow(now, true) {
            return true;
        }
        let helping = self.autoplay_assist();
        let following = self.autoplay_follow(now, false);
        helping || following
    }

    /// Hit what the leader is hitting, and nothing else. True while on
    /// the leader's target.
    ///
    /// The narrow half of the fight rules, and narrow on purpose: a
    /// player at the keyboard wants a hand in the fight they picked, not
    /// a second player. So this never picks a target of its own, never
    /// walks to one, never changes what is in the character's hands and
    /// never makes ammunition; it sends the two messages a player would
    /// send -- into combat, swing -- and only at what the leader is
    /// already on.
    ///
    /// The reach is the pair [`Client::pick_target`] uses, so assisting
    /// takes on exactly what fighting alongside would: within
    /// `Fight::radius` of the character, since further is a walk and the
    /// walk is the player's, and within `Team::fight_radius` of the
    /// leader, since further would draw the party apart. The leader's
    /// own choice stands otherwise: the name lists are for picking, and
    /// nothing is being picked here.
    pub(crate) fn autoplay_assist(&mut self) -> bool {
        if !self.autoplay.config.team.focus_fire {
            return false;
        }
        let Some((leader, at, guid)) = self
            .team_leader()
            .and_then(|m| Some((m.name.clone(), m.world, m.target?)))
        else {
            return false;
        };
        let cfg = self.autoplay.config.fight.clone();
        // Alive, and not something the character is walking past on its
        // leader's road (see `Client::joins_the_team_on`).
        if !self.joins_the_team_on(guid, &cfg) {
            return false;
        }
        let (Some(me), Some(target)) = (
            self.my_position(),
            self.world.objects.get(&guid).and_then(|o| o.world_pos()),
        ) else {
            return false;
        };
        let radius = self.autoplay.config.team.fight_radius.max(1.0);
        if target.distance(me) > cfg.radius || target.distance(at) > radius {
            return false;
        }
        let name = self.world.name_or_hex(guid);
        let say = format!("helping {leader} on {name}");
        // Already on it: say so, and send nothing. Said every tick the
        // fight lasts, so the line stands instead of being cleared the
        // frame after the swing went out.
        if self.attack_target == Some(guid) {
            self.autoplay.say(Doing::Fighting, say);
            return true;
        }
        // A caster's attack is a cast, and which spell to cast is the
        // player's own choice (`Client::attack` sends nothing with a
        // wand in hand). Entering combat would only take the character
        // out of magic stance.
        if self.combat_stance() == Stance::Magic {
            return false;
        }
        self.enter_combat();
        self.attack(guid);
        self.autoplay.say(Doing::Fighting, say);
        true
    }
}

#[cfg(test)]
mod tests;
