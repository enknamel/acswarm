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

    /// The server has answered for the cast in the air: the slot is free
    /// and the fight's own cast, if that is what it was, is spent.
    pub(crate) fn cast_answered(&mut self) {
        self.cast_sent = None;
        self.fight_cast = None;
    }
}

impl Client {
    /// Cast `spell` and hold the next cast until the server answers for
    /// this one (see [`Autoplay::cast_in_flight`]); true when it went
    /// out. Every cast autoplay sends goes through here or sets the same
    /// clock itself: the heal once did neither, and a character at 16%
    /// health sent Heal Self every frame, forty times in under two
    /// seconds, until the first one went up.
    ///
    /// A cast the client declines to send earns no wait. Nothing is
    /// coming back to end one, so the whole six-second backstop would be
    /// spent standing still -- and now that the takes and the wields
    /// wait on this clock too, standing still over a corpse.
    pub(crate) fn cast_paced(&mut self, spell: u32, now: Instant) -> bool {
        if matches!(self.try_cast(spell), crate::magic::CastCheck::Ok) {
            self.autoplay.cast_sent = Some(now);
            return true;
        }
        false
    }

    /// Cast the fight's own attack spell at `guid`, paced like any
    /// other, and remember it so a stop can take it back
    /// (`Client::stop_fight_cast`). True when it went out.
    pub(crate) fn cast_fight_spell(&mut self, spell: u32, guid: u32, now: Instant) -> bool {
        if !self.cast_paced(spell, now) {
            // A cast the client declined leaves `cast_sent` as it was --
            // a heal's, perhaps -- and there is nothing to take back.
            return false;
        }
        self.autoplay.fight_cast = Some((guid, now));
        true
    }

    /// Take the fight's own attack spell back while the server is still
    /// winding it up, the character having been told to stop fighting.
    /// True when the message went out.
    ///
    /// No message recalls a cast. What reaches one is a combat-mode
    /// change: `HandleActionChangeCombatMode_Inner` fails a cast still
    /// being wound up before it looks at the mode asked for
    /// (Player_Combat.cs:787-788), and `FailCast` fizzles the spell
    /// rather than landing it (Player_Magic.cs:1309). The mode sent is
    /// the one already held, which costs no animation at all
    /// (`resend_combat_mode`).
    ///
    /// A chance, not a promise, and worth spending only on a stop. Past
    /// the windup `MagicState.IsCasting` is false and nothing takes the
    /// spell back; before it, ACE holds the mode change on an
    /// ActionChain until `NextUseTime` (Player_Combat.cs:764-771), which
    /// `try_cast`'s own `enter_combat` sets for the first cast of a
    /// fight (Player_Combat.cs:890). The price when it does land is the
    /// spell itself and not its mana: `FailCast` runs
    /// `DoCastSpell_Inner` with the figure the windup already worked
    /// out, so the same mana and the same components go whether the
    /// spell lands or fizzles (Player_Magic.cs:857, :868, :917-918).
    ///
    /// Only the fight's own spell. `cast_sent` is set by a heal, a
    /// healing kit and a counter as well, and a stop that fizzled those
    /// would take the heal a character is alive by.
    pub(crate) fn stop_fight_cast(&mut self, now: Instant) -> bool {
        let Some((guid, sent)) = self.autoplay.fight_cast.take() else {
            return false;
        };
        // Another system's cast since ours means ours is already
        // answered for; the slot clock is the one to read.
        if self.autoplay.cast_sent != Some(sent) || !self.autoplay.cast_in_flight(now) {
            return false;
        }
        if !self.resend_combat_mode() {
            return false;
        }
        // No spell means no projectile: the shot-speed learner would
        // otherwise read the next missile beside us -- a fellow's -- as
        // this spell's, and the clock on the shot arriving is this
        // cast's (`note_fired`, `throw_at`).
        self.dodge.fired = None;
        if self.autoplay.thrown.is_some_and(|(g, _)| g == guid) {
            self.autoplay.thrown = None;
        }
        true
    }
}
