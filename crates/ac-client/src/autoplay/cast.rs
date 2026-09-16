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
    /// The id of a known spell whose name starts with `name`, preferring
    /// the highest level learnt (the last in the spellbook order).
    pub fn spell_by_name(&self, name: &str) -> Option<u32> {
        let want = name.trim().to_lowercase();
        if want.is_empty() {
            return None;
        }
        let table = self.assets.spell_table().ok();
        // Of the family, the strongest that can be cast right now: a
        // name like "Heal Self" means the best Heal Self we can manage,
        // not the best in the book. Failing any castable, the strongest
        // known, so the reason it cannot be cast can be reported.
        let mut best_castable: Option<(u32, u32)> = None;
        let mut best_known: Option<(u32, u32)> = None;
        for id in &self.world.stats.spells {
            let sp = table.as_ref().and_then(|t| t.get(*id));
            let full = sp
                .map(|s| s.name.clone())
                .or_else(|| self.known_spells.get(id).cloned())
                .unwrap_or_default()
                .to_lowercase();
            if !full.starts_with(&want) && !full.contains(&want) {
                continue;
            }
            // Power orders a family; a spell the table lacks ranks by id.
            let power = sp.map(|s| s.power).unwrap_or(*id);
            let castable = matches!(
                self.can_cast(*id),
                crate::magic::CastCheck::Ok | crate::magic::CastCheck::NoCaster
            );
            if castable && best_castable.is_none_or(|(_, p)| power > p) {
                best_castable = Some((*id, power));
            }
            if best_known.is_none_or(|(_, p)| power > p) {
                best_known = Some((*id, power));
            }
        }
        best_castable.or(best_known).map(|(id, _)| id)
    }

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
}
