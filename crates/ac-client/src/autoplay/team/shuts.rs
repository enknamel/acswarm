use std::time::{Duration, Instant};

use ac_agent::recent::Recent;

use super::turns::Shut;
use super::view::Mate;
#[cfg(doc)]
use super::view::TeamView;
use crate::autoplay::loot::judge::{takers_with, would_take, Fellow};
#[cfg(doc)]
use crate::autoplay::CORPSE_LIFE;
use crate::autoplay::{called_to, Autoplay, Left, Taker};
use crate::Client;

/// How long a body shut as emptied goes on being said on the board (see
/// [`Autoplay::shuts_to_say`]). A mate takes the word in for good the
/// first time it hears it, a board round after it is said
/// (`ac_plugin::team::SAY_EVERY`, half a second), so this only has to
/// outlast a mate that missed a few rounds: one quiet for six seconds is
/// dropped from the roster anyway.
const SHUT_SAID_FOR: Duration = Duration::from_secs(20);

/// How many bodies shut as emptied are said at once, the newest. A
/// character in a party of nine shuts one a minute or so.
const SHUTS_SAID: usize = 16;

/// How long a body opened first counts as a turn at the bodies (see
/// [`TeamView::opens_first`]). Long enough to hold a round of turns in a
/// party of nine killing eight a minute; short enough that one just come
/// to the ground, with no turns yet, is not dealt every body until it has
/// caught up with the rest.
pub(super) const DEAL_WINDOW: Duration = Duration::from_secs(120);

/// How many shuts heard of are remembered (see [`Autoplay::has_shut`]),
/// the oldest bodies forgotten first. The bodies are forgotten as they go
/// (see [`Autoplay::forget_corpses_gone`]); this only bounds a character
/// that never looks.
const SHUT_BY_KEPT: usize = 512;

/// How many fellows done with a body are remembered (see
/// [`Autoplay::done_with`]): a party of nine, each done with as many
/// bodies as there are shuts remembered.
const DONE_WITH_KEPT: usize = SHUT_BY_KEPT * 9;

/// How long a character stands by a body for the fellow something on it
/// was left for (see [`StandBy`]), at most. That fellow may be fighting,
/// or have other bodies left for it first; one that has not come by then
/// is not coming, and what was left for it is taken by whoever else wants
/// it, with the whole of a body's five minutes still to spare.
const STAND_BY_FOR: Duration = Duration::from_secs(60);

/// How long a body emptied is remembered as emptied (see
/// [`Autoplay::looted`]). No monster's body lasts anywhere near an hour
/// ([`CORPSE_LIFE`]), and ACE hands a released guid out again once it has
/// been free for six hours (`GuidManager`, `recycleTime`): remembered for
/// the whole of a long session, a new body that came with an old one's
/// guid was taken as emptied and never opened.
const EMPTIED_KEPT: Duration = Duration::from_secs(60 * 60);

/// What a body shut as emptied is for this character and for the fellows
/// judged at the shut (see [`judge_shut`], [`Shut`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShutFor {
    /// Where the body lies, which whoever stands by for a fellow measures
    /// that fellow's reach from.
    pub at: glam::Vec3,
    pub done_for: Vec<u32>,
    pub stand_by: Vec<u32>,
    pub left_for: Vec<u32>,
    /// This character left on it something its own rules would take, for
    /// one of `left_for`: it stands by for them too, rather than writing
    /// the body off.
    pub waits: bool,
}

/// What a body this character is shutting as emptied is for it and for
/// each of the `judged` (see [`Shut`]), judged on the things `lying` on it
/// (`None` for one not described yet) by the rules the party shares, read
/// with what each fellow said about itself.
///
/// A thing is left for one fellow in particular when the rules mean it for
/// the best at what they ask (see [`called_to`]) among this character
/// (`me`) and the `sendable`, the judged that things may be sent to. Then:
/// - a fellow that would take nothing still on the body is done with it,
///   whatever was left for whom;
/// - one that would take only things left for others stands by for them;
/// - the rest open it by their own lights, the ones things were left for
///   among them.
///
/// Each fellow is judged as carrying none of anything, its pack not being
/// known here, so a rule that stops at a count takes: at worst that costs
/// it an open that finds nothing.
///
/// A fellow that would take a thing left for another is never told the
/// body is done. Told it was, it wrote the body off for good; and when the
/// one the thing was left for carried its fill already, or died, or
/// followed its leader away, the thing lay there until the body rotted.
pub fn judge_shut(
    lying: &[Option<Left>],
    profile: &crate::profile::Profile,
    me: Taker,
    judged: &[Fellow],
    sendable: &[Fellow],
    salvager: Option<u32>,
) -> ShutFor {
    let meant: Vec<Option<u32>> = lying
        .iter()
        .map(|thing| {
            let left = thing.as_ref()?;
            if sendable.is_empty() {
                return None;
            }
            let me = Taker {
                held: left.held,
                ..me
            };
            let takers = takers_with(me, sendable);
            called_to(&left.stats, left.id, profile, &takers, salvager).filter(|to| *to != me.guid)
        })
        .collect();
    let mut shut = ShutFor::default();
    for to in meant.iter().flatten() {
        if !shut.left_for.contains(to) {
            shut.left_for.push(*to);
        }
    }
    for (guid, name, sheet) in judged {
        let (mut takes, mut takes_its_own) = (false, false);
        for (thing, to) in lying.iter().zip(&meant) {
            let wanted = thing
                .as_ref()
                .is_none_or(|left| would_take(left, profile, sheet, name, 0));
            if wanted {
                takes = true;
                takes_its_own |= to.is_none_or(|to| to == *guid);
            }
        }
        if !takes {
            shut.done_for.push(*guid);
        } else if !takes_its_own {
            shut.stand_by.push(*guid);
        }
    }
    shut.waits = lying.iter().zip(&meant).any(|(thing, to)| {
        to.is_some()
            && thing
                .as_ref()
                .is_some_and(|left| would_take(left, profile, me.sheet, me.name, left.held))
    });
    shut
}

/// What the log says when the rules shut a body as emptied: who shut it
/// (`who`), how many things it took, which of the others it is done for,
/// which it left things for, and which stand by for those.
///
/// The log of a nine-character run named nobody: "use Corpse of Biaka"
/// five times inside 140 milliseconds, and no telling who opened it
/// first or who came back to it for nothing.
pub fn shut_line(
    who: &str,
    corpse: &str,
    guid: u32,
    took: u32,
    done_for: &[String],
    left_for: &[String],
    stand_by: &[String],
) -> String {
    let mut line = format!("autoplay: {who} shut {corpse} ({guid:#010x}), took {took}");
    if !done_for.is_empty() {
        line += &format!("; done for {}", done_for.join(", "));
    }
    if !left_for.is_empty() {
        line += &format!("; left for {}", left_for.join(", "));
    }
    if !stand_by.is_empty() {
        line += &format!("; {} standing by", stand_by.join(", "));
    }
    line
}

/// What the log says when a body one of the others shut (`by`) is taken
/// as emptied for this character (`me`), which then leaves it alone.
pub fn taken_in_line(me: &str, corpse: &str, guid: u32, by: &str) -> String {
    format!("autoplay: {me}: {corpse} ({guid:#010x}) done for me by {by}")
}

/// What the log says when this character (`me`) stands by a body one of
/// the others shut (`by`) for the fellows something on it was left for
/// (`on`).
pub fn standing_by_line(me: &str, corpse: &str, guid: u32, by: &str, on: &[String]) -> String {
    format!(
        "autoplay: {me}: {corpse} ({guid:#010x}) left for {} by {by}; standing by",
        on.join(", ")
    )
}

/// A body this character leaves to the fellows something on it was left
/// for, before it takes what it wants off it itself (see
/// `Autoplay::stands_by`).
#[derive(Clone, Debug, PartialEq)]
pub struct StandBy {
    /// Where the body lies, which those fellows' reach is measured from.
    pub at: glam::Vec3,
    /// The fellows things on it were left for ([`Shut::left_for`]).
    pub on: Vec<u32>,
    /// Since when.
    pub since: Instant,
}

/// The bodies emptied, and when each was written off (see
/// [`Autoplay::looted`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct Emptied(Recent<u32>);

impl Emptied {
    /// Whether the body `guid` has been emptied.
    pub(crate) fn contains(&self, guid: &u32) -> bool {
        self.0.since(guid).is_some()
    }

    /// Write the body `guid` off as emptied at `now`, once.
    pub(crate) fn push(&mut self, guid: u32, now: Instant) {
        if !self.contains(&guid) {
            self.0.mark(guid, now);
        }
    }

    /// Forget the bodies written off [`EMPTIED_KEPT`] or longer before
    /// `now`, long rotted, before their guids can come back on others.
    pub(crate) fn forget_old(&mut self, now: Instant) {
        self.0.expire(now, EMPTIED_KEPT);
    }

    /// How many bodies are remembered as emptied.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What a character took in from a shut one of the others said (see
/// `Autoplay::take_in_shuts`), for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TakenIn {
    /// The body `body` is done for this character, by the word of `by`.
    Done { body: u32, by: String },
    /// This character stands by the body `body`, by the word of `by`, for
    /// the fellows `on`.
    StandBy { body: u32, by: String, on: Vec<u32> },
}

impl Autoplay {
    /// What becomes of a corpse the loot rules have shut, by what they
    /// said about it. Only one they finished with is done with. One
    /// that would not give up its contents is set aside and tried again
    /// later, as the rules meant: marking every shut corpse looted wrote
    /// those off for good, with their loot still on them.
    ///
    /// One shut for want of room to carry the rest (`left_for_weight`, the
    /// lightest thing left on it) waits on room rather than on a clock
    /// (see `Autoplay::left_for_weight`).
    ///
    /// One finished with says on the board what it is for the others
    /// (`shut`, see `Client::shut_for`). Only one finished with: a body set
    /// aside, or left for its weight, still has on it what somebody wanted.
    ///
    /// And one finished with is done with here, unless something on it
    /// that this character's rules would take was left for a fellow better
    /// at what they ask: then it stands by for that fellow (see
    /// [`Autoplay::stands_by`]), and takes the thing itself if the fellow
    /// never comes.
    pub(crate) fn corpse_shut(
        &mut self,
        guid: u32,
        did: &crate::did::Did,
        left_for_weight: Option<u32>,
        shut: ShutFor,
        now: Instant,
    ) {
        match (did, left_for_weight) {
            (crate::did::Did::Done, _) => {
                self.left_for_weight.remove(&guid);
                if shut.waits && !shut.left_for.is_empty() {
                    self.standing_by.insert(
                        guid,
                        StandBy {
                            at: shut.at,
                            on: shut.left_for.clone(),
                            since: now,
                        },
                    );
                } else {
                    self.standing_by.remove(&guid);
                    self.looted.push(guid, now);
                }
                // A turn at the bodies, unless one of the others shut it
                // first, or this one stood by it for another: then it was
                // left for this one, not dealt to it.
                let shut_before = self
                    .shut_by
                    .range((guid, 0, 0)..=(guid, u32::MAX, u32::MAX))
                    .next()
                    .is_some()
                    || self
                        .done_with
                        .range((guid, 0)..=(guid, u32::MAX))
                        .next()
                        .is_some();
                if !shut_before {
                    self.first_opens
                        .retain(|t| now.saturating_duration_since(*t) < DEAL_WINDOW);
                    self.first_opens.push(now);
                }
                self.say_shut(guid, shut, now);
            }
            (_, Some(burden)) => {
                self.left_for_weight.insert(guid, burden);
            }
            (_, None) => {
                self.left_for_weight.remove(&guid);
                self.shelved.note(guid, did, now);
            }
        }
        self.let_go_of_corpse();
    }

    /// Say on the board what the body `guid`, shut as emptied, is for the
    /// others (see [`Autoplay::shuts_to_say`]). Every one, whomever it is
    /// done for: one said only when it was done for somebody left the rest
    /// not knowing who had shut it, so going back to it counted as a turn
    /// and things on it were left for the one that had shut it already. A
    /// body shut again is said afresh, not twice. Off the team there is
    /// nobody to say it to.
    fn say_shut(&mut self, guid: u32, shut: ShutFor, now: Instant) {
        self.shut_lately.retain(|(said, when)| {
            said.body != guid && now.saturating_duration_since(*when) < SHUT_SAID_FOR
        });
        if !self.config.team.enabled {
            return;
        }
        let n = self.shuts_made;
        self.shuts_made = n.wrapping_add(1);
        self.shut_lately.push((
            Shut {
                body: guid,
                n,
                done_for: shut.done_for,
                stand_by: shut.stand_by,
                left_for: shut.left_for,
            },
            now,
        ));
        let over = self.shut_lately.len().saturating_sub(SHUTS_SAID);
        self.shut_lately.drain(..over);
    }

    /// The bodies this character shut lately as emptied, and what each is
    /// for the others: what it says about itself on the board (see
    /// [`Mate::shut`]). The newest `SHUTS_SAID`, each for
    /// `SHUT_SAID_FOR`.
    pub fn shuts_to_say(&self, now: Instant) -> Vec<Shut> {
        self.shut_lately
            .iter()
            .filter(|(_, when)| now.saturating_duration_since(*when) < SHUT_SAID_FOR)
            .map(|(said, _)| said.clone())
            .collect()
    }

    /// Take in what the others said about the bodies they shut, each shut
    /// once (see [`Shut`]), `me` being this character's player guid and
    /// `at_of` where a body lies, if it is in sight. Returns what was taken
    /// in, for the log.
    ///
    /// - Whoever shut it is noted: going back to it is no turn, and nothing
    ///   on it is left for that one (see [`Autoplay::may_be_sent`]). So are
    ///   the fellows it is done for.
    /// - A body done for this character is written off for good.
    /// - One this character is to stand by is left to the fellows things
    ///   on it were left for, for as long as one of them could still come
    ///   (see [`Autoplay::stands_by`]).
    /// - Otherwise the body is this character's to open by its own lights,
    ///   whatever an older word said: the newest shut of a body says what
    ///   is left on it.
    ///
    /// Nine characters opened 83 bodies 697 times, and 42% of those opens
    /// took nothing: the others opened a body one of them had emptied, to
    /// find it so. Written into `looted`, the word reaches everything that
    /// asks whether a body is done with (see
    /// [`Autoplay::corpse_waiting`]), and it outlasts the row it came on,
    /// which goes once its mate has been quiet a while.
    ///
    /// Off the team nothing is taken in: the rules there see nobody. With
    /// its own rules off, a character only notes who shut what: a body
    /// written off then, by a word about a character played by hand, was
    /// walked past once the player turned the rules back on beside it.
    pub(crate) fn take_in_shuts(
        &mut self,
        me: u32,
        now: Instant,
        at_of: impl Fn(u32) -> Option<glam::Vec3>,
    ) -> Vec<TakenIn> {
        if !self.config.team.enabled || me == 0 {
            return Vec::new();
        }
        let said: Vec<(u32, String, glam::Vec3, Shut)> = self
            .team
            .mates
            .iter()
            .flat_map(|m| {
                m.shut
                    .iter()
                    .map(|s| (m.guid, m.name.clone(), m.world, s.clone()))
            })
            .collect();
        let mut heard = Vec::new();
        for (by, name, where_it_was, shut) in said {
            if !self.shut_by.insert((shut.body, by, shut.n)) {
                continue;
            }
            self.done_with
                .extend(shut.done_for.iter().map(|g| (shut.body, *g)));
            if !self.config.enabled || self.looted.contains(&shut.body) {
                continue;
            }
            if shut.done_for.contains(&me) {
                // As its own emptying would: nothing waits on room for it.
                self.standing_by.remove(&shut.body);
                self.left_for_weight.remove(&shut.body);
                self.looted.push(shut.body, now);
                heard.push(TakenIn::Done {
                    body: shut.body,
                    by: name,
                });
            } else if shut.stand_by.contains(&me) {
                // The shutter stood over it a moment ago.
                let at = at_of(shut.body).unwrap_or(where_it_was);
                self.standing_by.insert(
                    shut.body,
                    StandBy {
                        at,
                        on: shut.left_for.clone(),
                        since: now,
                    },
                );
                heard.push(TakenIn::StandBy {
                    body: shut.body,
                    by: name,
                    on: shut.left_for,
                });
            } else {
                self.standing_by.remove(&shut.body);
            }
        }
        // The server hands out rising ids: the lowest body is oldest.
        while self.shut_by.len() > SHUT_BY_KEPT {
            self.shut_by.pop_first();
        }
        while self.done_with.len() > DONE_WITH_KEPT {
            self.done_with.pop_first();
        }
        heard
    }

    /// Whether this character leaves the body `guid` to the fellows
    /// something on it was left for (see [`StandBy`]): for at most
    /// [`STAND_BY_FOR`], and only while one of them could still come for
    /// it (see [`Mate::could_come_for`]) and has not shut it since. One
    /// dead, gone from the board, out of reach, without room or not
    /// looting is not waited on.
    pub(crate) fn stands_by(&self, guid: u32, now: Instant) -> bool {
        self.standing_by.get(&guid).is_some_and(|s| {
            now.saturating_duration_since(s.since) < STAND_BY_FOR
                && s.on.iter().any(|g| {
                    !self.has_shut(guid, *g)
                        && self
                            .team
                            .mates
                            .iter()
                            .any(|m| m.guid == *g && m.could_come_for(s.at))
                })
        })
    }

    /// Stop standing by the bodies no fellow is coming for any more (see
    /// [`Autoplay::stands_by`]). Each is this character's to open again,
    /// and nothing on it is left for the fellows it stood by for: one that
    /// looked able and never came would be left it again, and stood by
    /// for again, until the body rotted.
    pub(crate) fn stop_standing_by(&mut self, now: Instant) {
        let over: Vec<u32> = self
            .standing_by
            .keys()
            .copied()
            .filter(|g| !self.stands_by(*g, now))
            .collect();
        for body in over {
            if let Some(stood) = self.standing_by.remove(&body) {
                self.done_with.extend(stood.on.iter().map(|g| (body, *g)));
            }
        }
    }

    /// Whether the mate `m` is judged at the shut of the body `corpse`,
    /// lying at `at` (see [`judge_shut`]): it could come for something on
    /// it ([`Mate::could_come_for`]) and is not done with it already. One
    /// played by hand, dead, far off, without room or not looting is told
    /// nothing, and opens the body or not by its own lights, as before;
    /// nor is anything said to be left for one that is not coming.
    pub(crate) fn may_judge(&self, corpse: u32, m: &Mate, at: glam::Vec3) -> bool {
        m.could_come_for(at) && !self.done_with.contains(&(corpse, m.guid))
    }

    /// Whether something on the body `corpse`, lying at `at`, may be left
    /// for the mate `m` (see [`called_to`]): it is judged at the body's
    /// shut ([`Autoplay::may_judge`]) and has not shut it already. Nothing
    /// goes to one that has shut it, was told it is done with it, or was
    /// stood by for and never came: none of those opens it again for it.
    pub(crate) fn may_be_sent(&self, corpse: u32, m: &Mate, at: glam::Vec3) -> bool {
        self.may_judge(corpse, m, at) && !self.has_shut(corpse, m.guid)
    }

    /// Whether any of the others has said it shut the body `corpse`: one
    /// the leader's plan does not deal, what is still on it being routed
    /// by the shut (see [`Shut`]).
    pub(crate) fn shut_by_anyone(&self, corpse: u32) -> bool {
        self.shut_by
            .range((corpse, 0, 0)..=(corpse, u32::MAX, u32::MAX))
            .next()
            .is_some()
    }

    /// Whether the mate `who` has said it shut the body `corpse` (see
    /// [`Autoplay::take_in_shuts`]).
    pub(crate) fn has_shut(&self, corpse: u32, who: u32) -> bool {
        self.shut_by
            .range((corpse, who, 0)..=(corpse, who, u32::MAX))
            .next()
            .is_some()
    }
}

impl Client {
    /// Where the body `corpse` lies, the fellows judged at its shut (see
    /// [`Autoplay::may_judge`]), and those of them things on it may be left
    /// for (see [`Autoplay::may_be_sent`]), each as its row says it. `None`
    /// for a body with no place in the world to measure from.
    fn fellows_at(&self, corpse: u32) -> Option<(glam::Vec3, Vec<Fellow>, Vec<Fellow>)> {
        let at = self
            .world
            .objects
            .get(&corpse)
            .and_then(|o| o.world_pos())?;
        let fellows = self.fellows();
        let (mut judged, mut sendable) = (Vec::new(), Vec::new());
        for m in self.autoplay.team.judged_at_a_shut(fellows.as_deref()) {
            if !self.autoplay.may_judge(corpse, m, at) {
                continue;
            }
            let fellow = (m.guid, m.name.clone(), m.wielder());
            if self.autoplay.may_be_sent(corpse, m, at) {
                sendable.push(fellow.clone());
            }
            judged.push(fellow);
        }
        Some((at, judged, sendable))
    }

    /// The fellows a thing on the body `corpse` could be meant for instead
    /// of this character (see [`called_to`], [`Client::fellows_at`]).
    pub(crate) fn callable_to(&self, corpse: u32) -> Vec<Fellow> {
        self.fellows_at(corpse)
            .map(|(_, _, sendable)| sendable)
            .unwrap_or_default()
    }

    /// What the body `corpse`, being shut as emptied with `items` still on
    /// it, is for this character and for each fellow (see [`judge_shut`]):
    /// judged by this character's rules, which the party shares, with what
    /// each fellow said about itself. Under rules that mean nothing for
    /// anyone in particular, nothing is left for anyone in particular.
    pub(crate) fn shut_for(
        &self,
        corpse: u32,
        items: &[u32],
        profile: &crate::profile::Profile,
    ) -> ShutFor {
        let Some((at, judged, sendable)) = self.fellows_at(corpse) else {
            return ShutFor::default();
        };
        let sendable = if profile.sends_to_the_best() {
            sendable
        } else {
            Vec::new()
        };
        let lying: Vec<Option<Left>> = items
            .iter()
            .map(|g| {
                self.stats_of(*g).map(|stats| Left {
                    held: self.already_carried(stats.wcid),
                    id: self.appraisals.get(g),
                    stats,
                })
            })
            .collect();
        let sheet = self.wielder();
        let me = Taker {
            guid: self.world.player_guid.unwrap_or(0),
            name: &self.world.stats.name,
            sheet: &sheet,
            held: 0,
        };
        let salvager = if sendable.is_empty() {
            None
        } else {
            self.best_salvager().map(|(_, g)| g)
        };
        ShutFor {
            at,
            ..judge_shut(&lying, profile, me, &judged, &sendable, salvager)
        }
    }

    /// The names of the fellows `guids`, as their rows have them.
    pub(crate) fn names_of(&self, guids: &[u32]) -> Vec<String> {
        guids
            .iter()
            .filter_map(|g| self.autoplay.team.mates.iter().find(|m| m.guid == *g))
            .map(|m| m.name.clone())
            .collect()
    }

    /// Take in the bodies the others said they emptied for this
    /// character, and say so in the log (see
    /// `Autoplay::take_in_shuts`). The team plugin calls this as it
    /// hands the rules what the others said.
    pub fn take_in_shuts(&mut self) {
        let me = self.world.player_guid.unwrap_or(0);
        let objects = &self.world.objects;
        let heard = self.autoplay.take_in_shuts(me, Instant::now(), |g| {
            objects.get(&g).and_then(|o| o.world_pos())
        });
        let who = &self.world.stats.name;
        let corpse = |g: u32| objects.get(&g).map_or("a corpse", |o| o.name.as_str());
        for taken in heard {
            match taken {
                TakenIn::Done { body, by } => {
                    tracing::info!("{}", taken_in_line(who, corpse(body), body, &by));
                }
                TakenIn::StandBy { body, by, on } => tracing::info!(
                    "{}",
                    standing_by_line(who, corpse(body), body, &by, &self.names_of(&on))
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests;
