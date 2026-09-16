use std::time::{Duration, Instant};

use super::judge::{called_to, judge_loot, takers_with, Taker};
use super::take::only_so_often;
#[cfg(doc)]
use super::take::refused_item;
use super::walk::{loot_wait, still_holding_at};
use crate::autoplay::{shut_line, Autoplay, Doing, ShutFor};
#[cfg(test)]
use crate::refusals::refused;
use crate::refusals::{self, Answer, OpenRefusal, Refusal};
use crate::Client;

/// How long to wait for a corpse to open before asking again, when it
/// is right under our feet. A corpse further off is given time for the
/// walk as well (see [`loot_wait`]).
///
/// Short, because the character now walks to the corpse itself (see
/// [`CORPSE_REACH`]) and asks from on top of it. The first ask is
/// nonetheless often ignored -- the server still has us moving -- so
/// what matters is how quickly the second one follows.
pub(crate) const LOOT_TIMEOUT: Duration = Duration::from_millis(2500);

/// How many times to ask before leaving a corpse alone. Asking is
/// cheap now that it is asked from arm's length.
const LOOT_TRIES: u32 = 3;

/// A wait that has doubled this far means the stack has been asked to
/// come apart half a dozen times and has not. The party gets on
/// without that hand-over.
pub(crate) const WILL_NOT_SPLIT: Duration = Duration::from_secs(8);

/// Whether an ask to open a corpse `away` metres off, sent on the tick
/// `asked`, came back the way the server answers for a thing it does not
/// have: a `UseDone` with no error (`done`), and not a word (`told`) since.
///
/// Blargerton, in the Holtburg Dungeon: ACE lets an object go once it
/// has been out of the character's sight for twenty-five seconds, and a
/// body that rots after that sends its delete only to the players who
/// still know it. The client kept such bodies for the session. It walked
/// back to one, asked, set it aside, came back to ask again, and held the
/// next fight for it meanwhile. A corpse that is there and within reach
/// opens, or says why it will not ("You do not yet have the right to
/// loot"); only one that is not there says nothing at all. From further
/// off the server walks the character over first, and a walk it cannot
/// finish ends just as quietly.
///
/// A quiet answer can also be an earlier ask's walk cut short by this
/// one, so one is not enough: every ask at a body has to come back so
/// (see `Autoplay::nothing_came_back`). Answers are stamped with the tick
/// they came in on and the ask with the tick it went out on, so an
/// answer that came in on that same tick was to an earlier ask.
pub(crate) fn answered_with_nothing(
    away: f32,
    asked: Instant,
    done: Option<(u32, Instant)>,
    told: Option<Instant>,
) -> bool {
    away <= CORPSE_REACH
        && done.is_some_and(|(err, at)| err == 0 && at > asked)
        && told.is_none_or(|at| at <= asked)
}

/// A corpse this close is looted, wherever it came from.
pub(crate) const LOOT_NEAR: f32 = 20.0;

/// How long after a killing blow its corpse is taken to be on the way.
/// The server makes the body a moment after the creature dies.
pub(super) const CORPSE_APPEARS: Duration = Duration::from_secs(3);

/// How long a monster's corpse lasts before it rots away. ACE gives an
/// unlooted corpse no timer at all until its first heartbeat, when it
/// takes the default of five minutes and counts down from there
/// (`WorldObject_Decay`), so this is the whole window there is.
pub(crate) const CORPSE_LIFE: Duration = Duration::from_secs(300);

/// How close to rotting a corpse has to be before it is worth breaking
/// off for. Inside this there is no second chance.
pub(crate) const CORPSE_URGENT: Duration = Duration::from_secs(75);

/// How close the character has to stand before a corpse will open.
///
/// The server will not hand over a container we are not standing at: a
/// use from across the room is answered by being told to walk there,
/// and it waits for us to arrive. A client that never walks waits for
/// ever, which is what every corpse that "did not open" turned out to
/// be. Two and a half metres is inside the use radius of everything
/// that leaves a body.
pub(super) const CORPSE_REACH: f32 = 2.5;

impl Autoplay {
    /// Start on a corpse: asked to open just now, with `allow` for it
    /// to do so. The loot rules start afresh with it.
    pub(crate) fn take_up_corpse(&mut self, guid: u32, now: Instant, allow: Duration) {
        self.fresh_loot_run();
        self.corpse = Some((guid, now, allow, 0));
        self.quiet_answers = 0;
    }

    /// Start the loot rules afresh, first counting what the body before
    /// came to. Every run ends here however its corpse was let go, so
    /// each body opened and each thing taken is counted once.
    fn fresh_loot_run(&mut self) {
        let run = std::mem::take(&mut self.loot_run);
        self.loot_tally.count(&run);
    }

    /// The ask to open the corpse in hand has had its wait and it has not
    /// opened: note whether that ask came back with nothing (`quiet`, see
    /// [`answered_with_nothing`]), and say whether every ask at this body
    /// so far has.
    fn nothing_came_back(&mut self, quiet: bool) -> bool {
        let Some((_, _, _, tries)) = self.corpse else {
            return false;
        };
        self.quiet_answers += u32::from(quiet);
        // One ask to start with, and one more for each try.
        self.quiet_answers > tries
    }

    /// Put down the corpse in hand, however it ended: emptied, given up
    /// on, not opening, or out of reach. Everything about emptying it
    /// goes with it. The loot rules' clock once outlived a corpse let go
    /// any way but a shut, and the next body, opened more than
    /// forty-five seconds after the first, was shut on its first step.
    pub(crate) fn let_go_of_corpse(&mut self) {
        self.corpse = None;
        self.appraising = false;
        self.fresh_loot_run();
    }

    /// Words from the server in answer to the ask to open the body in
    /// hand, `in_hand` being that body's name as the world has it: let
    /// the body go and leave it alone for as long as the words are
    /// worth. True if the words were about it.
    ///
    /// The server says why it will not open a body, and the client used
    /// to throw the words away and wait out [`loot_wait`] instead, then
    /// ask again three more times and be refused in the same words. Nine
    /// characters hunting one spot sent nine hundred and thirty such
    /// asks against a thousand refusals that had already arrived, each
    /// about a third of a second after the ask.
    ///
    /// Guarded as tightly as a quiet answer is (see
    /// [`answered_with_nothing`]): only words the server stamped after
    /// this ask went out -- `told` is when it last put anything in words
    /// -- and only words naming the body in hand. Words about a chest,
    /// or about a body one of the others is working, change nothing.
    pub(crate) fn corpse_refused(
        &mut self,
        named: &str,
        why: OpenRefusal,
        in_hand: &str,
        told: Option<Instant>,
        now: Instant,
    ) -> bool {
        let Some((guid, asked, ..)) = self.corpse else {
            return false;
        };
        if named != in_hand || told.is_none_or(|at| at <= asked) {
            return false;
        }
        let said = match why {
            OpenRefusal::InUse => "someone else has it open",
            OpenRefusal::NotYetOurs => "not ours yet",
            OpenRefusal::NeverOurs => "it is the killer's alone",
        };
        // The table's decision: a short doubling wait while someone has
        // it or it is not ours yet, and for good when it never will be
        // -- it still lies there, and every wait that runs out would be
        // another walk back to it.
        let answer = refusals::answer(&Refusal::Open { name: named, why });
        answer.hold(&mut self.shelved, guid, said, now);
        match (answer, self.shelved.waited(&guid)) {
            (Answer::Never, _) | (_, None) => {
                tracing::info!("autoplay: {in_hand} ({guid:#010x}): {said} -- leaving it");
            }
            (_, Some(wait)) => tracing::info!(
                "autoplay: {in_hand} ({guid:#010x}): {said} -- trying again in {} s",
                wait.as_secs().max(1)
            ),
        }
        self.let_go_of_corpse();
        true
    }

    /// [`Autoplay::corpse_refused`] from the server's own words, for
    /// the tests, which are written in them.
    #[cfg(test)]
    pub(crate) fn corpse_refused_in_words(
        &mut self,
        text: &str,
        in_hand: &str,
        told: Option<Instant>,
        now: Instant,
    ) -> bool {
        match refused(text) {
            Some(Refusal::Open { name, why }) => self.corpse_refused(name, why, in_hand, told, now),
            _ => false,
        }
    }
}

impl Client {
    /// The corpse `guid` holding `items`, asked to open at `asked`, and
    /// the character standing over it, as the loot rules see them.
    pub(crate) fn corpse_now(
        &mut self,
        guid: u32,
        items: &[u32],
        profile: &crate::profile::Profile,
        asked: Instant,
        now: Instant,
    ) -> ac_loot::Open {
        use crate::profile::Verdict as Judged;
        let away = match (
            self.player.as_ref().map(|p| p.world_position()),
            self.world.objects.get(&guid).and_then(|o| o.world_pos()),
        ) {
            (Some(me), Some(at)) => at.distance(me),
            _ => 0.0,
        };
        let name = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| "the corpse".to_string());
        // Our own body: everything on it is ours, and the wand and the
        // components are what the character needs to fight again.
        let mine = {
            let me = self.world.stats.name.to_lowercase();
            !me.is_empty() && name.to_lowercase() == format!("corpse of {me}")
        };
        let wielder = self.wielder();
        let who = self.world.stats.name.clone();
        let my_guid = self.world.player_guid.unwrap_or(0);
        // The fellows a thing here may be meant for instead of this
        // character (see `called_to`): nobody off its own body, or under
        // rules that mean nothing for anyone in particular.
        let fellows = if mine || !profile.sends_to_the_best() {
            Vec::new()
        } else {
            self.callable_to(guid)
        };
        let salvager = if fellows.is_empty() {
            None
        } else {
            self.best_salvager().map(|(_, g)| g)
        };
        // What has already been spoken for off this body counts towards
        // a cap (see `ac_loot::Claimed`).
        let mut claimed = ac_loot::corpse::Claimed::default();
        // Listed but not described yet: the server sends the items a
        // pass after the list. Dropped silently, they made a full body
        // read as empty (see `ac_loot::Open::arriving`).
        let mut arriving = Vec::new();
        // Where a take can go, as the rules must see it: a thing that
        // pours onto a carried pile needs no slot. Judged here from the
        // same room the take is sent from (`Client::how_to_take`), so
        // the two cannot disagree. The stacks carried are copied by
        // name, and only when something on the body could pour.
        let packs = self.packs();
        let carried = if items
            .iter()
            .filter_map(|g| self.world.objects.get(g))
            .any(|o| Self::loose(o).pours)
        {
            self.pack_stacks()
        } else {
            Vec::new()
        };
        let lying: Vec<ac_loot::Lying> = items
            .iter()
            .filter_map(|g| {
                let Some(stats) = self.stats_of(*g) else {
                    arriving.push(*g);
                    return None;
                };
                let needs_no_slot = self.world.objects.get(g).is_some_and(|o| {
                    let loose = Self::loose(o);
                    loose.pours
                        && matches!(
                            crate::room::how_to_take(&loose, &carried, &packs),
                            Some(crate::room::Take::Merge { .. })
                        )
                });
                // A kind the server has lately said cannot be had yet
                // is left alone for a while (see `loot_refused`).
                if self.refused_lately(stats.wcid, now) {
                    return None;
                }
                let held = self.already_carried(stats.wcid) + claimed.of(stats.wcid);
                // Judged once, here, whoever the body belongs to.
                let judged = judge_loot(
                    &stats,
                    self.appraisals.get(g),
                    Some(profile),
                    &wielder,
                    &who,
                    held,
                );
                let verdict = if mine {
                    // Our own body: everything on it comes back, but
                    // what each thing is for is still what the rules
                    // said (see `ac_loot::corpse::recovered`).
                    let took = match judged {
                        Judged::Decided(action, _) => Some(action),
                        Judged::NeedsId(_) | Judged::None => None,
                    };
                    ac_loot::Verdict::Take(ac_loot::corpse::recovered(took))
                } else {
                    match judged {
                        Judged::Decided(action, _) if action.takes() => {
                            // Meant for a fellow better at what the rule
                            // asks: left for that one, and the shut says
                            // so (see `Client::shut_for`).
                            let meant_for = if fellows.is_empty() {
                                None
                            } else {
                                let me = Taker {
                                    guid: my_guid,
                                    name: &who,
                                    sheet: &wielder,
                                    held,
                                };
                                let takers = takers_with(me, &fellows);
                                called_to(
                                    &stats,
                                    self.appraisals.get(g),
                                    profile,
                                    &takers,
                                    salvager,
                                )
                            };
                            if meant_for.is_some_and(|to| to != my_guid) {
                                ac_loot::Verdict::Leave
                            } else {
                                ac_loot::Verdict::Take(action)
                            }
                        }
                        Judged::Decided(_, _) => ac_loot::Verdict::Leave,
                        // Only worth asking about when asking is allowed
                        // and might answer.
                        Judged::NeedsId(_) if profile.looting.appraise => ac_loot::Verdict::MustAsk,
                        Judged::NeedsId(_) | Judged::None => ac_loot::Verdict::Leave,
                    }
                };
                if matches!(verdict, ac_loot::Verdict::Take(_)) {
                    claimed.take(stats.wcid, stats.stack.max(1));
                }
                Some(ac_loot::Lying {
                    guid: *g,
                    name: stats.name.clone(),
                    burden: stats.burden,
                    verdict,
                    needs_no_slot,
                })
            })
            .collect();
        ac_loot::Open {
            guid,
            name,
            away,
            open: true,
            items: lying,
            slots_free: self.free_space(),
            room_anywhere: self.room_anywhere(),
            keep_free: self.autoplay.config.team.restock.keep_slots,
            carry_room: self.carry_room(&self.autoplay.config.growth),
            may_ask: profile.looting.appraise,
            asking: self.appraise_inflight.iter().map(|(g, _)| *g).collect(),
            // What the server has turned down since the corpse was asked
            // to open (see `ac_loot::Open::refused`). One turned down
            // before is not held against this opening.
            refused: items
                .iter()
                .filter(|g| {
                    self.move_refused
                        .get(g)
                        .is_some_and(|(_, when)| *when >= asked)
                })
                .copied()
                .collect(),
            arriving,
        }
    }

    /// The server has refused something with `code`. When the reason is
    /// one that will not change today -- a thing that can only be had
    /// so many times a day -- the *kind* of thing is remembered, not
    /// the one on this corpse: the next corpse's copy would be refused
    /// for the same reason, and asking again is a round trip spent to
    /// be told no twice.
    ///
    /// `guid` is the item turned down, as the refusal named it or as the
    /// take in flight (see [`refused_item`]). This used to look up the
    /// front of a take queue that nothing filled any more, so no kind was
    /// ever remembered.
    pub(crate) fn loot_refused(&mut self, guid: u32, code: u32) {
        // Both are waits -- the first will certainly lift, and a solve
        // cap can be raised -- so neither is a `Refused`, which would
        // mean never.
        if !only_so_often(code) {
            return;
        }
        let Some(o) = self.world.objects.get(&guid) else {
            return;
        };
        let (wcid, name) = (o.weenie_class_id, o.name.clone());
        if wcid != 0 {
            let now = Instant::now();
            // The wait doubles with each refusal, so a cooldown of an
            // hour is picked up within the hour and a daily one costs a
            // handful of wasted asks a day -- nothing against missing
            // the thing for a day.
            self.autoplay.refused_kinds.note(
                wcid,
                &crate::did::Did::Blocked(crate::did::Because::server(code)),
                now,
            );
            tracing::info!("autoplay: {name} cannot be had yet; leaving its kind for a while");
        }
    }

    /// Whether this kind of thing is still inside the wait a refusal
    /// put on it.
    fn refused_lately(&self, wcid: u32, now: Instant) -> bool {
        self.autoplay.refused_kinds.held(&wcid, now)
    }

    /// Open the corpse of something we killed and take what is worth
    /// taking. True while looting.
    pub(crate) fn autoplay_loot(&mut self, now: Instant) -> bool {
        // No profile, no looting: it is what says what to take.
        let Some(profile) = self.loot_profile() else {
            return false;
        };
        // Already at one: wait for its contents, then empty it.
        if let Some((guid, since, allow, tries)) = self.autoplay.corpse {
            // The clock is on the opening, not on the emptying. Once
            // the corpse is open the character is taking from it an
            // item at a time, and asking the server to open it again
            // in the middle of that pulls the container out from under
            // the take in flight -- which is what "Source item not
            // found!" is.
            let opened = self
                .world
                .open_container
                .as_ref()
                .is_some_and(|(g, _)| *g == guid);
            // How long to keep at an open corpse is the loot rules' to say
            // (`ac_loot::run::KEEP_AT_IT`), counting only the time spent
            // at it, and they set a body that will not empty aside. This
            // used to give up first, forty-five seconds after the open was
            // asked for -- a Drudge pack fought off in between counted
            // too -- and wrote the body off for good with its loot still
            // on it, so the rules' own close could never be reached.
            if !opened && now.duration_since(since) > allow && self.autoplay.cast_in_flight(now) {
                // A spell went out meanwhile, and the use was most likely
                // turned away as too busy: that is not the corpse refusing.
                // Wait for the cast, and do not count it as a try.
                self.autoplay.corpse = Some((guid, now, allow, tries));
                return true;
            }
            if !opened && now.duration_since(since) > allow {
                // How this ask came back, from where the character stands
                // now: nothing walks it while nothing comes back.
                let me = self.player.as_ref().map(|p| p.world_position());
                let away = self
                    .world
                    .objects
                    .get(&guid)
                    .and_then(|o| o.world_pos())
                    .zip(me)
                    .map_or(f32::INFINITY, |(at, me)| at.distance(me));
                let quiet = answered_with_nothing(away, since, self.use_done, self.told);
                let nothing_there = self.autoplay.nothing_came_back(quiet);
                self.autoplay.let_go_of_corpse();
                self.stop_walking_to_loot();
                // Opening a corpse asks the server to walk us to it,
                // and indoors that walk goes round corners. Giving up
                // once and never asking again left loot on the floor,
                // so ask again before writing it off.
                if tries < LOOT_TRIES {
                    tracing::info!(
                        "autoplay: corpse {guid:#010x} did not open; asking again ({tries})"
                    );
                    self.interact(guid);
                    self.autoplay.corpse = Some((guid, now, allow, tries + 1));
                    return true;
                }
                // Every ask, from within reach, came back with nothing: the
                // server has no such body. It let it go while the
                // character was out of sight and never said so (see
                // [`answered_with_nothing`]). Setting it aside only brought
                // the character back to ask again, and held the next fight
                // for it meanwhile, so it is forgotten the way its delete
                // would have had it. One the server still has but cannot
                // walk the character to answers the same way, and is no
                // more to be had; should the server describe it again, it
                // is back.
                if nothing_there {
                    tracing::info!(
                        "autoplay: corpse {guid:#010x} is not there any more; forgetting it"
                    );
                    self.forget_kill_spot(guid);
                    self.world.forget(guid);
                    self.autoplay.corpse_seen.retain(|(g, _)| *g != guid);
                    return false;
                }
                // Not "done with": set aside. A corpse belongs to
                // whoever killed it until it has rotted a while, so one
                // that will not open now may well open later, and
                // writing it off for good leaves a boss's loot on the
                // floor. It is tried again while it is still there.
                tracing::info!(
                    "autoplay: corpse {guid:#010x} will not open yet; trying again later"
                );
                self.autoplay.shelved.note(
                    guid,
                    &crate::did::Did::blocked("it will not open yet"),
                    now,
                );
                return false;
            }
            let open = self.world.open_container.clone();
            let Some((open_guid, items)) = open else {
                self.autoplay.say(Doing::Looting, "opening a corpse");
                return true;
            };
            if open_guid != guid {
                return true;
            }
            // What to ask about, what to take, in what order and when
            // to stop is decided in `ac-loot`, which knows nothing of
            // sockets or packs: it is handed the body and the character
            // standing over it and answers with one thing to do. The
            // judging stays here, where the profile and the character's
            // own skills are (see `corpse_now`).
            let at = self.corpse_now(guid, &items, &profile, since, now);
            let next = self.autoplay.loot_run.step(&at, now);
            // Any walk back to this body is over the moment the rules
            // stop asking for one (see the branch below).
            if !matches!(
                next.act,
                Some(ac_loot::Act::Approach) | Some(ac_loot::Act::Open)
            ) {
                self.stop_walking_to_loot();
            }
            match next.act {
                Some(ac_loot::Act::Approach) | Some(ac_loot::Act::Open) => {
                    // Out of reach of a body it is holding: the
                    // character drifted while it worked, or a fight came
                    // to it and moved it.
                    //
                    // The body is still open on the server, whatever the
                    // distance. ACE shuts a container when its viewer
                    // stops viewing it, when that viewer uses another,
                    // or on the container's own reset -- and a corpse
                    // has no reset interval. Shutting it here therefore
                    // gave a body the character was already holding back
                    // to everyone else standing over it, and the walk
                    // back had to win it again: fifty times in one
                    // nine-character run.
                    //
                    // So walk back still holding it, and put it down
                    // only once the character has really gone
                    // ([`HOLD_ON_WITHIN`]) or the walk cannot arrive.
                    let spot = self.world.objects.get(&guid).and_then(|o| o.world_pos());
                    let setting_off = self.autoplay.walking_to.map(|w| w.guid) != Some(guid);
                    let going_back = still_holding_at(at.away)
                        && spot.is_some_and(|spot| {
                            self.walk_to_corpse(guid, &at.name, spot, at.away, now)
                        });
                    if going_back {
                        if setting_off {
                            tracing::info!(
                                "autoplay: corpse {guid:#010x} is {} m off; walking back to it \
                                 without shutting it",
                                at.away.round()
                            );
                        }
                        return true;
                    }
                    tracing::info!(
                        "autoplay: corpse {guid:#010x} is out of reach for good; letting it go"
                    );
                    self.close_container();
                    self.autoplay.let_go_of_corpse();
                    self.stop_walking_to_loot();
                    self.autoplay.say(Doing::Looting, next.saying);
                    return true;
                }
                Some(ac_loot::Act::Ask(ids)) => {
                    let n = ids.len();
                    self.appraise_many(ids);
                    self.autoplay.appraising = true;
                    self.autoplay.say(
                        Doing::Looting,
                        format!("looking over {n} of {} item(s)", items.len()),
                    );
                    return true;
                }
                Some(ac_loot::Act::Take(g)) => {
                    // What it is being taken for was settled when the
                    // lid came up; it is not asked again here, because
                    // asking again is how the two answers came to
                    // differ.
                    let took = at.items.iter().find(|i| i.guid == g).and_then(|i| i.took());
                    if let (Some(action), Some(stats)) = (took, self.stats_of(g)) {
                        self.autoplay.tag(&stats, action);
                    }
                    self.take(g);
                    self.autoplay.say(Doing::Looting, next.saying);
                    return true;
                }
                // Nothing to do yet: the rules are waiting on appraisals,
                // or on the things the corpse lists to be described. The
                // corpse stays open and in hand, and the rules' own clock
                // is still the limit. Reading this as done
                // closed a corpse the moment its items went off to be
                // appraised, marked it looted, and sent the character to the
                // next body -- which closed the first on the server and left
                // both, and everything worth taking on them, behind.
                None => {
                    self.autoplay.say(Doing::Looting, next.saying);
                    return true;
                }
                Some(ac_loot::Act::Close) => {
                    let done = matches!(next.did, crate::did::Did::Done);
                    // What it is for the others is judged on what is still
                    // on it, so before the lid goes down, and only for a
                    // body finished with (see `Autoplay::corpse_shut`).
                    let shut = if done {
                        self.shut_for(guid, &items, &profile)
                    } else {
                        ShutFor::default()
                    };
                    self.close_container();
                    // A body set aside is still this character's to come
                    // back to, however far off it fell.
                    if done {
                        self.forget_kill_spot(guid);
                        tracing::info!(
                            "{}",
                            shut_line(
                                &self.world.stats.name,
                                &at.name,
                                guid,
                                self.autoplay.loot_run.taken,
                                &self.names_of(&shut.done_for),
                                &self.names_of(&shut.left_for),
                                &self.names_of(&shut.stand_by),
                            )
                        );
                    } else {
                        tracing::info!(
                            "autoplay: corpse {guid:#010x} set aside; trying again later"
                        );
                    }
                    let left_for = self.names_of(&shut.left_for);
                    self.autoplay
                        .corpse_shut(guid, &next.did, next.left_for_weight, shut, now);
                    self.stop_walking_to_loot();
                    // Whom something is left for says why the others go on
                    // standing by a body this one has shut.
                    let saying = if left_for.is_empty() {
                        next.saying
                    } else {
                        format!("{}; left for {}", next.saying, left_for.join(", "))
                    };
                    self.autoplay.say(Doing::Looting, saying);
                    return true;
                }
            }
        }
        // Look for one nearby that we have not emptied.
        //
        // Mid-fight the loot waits: a corpse keeps for five minutes and
        // the thing hitting us does not. The exception is a corpse
        // about to rot, which is worth breaking off for because there
        // is no second chance at it.
        //
        // A cast in progress is different from a fight in progress: a
        // spell half thrown is wasted, and the corpse keeps for the
        // second it takes to finish. Whether the fight itself is worth
        // breaking off is not decided here any more -- the worth of
        // looting says that, and it rises as bodies age and pile up
        // (see `crate::steps`).
        // Only a cast at something still alive: the fight forgets a dead
        // target when it next runs, and while it waits for this very
        // body it does not run.
        let pressed = self
            .autoplay
            .casting_at()
            .and_then(|g| self.world.objects.get(&g))
            .is_some_and(|o| o.health.unwrap_or(1.0) > 0.0);
        let me = self.player.as_ref().map(|p| p.world_position());
        let Some(me) = me else { return false };
        // Kills go stale with the bodies they leave.
        self.autoplay
            .kill_spots
            .retain(|(_, t)| now.duration_since(*t) < CORPSE_LIFE);
        // A corpse set aside is tried again once its wait is up, and its
        // wait is remembered until the corpse is gone (see
        // `Autoplay::forget_corpses_gone`).
        let objects = &self.world.objects;
        self.autoplay
            .forget_corpses_gone(|g| objects.contains_key(&g));
        // A body stood by for a fellow that is not coming is this
        // character's to open again, and one emptied long ago is forgotten.
        self.autoplay.stop_standing_by(now);
        self.autoplay.looted.forget_old(now);
        // When each corpse turned up, so the ones running out can be
        // emptied first. Noted by the housekeeping rather than here (see
        // [`Client::autoplay_watch_the_ground`]): a body this character
        // is standing off from never reaches this step, and a body it
        // never noted is for ever newly fallen.
        let seen_at: std::collections::BTreeMap<u32, Instant> =
            self.autoplay.corpse_seen.iter().copied().collect();
        let room = self.room_for_loot();
        let corpse = self
            .world
            .objects
            .values()
            // Not emptied, not set aside, something there is room for,
            // close by on this floor or where one of this character's
            // kills fell, not one of the others' to open, and not another
            // player's remains. The one rule every part of this asks, so
            // what the looting goes to and what the next fight waits on
            // cannot come apart (see [`Client::corpse_for_us`]).
            .filter(|o| self.corpse_for_us(o, me, now, room))
            .filter_map(|o| Some((o.world_pos()?.distance(me), o)))
            .map(|(d, o)| (d, o.guid, o.name.clone()))
            .map(|(d, guid, name)| {
                let seen = seen_at.get(&guid).copied().unwrap_or(now);
                let left = CORPSE_LIFE.saturating_sub(now.duration_since(seen));
                (left, d, guid, name)
            })
            .min_by(|a, b| {
                // The one closest to rotting first, and among the ones
                // in no danger, the nearest.
                let urgent = |l: Duration| l <= CORPSE_URGENT;
                urgent(b.0)
                    .cmp(&urgent(a.0))
                    .then_with(|| a.1.total_cmp(&b.1))
            });
        let Some((left, away, guid, name)) = corpse else {
            // Nothing left to go to, so any walk that was going to one
            // is over. Left standing, that walk went on being said as a
            // claim on a body this character had just given up on, and
            // it kept the looting's own worth up (see `crate::steps`)
            // for a body it would never open.
            self.stop_walking_to_loot();
            // A body stood off from only because it is one of the others'
            // turn says so: one character opens it while the rest stand
            // by, and without a word that reads as a hang.
            if let Some((corpse, who)) = self.turn_of_another(me, now, room) {
                self.autoplay.note(format!("{corpse} is {who}'s turn"), now);
            }
            return false;
        };
        // Never mid-cast, unless the body will not be there when the
        // spell lands.
        if pressed && left > CORPSE_URGENT {
            return false;
        }
        // Breaking off a fight means breaking it off. The server marks
        // a character swinging or shooting as busy and refuses to open
        // anything for it, so letting the attack run while walking to a
        // corpse buys "You're too busy" and nothing else -- the target
        // goes first, then the combat stance.
        self.attack_target = None;
        self.autoplay.casting_at = None;
        self.autoplay.armed_for = None;
        if self.combat {
            self.toggle_combat();
        }
        // Stand over it first (see [`CORPSE_REACH`]).
        if away > CORPSE_REACH {
            if let Some(at) = self.world.objects.get(&guid).and_then(|o| o.world_pos()) {
                if !self.walk_to_corpse(guid, &name, at, away, now) {
                    // Through a floor or behind a wall, the walk never
                    // ends by itself, and it held the looting and the
                    // next fight until the body rotted. Set it aside and
                    // get on; it is tried again once the wait is up.
                    let walked = self
                        .autoplay
                        .walking_to
                        .map_or(0, |w| now.duration_since(w.started).as_secs());
                    tracing::info!(
                        "autoplay: cannot reach corpse {guid:#010x} after {walked} s; \
                         trying again later"
                    );
                    self.stop_walking_to_loot();
                    self.autoplay.set_aside_out_of_reach(guid, now);
                    self.autoplay
                        .say(Doing::Looting, format!("cannot reach {name}; leaving it"));
                    return false;
                }
                return true;
            }
        }
        // Not while a spell is on its way: the server turns the use away
        // as too busy.
        if self.autoplay.cast_in_flight(now) {
            return true;
        }
        self.stop_walking_to_loot();
        // Opening a corpse is "using something", which ends a journey.
        self.remember_journey();
        self.interact(guid);
        self.autoplay.take_up_corpse(guid, now, loot_wait(away));
        self.autoplay.say(Doing::Looting, format!("looting {name}"));
        true
    }
}

#[cfg(test)]
mod tests;
