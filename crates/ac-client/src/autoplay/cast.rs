use std::time::{Duration, Instant};

use super::Autoplay;
use crate::Client;

/// How long to wait on a cast the server never answers for. Casting is
/// paced by its answer, not by a clock; this only stops a character
/// waiting for ever on one that went astray.
const CAST_LOST: Duration = Duration::from_secs(6);

/// ACE's `YouCantDoThatWhileInTheAir`: the server's own physics has the character off the floor,
/// whatever this client sends (`Player.cs:49`, `Player_Magic.cs:105`).
pub(crate) const IN_THE_AIR: u32 = 0x04EB;

/// How long every cast waits once the server says the character is in the air: asked again at
/// once, it says so again (test: casts_wait_while_the_server_has_us_in_the_air).
const IN_THE_AIR_WAIT: Duration = Duration::from_secs(10);

/// A recall cast or portal use holds every act till the teleport, seen 3.6-6.0 s on: its UseDone
/// (Player_Magic.cs:960-965) comes before the 2 s wait for it (WorldObject_Magic.cs:1143, 1169).
pub(crate) const HOP_LANDS: Duration = Duration::from_secs(8);

/// The same for `/lifestone`: the server is busy through the LifestoneRecall animation, 15.2 s on
/// Coldeve (Player_Location.cs:179-183).
pub(crate) const FREE_HOP_LANDS: Duration = Duration::from_secs(20);

/// How long a step's wait for the busy slot holds back the acts that can wait (guess: a tick or two).
const HANDS_WANTED_FOR: Duration = Duration::from_secs(3);

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
    /// A spell of any kind went out less than a cast ago, or a take is in the air, so a cast sent
    /// now would queue behind it or be dropped.
    pub(crate) fn cast_in_flight(&self, now: Instant) -> bool {
        // A take keeps the server busy until it lands, and a cast sent meanwhile is turned away
        // busy (0x1D) with no answer of its own to free the slot.
        self.take_in_air.is_some() || self.casting(now)
    }

    /// A spell of any kind is still going up: what a take or a wield waits on (`server_busy`).
    pub(crate) fn casting(&self, now: Instant) -> bool {
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
        // A scroll being read: a cast sent then is turned away busy, 0x1D (Scroll.cs:58-82).
        if self.learnable.is_reading(now) {
            return true;
        }
        if self.in_the_air_until.is_some_and(|t| now < t) {
            return true;
        }
        if self.is_hopping(now) {
            return true;
        }
        match self.cast_sent {
            Some(t) => now.duration_since(t) < CAST_LOST,
            None => false,
        }
    }

    /// A recall, portal or gem sent to carry the body off has not landed or been refused.
    pub(crate) fn is_hopping(&self, now: Instant) -> bool {
        self.hop_until.is_some_and(|t| now < t)
    }

    /// A step waited on the busy slot lately (`Client::hands_busy`): top-ups, vitals, stones and
    /// refills let it have the slot next. Only survive, dodge and a lapsing buff go ahead of it.
    pub(crate) fn is_slot_wanted(&self, now: Instant) -> bool {
        self.hands_wanted
            .is_some_and(|t| now.saturating_duration_since(t) < HANDS_WANTED_FOR)
    }

    /// The server has answered for the cast in the air: the slot is free
    /// and the fight's own cast, if that is what it was, is spent.
    pub(crate) fn cast_answered(&mut self) {
        self.cast_sent = None;
        self.fight_cast = None;
    }
}

impl Client {
    /// A recall, portal or gem that carries the body off went out: it takes the slot, and every
    /// act waits until the body lands or `lasts` runs out (its UseDone comes before the teleport).
    pub(crate) fn hop_sent(&mut self, now: Instant, lasts: Duration) {
        self.autoplay.cast_sent = Some(now);
        self.autoplay.hop_until = Some(now + lasts);
    }

    /// Whether a cast or use is still being answered: the server turns an act sent now away busy
    /// (0x1D), and Shin Ro was used in a bane's recoil three times and never asked. The wait is
    /// marked, so that the acts that can wait give way ([`Autoplay::is_slot_wanted`]).
    pub(crate) fn hands_busy(&mut self, now: Instant) -> bool {
        if !self.autoplay.cast_in_flight(now) {
            return false;
        }
        self.autoplay.hands_wanted = Some(now);
        true
    }

    /// The body was carried off, or the hop will not carry it: the next act may go.
    pub(crate) fn hop_landed(&mut self) {
        self.autoplay.hop_until = None;
    }

    /// The server's answer `err` to an act while a hop is up: a refusal lands nothing, and a busy
    /// one to a recall casts it again once the slot is free, not at the give-up.
    pub(crate) fn hear_hop_refused(&mut self, err: u32, now: Instant) {
        if err == 0 || !self.autoplay.is_hopping(now) {
            return;
        }
        self.hop_landed();
        if err == crate::YOURE_TOO_BUSY {
            self.travel_recall_fizzled();
        }
    }

    /// The server's answer `err` to what was sent: "in the air" to a cast holds every cast
    /// [`IN_THE_AIR_WAIT`], said once a hold.
    pub(crate) fn hear_airborne(&mut self, err: u32, now: Instant) {
        if err != IN_THE_AIR || self.autoplay.cast_sent.is_none() {
            return;
        }
        let held = self.autoplay.in_the_air_until.is_some_and(|t| now < t);
        self.autoplay.in_the_air_until = Some(now + IN_THE_AIR_WAIT);
        if !held {
            let wait = IN_THE_AIR_WAIT.as_secs();
            self.autoplay.note(
                format!("the server has us in the air: no casts for {wait} s"),
                now,
            );
        }
    }

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
            // A debuff is not a projectile: its resist comes before the next cast goes out.
            self.autoplay.debuff_sent = None;
            self.autoplay.yield_sent = None;
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
