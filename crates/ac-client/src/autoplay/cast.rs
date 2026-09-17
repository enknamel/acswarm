use std::time::{Duration, Instant};

use super::Autoplay;
use crate::Client;

/// How long to wait on a cast the server never answers for. Casting is
/// paced by its answer, not by a clock; this only stops a character
/// waiting for ever on one that went astray.
const CAST_LOST: Duration = Duration::from_secs(6);

/// Why a cast is refused, in a few words for a status line.
pub fn cast_problem(check: &crate::magic::CastCheck) -> String {
    use crate::magic::CastCheck;
    match check {
        CastCheck::Ok => "fine".into(),
        CastCheck::NotKnown => "not known".into(),
        CastCheck::NoCaster => "no wand wielded".into(),
        CastCheck::NoTarget => "the target is gone".into(),
        CastCheck::MissingComponents(m) => format!("short of {} components", m.len()),
        CastCheck::NotEnoughMana { need, have } => format!("mana {have}/{need}"),
        CastCheck::TooHard { power, skill } => format!("power {power} over skill {skill}"),
    }
}

impl Autoplay {
    /// A spell of any kind went out less than a cast ago, so another
    /// sent now would queue behind it or be dropped.
    pub(crate) fn cast_in_flight(&self, now: Instant) -> bool {
        // The server says when a cast is finished, so that is what is
        // waited on -- not a guess at how long spells take. A heal sent
        // the moment the last one lands is the difference between
        // living and dying, and no fixed interval can be both quick
        // enough for that and slow enough never to have the next spell
        // dropped for arriving early.
        //
        // The clock that remains is a backstop, not the pacing: if the
        // server never answers at all, the character must not wait for
        // ever.
        match self.cast_sent {
            Some(t) => now.duration_since(t) < CAST_LOST,
            None => false,
        }
    }
}

impl Client {
    /// Cast `spell` and hold the next cast until the server answers for
    /// this one (see [`Autoplay::cast_in_flight`]). Every cast autoplay
    /// sends goes through here or sets the same clock itself: the heal
    /// once did neither, and a character at 16% health sent Heal Self
    /// every frame, forty times in under two seconds, until the first
    /// one went up.
    ///
    /// A cast the client declines to send earns no wait. Nothing is
    /// coming back to end one, so the whole six-second backstop would be
    /// spent standing still -- and now that the takes and the wields
    /// wait on this clock too, standing still over a corpse.
    pub(crate) fn cast_paced(&mut self, spell: u32, now: Instant) {
        if matches!(self.try_cast(spell), crate::magic::CastCheck::Ok) {
            self.autoplay.cast_sent = Some(now);
        }
    }

    /// Cast a spell the fight is throwing -- an attack, a vulnerability,
    /// an imperil -- paced like any other, and remember it as the
    /// fight's own so a stop can take it back (`Client::stop_fight_cast`).
    pub(crate) fn cast_fight_spell(&mut self, spell: u32, now: Instant) {
        self.cast_paced(spell, now);
        // Only when it actually went out: a cast the client declined
        // leaves `cast_sent` alone, and there is nothing to take back.
        self.autoplay.fight_cast = self.autoplay.cast_sent;
    }

    /// Stop the fight's own spell when the server is still winding it up.
    ///
    /// No message recalls a cast. What reaches one is a combat-mode
    /// change: ACE fails the cast outright for a character still casting
    /// (`HandleActionChangeCombatMode_Inner`, Player_Combat.cs:787),
    /// and `FailCast` fizzles the spell rather than landing it
    /// (Player_Magic.cs:1309). Past the windup the spell is already
    /// made and nothing takes it back, so this is a chance, not a
    /// promise.
    ///
    /// Only the fight's own spell. `cast_sent` is set by a heal, a
    /// healing kit and a counter as well, and a stop that fizzled those
    /// would take the heal a character is alive by.
    pub(crate) fn stop_fight_cast(&mut self) {
        let ours = self.autoplay.fight_cast.take();
        if ours.is_none() || ours != self.autoplay.cast_sent {
            return;
        }
        if !self.autoplay.cast_in_flight(Instant::now()) {
            return;
        }
        self.leave_combat();
    }
}
