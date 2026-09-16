//! A headless game session: the connection, the world the server describes,
//! our character, and the gameplay commands a UI or a script can issue.
//! Nothing here renders; several `Client`s can live in one process.

pub mod academy;
pub mod advance;
pub mod aim;
pub mod augmentations;
pub mod autoplay;
mod body;
pub use body::movement::Follow;
pub(crate) use body::movement::SAME_FLOOR;
pub use body::player_tick::PlayerFrame;
#[cfg(doc)]
use body::server_walk::server_walk_over;
pub(crate) use body::server_walk::{
    answers_a_pour, answers_walk, attack_ended_walk, attack_unanswered, heard_server_walk,
    ServerWalk,
};
pub use body::standing::Standing;
pub mod buffs;
pub mod creation;
pub mod daytime;
pub mod dodge;
pub mod emotes;
pub mod explore;
// The vocabulary every system speaks now lives below this crate, in
// `ac-agent`. Re-exported under its old names so that nothing which
// says `crate::did` or `crate::pack` has to care where it went.
pub use ac_agent::{did, pack, refusals, room, weenie_errors};
// The judgements about carried items live in ac-loot now (the search
// language, what to wield, how a character fights); the client keeps
// the network side of them.
pub use ac_loot::weapons::Stance;

/// How long a take off a body -- a put or a pour -- is waited on before
/// it is given up for lost. The server walks to the body and stoops
/// before it answers; nine characters' takes were answered in 1.4 s at
/// the median and 2.1 s at the ninety-ninth.
const TAKE_LOST: Duration = Duration::from_secs(4);

/// A take off a body as it went out: what was asked for, where it was
/// to go, and what the server has said of it since.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TakeSent {
    pub(crate) item: u32,
    /// The pack named in the put, or the carried stack poured onto.
    pub(crate) into: u32,
    /// Sent once already and refused for room: this is its second try,
    /// into another pack, and there is no third.
    pub(crate) retried: bool,
    /// The server has said "Unable to put {item} into container" of it
    /// since it went out (see `refusals::Refusal::Put`).
    pub(crate) said_full: bool,
    /// The server has refused it, with this reason (0 for none). Kept
    /// with the take rather than acted on at once because the words
    /// that say why come as chat, and a tick reads its chat after its
    /// events -- or a packet later: the refusal is judged again when
    /// they arrive (see `Client::judge_take_refusal`).
    pub(crate) refused: Option<u32>,
}
pub mod logoff;
pub use logoff::{log_off_all, LOG_OFF_WAIT};
// Getting somewhere is its own system now (`ac-nav`), with the world
// behind a trait so that a route can be argued about without one.
pub use ac_nav::steering as route;
// The shopping is its own system now (`ac-vendor`); the planner it was
// built around keeps its old name here.
pub use ac_vendor::errand;

// Autoplay's own rules live under `autoplay` now, and keep their old paths.
pub use autoplay::{growth, steps, summoning};
pub mod holdings;
pub mod hunt;
pub mod items;
pub mod logistics;
pub mod magic;
pub mod options;
pub mod pathfinder;
pub mod plan;
pub mod player;
pub mod position;
pub mod profile;
pub mod recalls;
pub mod reconnect;
pub mod recovery;
mod refused;
mod session;
pub(crate) use session::apply::YOURE_TOO_BUSY;
pub mod shopping;
#[cfg(any(test, feature = "testkit"))]
pub mod testkit;
pub mod travel;
pub mod visit;
pub mod weapons;

use std::time::{Duration, Instant};

/// Things the session reports to whoever drives it (a UI, a script).
#[derive(Debug, Clone)]
pub enum Event {
    /// A line for the chat log; `kind` is the server's ChatMessageType.
    Chat {
        text: String,
        kind: u32,
    },
    /// A sound to play at a volume (0..=1).
    Sound {
        wave: std::rc::Rc<ac_formats::wave::Wave>,
        volume: f32,
    },
    /// An object plays a particle script (PlayEffect 0xF755): `script`
    /// is ACE's PlayScript id (0x51 Fizzle, 0x52 PortalEntry, 0x53
    /// PortalExit, 0x33 ShieldUpBlue...), `speed` its playback rate.
    Effect {
        guid: u32,
        script: u32,
        speed: f32,
    },
    Connected,
    Terminated(String),
    /// CharacterError / AccountBoot opcode.
    Refused(u32),
    /// The character stands in the world; a scene can be built around it.
    Placed {
        cell: u32,
    },
    /// A spell entered the spellbook (MagicUpdateSpell).
    SpellLearned(u32),
    /// A spell left the spellbook (MagicRemoveSpell).
    SpellForgotten(u32),
    /// The account's characters, when the client is not entering the
    /// world by itself (`Config::auto_enter` off and no `character`
    /// named, or the named one is missing). Re-emitted whenever the
    /// server refreshes the list (after a delete, entries pending
    /// deletion carry `seconds_until_deleted > 0`) and after a restore.
    Characters(Vec<ac_net::messages::CharacterEntry>),
    /// The server accepted a `create_character`; the client enters the
    /// world with it.
    CharacterCreated {
        id: u32,
        name: String,
    },
    /// The server refused a `create_character` (or a
    /// `restore_character`, which is answered by the same message) with
    /// an ACE `CharacterGenerationVerificationResponse` code; see
    /// `creation::create_failure_message`.
    CharacterCreateFailed(u32),
    /// Autoplay changed what it is doing (see `autoplay::Doing`): `doing`
    /// is the state's name in lower case ("fighting", "looting",
    /// "idle"...), `text` the line the panel shows ("fighting Drudge
    /// Skulker"). Emitted once per change, not once per tick.
    Autoplay {
        doing: String,
        text: String,
    },
}

/// How to reach the server and who to be.
/// This client does not hold its character to the game's formulas: out
/// of the box it runs twice as fast as the Run skill says, and a full
/// jump rises nine metres whatever the Jump skill says (the server's
/// tolerance is ten). The Options panel and the script API change both.
pub const DEFAULT_SPEED_BOOST: f32 = 2.0;
pub const DEFAULT_JUMP_HEIGHT: f32 = 9.0;

#[derive(Debug, Clone)]
pub struct Config {
    /// `host` or `host:port` of the login (primary) port.
    pub host: String,
    pub account: String,
    pub password: String,
    /// Character to enter with; the first on the account when None.
    pub character: Option<String>,
    /// Enter the world as soon as the character list arrives (with
    /// `character`, or the first one). Off, and with no `character`
    /// named, the client emits `Event::Characters` instead and waits for
    /// `enter_world` / `create_character`. A named `character` always
    /// auto-enters.
    pub auto_enter: bool,
}

pub struct Client {
    pub config: Config,
    /// `None` for a session with no server ([`Client::offline`]).
    pub socket: Option<std::net::UdpSocket>,
    pub primary: std::net::SocketAddr,
    pub secondary: std::net::SocketAddr,
    pub session: ac_net::session::Session,
    pub world: ac_world::World,
    pub assets: std::rc::Rc<ac_scene::Assets>,
    pub characters: Vec<ac_net::messages::CharacterEntry>,
    pub characters_known: bool,
    pub ddd_done: bool,
    /// CharacterEnterWorldRequest was sent (see `entering`).
    pub enter_requested: bool,
    /// Character we are entering (or will, once the handshake is done).
    pub entering: Option<u32>,
    /// A create or restore whose CharacterCreateResponse is still to come.
    pub pending_create: Option<creation::Pending>,
    /// What to create when the account lacks `config.character` (see
    /// `Client::create_when_missing`), and whether it was tried.
    create_if_missing: Option<creation::CreateSpec>,
    create_attempted: bool,
    /// Why the last `create_when_missing` failed, for a panel.
    create_error: Option<String>,
    /// Landblock the static scene is built around, once the player is placed.
    pub scene_block: Option<u32>,
    /// Server-requested MoveTo for our own character, until the server
    /// reports us idle again.
    pub move_to: Option<ac_world::object::MoveTarget>,
    pub move_to_since: Instant,
    /// The server has answered what the walk under way is for (a UseDone,
    /// a refused pickup) since it sent the walk (see [`server_walk_over`]
    /// and [`answers_walk`]).
    move_to_answered: bool,
    /// What the last use or pickup of something in the world was sent
    /// for, until a cast is sent: what a UseDone can be the answer to
    /// (see [`answers_walk`]).
    last_used: Option<u32>,
    /// Whether the run key was held on the last tick: what a stop
    /// reported ahead of a use says, so the next report agrees with it.
    held_run: bool,
    /// The server refusing to move, merge or split an item. It answers
    /// InventoryServerSaveFailed with the item's guid and, where it has
    /// one, a reason; without reading that, a step cannot tell a
    /// refusal from a request still in flight and so asks for ever.
    /// Keyed by the guid the refusal names.
    pub move_refused: std::collections::HashMap<u32, (u32, Instant)>,
    /// The server's last `UseDone`: its error, and the tick it came in on.
    /// A use of something the server no longer has is answered with that
    /// and nothing more (see `autoplay::answered_with_nothing`).
    pub(crate) use_done: Option<(u32, Instant)>,
    /// The tick the server last put something in words: a transient
    /// string or a weenie error. "You do not yet have the right to loot"
    /// is one, and it is what tells a corpse that is locked from one that
    /// is not there. The words themselves are read where they say what to
    /// do (see `autoplay::corpse_refused`); this says which ask they
    /// answered, since a tick's words cannot be an answer to that same
    /// tick's ask.
    pub(crate) told: Option<Instant>,
    /// Route steering toward `move_to` when the straight line to it is
    /// blocked (see `route`).
    pub steering: route::Steering,
    /// The server's objects standing round the character, which a
    /// walk goes round (see `ac_nav::obstacles`).
    pub clutter: ac_nav::Clutter,
    /// Planner for routes that leave the landblock we stand in, run on a
    /// thread of its own (see `pathfinder`).
    pub pathfinder: pathfinder::Pathfinder,
    /// The overland route being walked, if any (see `travel`).
    pub travel: travel::Travel,
    /// Going to see someone, and the last use sent by hand (see `visit`).
    pub visits: visit::Visits,
    /// Melee combat mode is on.
    pub combat: bool,
    /// Magic combat mode is on.
    pub magic: bool,
    /// Combat mode is missile (a bow, crossbow or thrown weapon is wielded).
    pub missile: bool,
    /// Attack height: 1 high, 2 medium, 3 low.
    pub attack_height: u32,
    /// Power (melee) or accuracy (missile) bar, 0..=1.
    pub attack_power: f32,
    /// Names of spells learnt from scrolls this session, for spells the
    /// SpellTable lacks; `world.stats.spells` is the spellbook itself.
    pub known_spells: std::collections::HashMap<u32, String>,
    /// Playing on its own: the rules and what it is doing (see
    /// [`autoplay`]).
    pub autoplay: autoplay::Autoplay,
    /// Whether this character's loot ledger has been read back yet. It
    /// cannot be until the server has said who the character is.
    ledger_loaded: bool,
    /// When the character was asked to log off, and whether the server
    /// has said it has (see [`Client::log_off`]).
    log_off_sent: Option<Instant>,
    logged_off: bool,
    /// Refuse to cast when components of the current formula are missing
    /// (`CastCheck::MissingComponents`). Off by default: the server
    /// decides whether components are required (`require_spell_comps`),
    /// and a refused cast is only a chat line.
    pub require_components: bool,
    /// Creature we keep swinging at until it dies or we stop.
    pub attack_target: Option<u32>,
    /// An attack was sent and AttackDone has not come back yet.
    pub attack_pending: bool,
    /// The creature the last attack was sent at, kept after the target
    /// is let go: the AttackDone that answers it ends a charge at it (see
    /// [`attack_ended_walk`]).
    attacked: Option<u32>,
    pub last_attack: Instant,
    pub attack_backoff: Duration,
    /// Something in the rules wants the character's hands -- a weapon
    /// swapped, a wand taken up -- and is waiting for the attack in
    /// flight to be answered. It books the tick after that answer, so
    /// that the change gets its turn: the client swings again the moment
    /// AttackDone arrives, and without the booking the wait never ends
    /// (see [`Client::mid_attack`]).
    wants_the_hands: bool,
    /// Name of the last creature we attacked (its corpse is what we loot).
    pub last_target_name: String,
    pub sound_tables:
        std::collections::HashMap<u32, Option<std::rc::Rc<ac_formats::sound_table::SoundTable>>>,
    pub waves: std::collections::HashMap<u32, Option<std::rc::Rc<ac_formats::wave::Wave>>>,
    /// Items still to take from the open container, one at a time (the
    /// server refuses a second pickup while one is in progress).
    pub loot_queue: std::collections::VecDeque<u32>,
    pub loot_inflight: Option<(u32, Instant)>,
    /// The take in flight as it was sent: which pack it was aimed at,
    /// and what the server has said about it since. A refusal names the
    /// item and not the pack, so the pack a "full" is about is this.
    pub(crate) loot_sent: Option<TakeSent>,
    /// The take in flight is a pour off the body onto a carried stack
    /// rather than a put, and this is what was poured and what the
    /// target held: its answer is read the way a tidying pour's is
    /// (see `pack::pour_answer`), off the target's count.
    pub(crate) loot_merge: Option<pack::PourSent>,
    /// The item at the head of the queue is a take refused for room and
    /// sent once more, into another pack. Once: a second refusal is the
    /// body's to be set aside for.
    pub(crate) loot_retry: Option<u32>,
    /// Packs the server has said were full, by guid, and how many items
    /// each held when it said so. The word stands until something
    /// leaves the pack (see `room::still_full`), and outranks the
    /// count: a take aimed at a pack the count believed had two slots
    /// was refused four hundred times.
    pub(crate) packs_said_full: std::collections::HashMap<u32, u32>,
    /// FellowshipUpdateRequest(open) has been sent for the current
    /// fellowship: the server sends fellows' vitals only to members
    /// whose panel it believes open.
    pub fellow_updates: bool,
    /// Items waiting to be appraised in the background (`appraise_all`),
    /// and the one asked for; their answers do not open the window.
    pub appraise_queue: std::collections::VecDeque<u32>,
    /// Appraisals asked for and not yet answered.
    ///
    /// Several at once: the server answers each on its own and a
    /// corpse of eight things asked one at a time is eight round trips
    /// of standing over it, which is most of what made looting slow.
    pub appraise_inflight: Vec<(u32, Instant)>,
    /// An item to put into a world container (chest, hook, storage) once
    /// the server has opened it for us: (item, container, asked at).
    pub pending_store: Option<(u32, u32, Instant)>,
    pub selected: Option<u32>,
    /// What was selected before `selected`: clicking a kit in the pack
    /// selects the kit, and using it then applies it to the player or
    /// item selected just before (see `use_target`).
    pub previous_selected: Option<u32>,
    /// The salvage window is open (an Ust was used); the panel closes it.
    pub salvage_open: bool,
    /// A jump asked for by a script or the bot, done on the next tick.
    pub pending_jump: Option<f32>,
    /// Someone being kept up with (a team's leader): where they are and
    /// how close to stop. Walked or flown to like a journey's leg.
    pub follow: Option<Follow>,
    /// A sidestep out of a spell's way (see `dodge`): where to, and
    /// until when. Outranks every other movement goal while it lasts.
    pub dodge_to: Option<(glam::Vec3, Instant)>,
    /// The projectiles being watched for it.
    pub dodge: dodge::State,
    /// Client-side multiplier on the run speed (see
    /// [`player::run_rate`] for the game's own rate and the server's
    /// tolerance). 1 is the game as it was.
    pub speed_boost: f32,
    /// Height of a full jump in metres, whatever the Jump skill says
    /// (0 = the skill's own; capped at [`player::MAX_JUMP_HEIGHT`], the
    /// server's tolerance).
    pub jump_height: f32,
    /// Which movement rules to hold the character to: by default the
    /// game's own away from home, so `speed_boost`, `jump_height` and
    /// flying only take effect on a server that allows them (see
    /// [`player::MovementRules`]).
    pub movement_rules: player::MovementRules,
    /// Appraisals received, by object guid (the last one is the panel's).
    pub appraisals: std::collections::HashMap<u32, ac_net::messages::Appraisal>,
    /// The loot profiles, shared with every other session in the
    /// process: a rule switched off is off for all of them at once
    /// (see [`profile::Library`]).
    pub profiles: std::sync::Arc<profile::Library>,
    /// The guid of the latest appraisal and a counter bumped with each.
    pub last_appraisal: Option<u32>,
    pub appraisal_seq: u64,
    /// The book, sign or plaque last opened, and a counter bumped when
    /// one arrives (the book window opens on it).
    pub book: Option<ac_net::messages::BookData>,
    pub book_seq: u64,
    /// Our allegiance's Turbine chat room (SetTurbineChatChannels), 0
    /// without one.
    pub allegiance_room: u32,
    /// Context id of the next Turbine chat request.
    turbine_context: u32,
    pub last_click: Option<(Instant, u32)>,
    pub player: Option<player::Player>,
    pub player_setup: u32,
    /// `disconnect` was called: this session ended on purpose and is
    /// never reconnected (see [`reconnect`]).
    pub quitting: bool,
    /// Why the session ended, once the server or the network ended it.
    pub ended: Option<String>,
    /// The last refusal from the server as (opcode, code): a
    /// CharacterError, AccountBoot or AccountBanned. `Event::Refused`
    /// carries only the opcode; the code says whether coming back is
    /// worth trying (see [`reconnect::classify_refusal`]).
    pub last_refusal: Option<(u32, u32)>,
    /// Pending events for the driver.
    pub events: Vec<Event>,
}

impl Client {
    /// The character is in the world and our physics body exists.
    pub fn placed(&self) -> bool {
        self.scene_block.is_some()
    }

    pub fn interact(&mut self, guid: u32) {
        use ac_net::messages::action;
        use ac_world::{item_type, object_desc_flags};
        let me = self.world.player_guid;
        // Using something in the world walks us to it: stop any journey
        // so it does not walk us away again afterwards.
        let in_the_world = self
            .world
            .objects
            .get(&guid)
            .is_some_and(|o| o.container.is_none() && o.wielder.is_none());
        if in_the_world {
            self.interrupt_travel("using something");
        }
        let Some(o) = self.world.objects.get(&guid) else {
            return;
        };
        let stuck = o.object_desc_flags
            & (object_desc_flags::STUCK
                | object_desc_flags::PLAYER
                | object_desc_flags::DOOR
                | object_desc_flags::VENDOR
                | object_desc_flags::PORTAL
                | object_desc_flags::CORPSE)
            != 0
            || o.item_type & item_type::CREATURE != 0;
        let carried = me.is_some() && (o.container == me || o.wielder == me);
        let name = o.name.clone();
        let attackable = o.object_desc_flags & object_desc_flags::ATTACKABLE != 0
            && o.object_desc_flags & object_desc_flags::PLAYER == 0
            && o.item_type & item_type::CREATURE != 0;
        let mut w = ac_net::wire::Writer::new();
        if self.combat && attackable {
            self.attack(guid);
            return;
        }
        if o.object_desc_flags & object_desc_flags::PLAYER != 0 && Some(guid) != me {
            // Using another player opens a secure trade (the retail client
            // sent this itself; the server ignores Use on players).
            self.open_trade(guid);
            return;
        }
        if carried && o.spell_id != 0 {
            if let Some(spell) = name.strip_prefix("Scroll of ") {
                self.known_spells.insert(o.spell_id, spell.to_string());
            }
        }
        // A kit, a mana stone or a key used with something selected is
        // applied to it (retail: select the target, then use the item).
        // With nothing selected an item that may be used on its owner (a
        // kit, a stone) goes on ourselves; the server has no plain Use
        // for those.
        if carried && ac_world::usable::needs_target(o.usable) {
            let target = self
                .use_target(guid)
                .or(if ac_world::usable::on_self(o.usable) {
                    me
                } else {
                    None
                });
            if let Some(target) = target {
                self.use_on(guid, target);
                return;
            }
        }
        if carried && o.weenie_class_id == ac_world::material::UST_WCID {
            // The Ust is the salvaging tool: using it opens the salvage
            // window (the retail client did this itself; the server only
            // hears the final list).
            self.salvage_open = !self.salvage_open;
            return;
        }
        if carried && o.wielder != me && o.item_type & item_type::WIELDABLE != 0 {
            tracing::info!("wield {name} ({guid:#010x}) at {:#x}", o.valid_locations);
            w.u32(guid).u32(o.valid_locations);
            self.session
                .send_action(action::GET_AND_WIELD_ITEM, &w.finish());
        } else if carried && o.wielder == me {
            tracing::info!("take off {name} ({guid:#010x})");
            w.u32(guid).u32(me.unwrap_or(0)).u32(0);
            self.session
                .send_action(action::PUT_ITEM_IN_CONTAINER, &w.finish());
        } else if !carried && !stuck && o.position.is_some() {
            // Double-click on a loose item picks it up, as retail did; the
            // "use selected" key (`use_object`) reads or activates it in
            // place instead.
            tracing::info!("pick up {name} ({guid:#010x})");
            self.last_used = Some(guid);
            w.u32(guid).u32(me.unwrap_or(0)).u32(0);
            self.session
                .send_action(action::PUT_ITEM_IN_CONTAINER, &w.finish());
        } else {
            tracing::info!("use {name} ({guid:#010x})");
            // Stopped first, on the wire: a stop reported after the use
            // cancels the walk the server starts for it, and the use with
            // it (ACE `GameActionMoveToState`).
            if in_the_world {
                if let Some(pl) = self.player.as_mut() {
                    pl.report_stopped(&mut self.session, self.held_run);
                    if let Some((cell, local)) = pl.take_sent() {
                        self.world.player_reported(cell, local);
                    }
                }
                // What a server walk that runs out was walking to.
                self.visits.used(guid, Instant::now());
            }
            self.last_used = Some(guid);
            self.session.send_action(action::USE, &guid.to_le_bytes());
        }
    }

    /// Cast a spell (see [`Client::try_cast`]); the outcome is logged.
    pub fn cast(&mut self, spell: u32) {
        let _ = self.try_cast(spell);
    }

    /// Cast a spell on the selected target (or ourselves), switching to
    /// magic mode first when needed. `CastCheck::Ok` once the cast was
    /// sent. Without a wielded caster nothing is sent (`NoCaster`: the
    /// server would only drop us back to peace mode); missing components
    /// refuse the cast only with `require_components` set. An unknown
    /// spell or low mana is logged and sent anyway, the server being the
    /// judge (Mana Conversion can make a cast the estimate rejects).
    pub fn try_cast(&mut self, spell: u32) -> magic::CastCheck {
        use ac_net::messages::action;
        use magic::CastCheck;
        let check = self.can_cast(spell);
        match &check {
            CastCheck::Ok => {}
            CastCheck::NoCaster => {
                tracing::warn!("cannot cast {spell}: no magic caster wielded");
                return check;
            }
            CastCheck::MissingComponents(missing) if self.require_components => {
                tracing::warn!("cannot cast {spell}: missing components {missing:?}");
                return check;
            }
            other => tracing::info!("casting {spell} although {other:?}"),
        }
        let table = self.assets.spell_table().ok();
        let entry = table.as_ref().and_then(|t| t.get(spell));
        let name = entry
            .map(|sp| sp.name.clone())
            .or_else(|| self.known_spells.get(&spell).cloned())
            .unwrap_or_default();
        // Spells that need a target go to the selected creature (or
        // ourselves when nothing is selected); the rest are self casts.
        let needs_target = entry
            .map(|sp| sp.needs_target())
            .unwrap_or_else(|| name.contains("Other"));
        let target = if needs_target {
            self.selected.or(self.world.player_guid)
        } else {
            None
        };
        // The selection can go stale between the tick that made it and
        // this one: the creature dies and is replaced by a corpse of
        // another guid. ACE looks the target up before it starts the
        // windup and answers TargetNotAcquired when it is not there
        // (`Player_Magic.cs`, `HandleActionCastTargetedSpell`), so the
        // cast is a wasted message and a wasted UseDone. The swing makes
        // exactly this test before it goes out; the cast never did, and
        // spent 36 of them in one run. Ourselves we never doubt: a self
        // cast is aimed at a character the server has in front of it by
        // definition, and asking the world about our own object would
        // only find new ways to refuse a buff.
        let gone = target
            .filter(|t| Some(*t) != self.world.player_guid)
            .is_some_and(|t| !self.target_still_there(t));
        if gone {
            tracing::debug!("not casting {name}: the target is gone");
            return CastCheck::NoTarget;
        }
        // A caster is wielded (can_cast said so), so entering combat is
        // entering magic mode; there is no separate choice to make.
        self.enter_combat();
        self.note_cast(spell);
        // The next UseDone is the cast's, not the answer to a use.
        self.last_used = None;
        match target {
            Some(t) => {
                tracing::info!("cast {name} ({spell}) on {t:#010x}");
                self.session.send_action(
                    action::CAST_TARGETED_SPELL,
                    &ac_net::messages::cast_targeted(t, spell),
                );
                // Keep the target's health bar moving (see query_health).
                let creature = self
                    .world
                    .objects
                    .get(&t)
                    .is_some_and(|o| o.item_type & ac_world::item_type::CREATURE != 0);
                if creature {
                    self.query_health(t);
                }
            }
            None => {
                tracing::info!("cast {name} ({spell})");
                self.session
                    .send_action(action::CAST_UNTARGETED_SPELL, &spell.to_le_bytes());
            }
        }
        CastCheck::Ok
    }

    /// Cast `spell` on a particular thing rather than on whatever is
    /// selected: an item being enchanted, a creature being softened.
    /// The selection is left as it was.
    pub fn cast_at(&mut self, spell: u32, target: u32) -> magic::CastCheck {
        let was = self.selected;
        self.selected = Some(target);
        let r = self.try_cast(spell);
        self.selected = was;
        r
    }

    pub fn attack(&mut self, guid: u32) {
        self.interrupt_travel("attacking");
        use ac_net::messages::action;
        // A wand does not swing and does not shoot: with a caster in
        // hand the way to attack is to cast.
        if self.combat_stance() == Stance::Magic {
            tracing::info!("cannot attack with a caster wielded; cast instead");
            return;
        }
        if !self.combat {
            return;
        }
        let name = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.name.clone())
            .unwrap_or_default();
        tracing::info!(
            "attack {name} ({guid:#010x}){}",
            if self.missile { " with a missile" } else { "" }
        );
        self.last_target_name = name.clone();
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid)
            .u32(self.attack_height)
            .f32(self.attack_power.clamp(0.0, 1.0));
        let opcode = if self.missile {
            action::TARGETED_MISSILE_ATTACK
        } else {
            action::TARGETED_MELEE_ATTACK
        };
        self.session.send_action(opcode, &w.finish());
        self.attack_target = Some(guid);
        self.attack_pending = true;
        self.attacked = Some(guid);
        self.last_attack = Instant::now();
        self.select(Some(guid));
        self.query_health(guid);
    }

    /// Select a creature on the server (QueryHealth 0x01BF): it answers
    /// UpdateHealth now and again every heartbeat (5 s) while the
    /// creature stays selected, which is the only way its health bar
    /// keeps moving between our own blows. Sent by `attack` and by a
    /// targeted cast; `None` clears the selection.
    pub fn query_health(&mut self, guid: impl Into<Option<u32>>) {
        let guid = guid.into().unwrap_or(0);
        self.session
            .send_action(ac_net::messages::action::QUERY_HEALTH, &guid.to_le_bytes());
    }

    /// Stop the attack in progress (CancelAttack 0x01B7): the server
    /// ends the swing chain with AttackDone and cancels its walk toward
    /// the target. Combat mode stays on.
    pub fn cancel_attack(&mut self) {
        if self.attack_target.take().is_some() || self.attack_pending {
            tracing::info!("cancel attack");
        }
        self.attack_pending = false;
        self.session
            .send_action(ac_net::messages::action::CANCEL_ATTACK, &[]);
    }

    /// Whether something we mean to act on is still there to act on: in
    /// view, and not a creature that has died. A dead creature does not
    /// linger under its own guid -- the server replaces it with a corpse
    /// of another -- so the guid simply goes, and anything still aimed
    /// at it is refused.
    pub fn target_still_there(&self, guid: u32) -> bool {
        self.world
            .objects
            .get(&guid)
            .is_some_and(|o| o.health.is_none_or(|h| h > 0.0))
    }

    pub fn tick_combat(&mut self) {
        let Some(target) = self.attack_target else {
            return;
        };
        let alive = self.target_still_there(target);
        // Dead: the server drops us to peace mode and refuses attacks
        // until we stand at the lifestone.
        let dead = !self.world.stats.name.is_empty() && self.world.stats.vitals[0].current == 0;
        if dead && self.combat {
            tracing::info!("dead: leaving combat");
            self.combat = false;
            self.missile = false;
            self.magic = false;
        }
        if !alive || !self.combat {
            tracing::info!("attack target gone");
            self.attack_target = None;
            self.attack_pending = false;
            return;
        }
        // Re-sending while the server walks us to the target would cancel
        // that walk, so wait for it; back off after a refused attack.
        if !self.attack_pending
            && self.move_to.is_none()
            && self.last_attack.elapsed() > self.attack_backoff
        {
            // A change of hands has been waiting for this attack to be
            // answered, and this is the gap between two swings it was
            // waiting for (see [`Client::wait_for_the_swing`]). It has
            // this tick; the next swing goes out on the one after. One
            // tick only, so nothing can hold the character's sword arm.
            if std::mem::take(&mut self.wants_the_hands) {
                return;
            }
            // And a swap already under way holds the swing until it
            // lands: between the put and the wield the hands are empty,
            // and a swing sent into that gap is a punch. The picker
            // asks this before it joins a fight; the fight it is
            // already in comes back through here, so it asks too.
            // Bounded by SWAP_SETTLES, so a swap that never lands
            // cannot stop the character fighting.
            if self.hands_changing(Instant::now()) {
                return;
            }
            self.attack(target);
        }
    }

    pub fn use_by_name(&mut self, name: &str) -> bool {
        let me = self.player.as_ref().map(|p| p.world_position());
        let my_guid = self.world.player_guid;
        let mut best: Option<(f32, u32)> = None;
        for o in self.world.objects.values() {
            if !o.name.starts_with(name) {
                continue;
            }
            // Carried items count as distance zero, so they win over the floor;
            // exact names beat prefix matches.
            let carried = my_guid.is_some() && (o.container == my_guid || o.wielder == my_guid);
            let exact = if o.name == name { 0.0 } else { 1000.0 };
            let d = exact
                + if carried {
                    0.0
                } else {
                    let Some(p) = o.display.or(o.position) else {
                        continue;
                    };
                    me.map(|m| (ac_world::landblock_origin(p.cell) + p.local).distance(m))
                        .unwrap_or(0.0)
                };
            if best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, o.guid));
            }
        }
        match best {
            Some((_, guid)) => {
                self.select(Some(guid));
                self.interact(guid);
                true
            }
            None => {
                tracing::debug!("no object named {name:?} in view yet");
                false
            }
        }
    }

    /// Wield the carried item whose name starts with `name` (the first
    /// exact match wins); nothing happens when it is wielded already.
    /// Returns false when no such item is carried.
    pub fn wield_by_name(&mut self, name: &str) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some((guid, locations, wielded)) = self
            .world
            .objects
            .values()
            .filter(|o| self.world.is_carried(o.guid) || o.wielder == me)
            .filter(|o| o.name.starts_with(name))
            .min_by_key(|o| if o.name == name { 0 } else { 1 })
            .map(|o| (o.guid, o.valid_locations, o.wielder == me))
        else {
            return false;
        };
        if !wielded {
            let mut w = ac_net::wire::Writer::new();
            w.u32(guid).u32(locations);
            self.session
                .send_action(action::GET_AND_WIELD_ITEM, &w.finish());
        }
        true
    }

    /// What this character can bring to bear, for judging what it may
    /// wield and how well (see `crate::weapons`).
    pub fn wielder(&self) -> crate::weapons::Wielder {
        let stats = &self.world.stats;
        let table = self.assets.skill_table().ok();
        let skills = stats
            .skills
            .iter()
            .map(|s| {
                let base = table.as_ref().and_then(|t| t.get(s.id));
                (
                    s.id,
                    stats.skill_value(s, base),
                    stats.skill_current(s, base),
                    s.advancement,
                )
            })
            .collect();
        let mut attributes = [0u32; 6];
        let mut attributes_current = [0u32; 6];
        for (i, a) in stats.attributes.iter().enumerate() {
            attributes[i] = a.value();
            attributes_current[i] = stats.attribute_current(i as u32 + 1);
        }
        let mut vitals = [0u32; 3];
        for (i, v) in vitals.iter_mut().enumerate() {
            *v = stats.vital_max_current(i);
        }
        crate::weapons::Wielder {
            level: stats.level.max(0) as u32,
            skills,
            attributes,
            attributes_current,
            vitals,
        }
    }

    /// Wield a carried item by guid, in whatever slot it goes in. True
    /// when the item is in hand or on its way there.
    ///
    /// Nothing is sent for an item already wielded, nor for one asked
    /// for so recently that the answer may still be in flight (see
    /// `Autoplay::wield_in_flight`) -- both of those the server
    /// answers with "You must remove your Slashing Sceptre to wield
    /// Slashing Sceptre" and a refusal that then backs the weapon off.
    /// Nothing is sent either while the item is inside a refusal's wait
    /// or while the server has us busy (see [`Client::wield_must_wait`]),
    /// and those two return false: the caller keeps the errand and asks
    /// again on a free tick.
    pub fn wield_guid(&mut self, guid: u32) -> bool {
        use ac_net::messages::action;
        let now = Instant::now();
        let me = self.world.player_guid;
        // Already in hand. Asking again is not a wasted message but a
        // harmful one: the refusal it earns backs the weapon off for
        // three seconds, then six, then twelve, and takes the wand the
        // buff pass needs with it.
        if self
            .world
            .objects
            .get(&guid)
            .is_some_and(|o| o.wielder == me)
        {
            return true;
        }
        if self.autoplay.wield_in_flight(guid, now) {
            return true;
        }
        let Some(locations) = self
            .world
            .objects
            .get(&guid)
            .filter(|o| self.world.is_carried(o.guid))
            .map(|o| o.valid_locations)
        else {
            return false;
        };
        if self.wield_must_wait(guid, now) {
            return false;
        }
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid).u32(locations);
        self.session
            .send_action(action::GET_AND_WIELD_ITEM, &w.finish());
        self.autoplay.wield_asked = Some((guid, now));
        true
    }

    /// The server has refused to move `item`. When that is the answer to
    /// a wield we asked for, decide what the refusal is worth.
    ///
    /// A refusal for something now in our own hands is the answer to an
    /// ask the server had already granted: the client asked twice before
    /// the first answer came back, and the second ask was refused for
    /// the first one having worked ("You must remove your Slashing
    /// Sceptre to wield Slashing Sceptre"). Backing the weapon off for
    /// that is worse than useless. [`Client::hold_off_wield`] clears
    /// `wield_asked`, and with it the check in
    /// [`Client::autoplay_pending_wield`] that would have forgotten the
    /// wait once the item was seen wielded -- so the wait only ever
    /// doubles, and the wand the buff pass needs is never taken up
    /// again. Nine characters went a whole run without casting Prodigal
    /// Strength once, behind a back-off against a wield that had worked.
    pub(crate) fn wield_refused(&mut self, item: u32, now: Instant) {
        if !self.autoplay.wield_asked.is_some_and(|(g, _)| g == item) {
            return;
        }
        let me = self.world.player_guid;
        if self
            .world
            .objects
            .get(&item)
            .is_some_and(|o| o.wielder == me)
        {
            self.autoplay.wield_asked = None;
            self.autoplay.wield_refused.forget(&item);
        } else {
            self.hold_off_wield(item, now);
        }
    }

    /// Whether the server has the character busy, so that a take sent
    /// now would be thrown away.
    ///
    /// ACE refuses a take made while `IsBusy` is set and answers with
    /// YoureTooBusy plus a wasted InventoryServerSaveFailed
    /// (`Player_Inventory.cs`, `HandleActionPutItemInContainer_Verify`).
    /// What sets `IsBusy` is what the server owes us a UseDone for: a
    /// spell being wound up (`MagicState.OnCastStart`), a recipe, a
    /// counter's sale. Nine characters bought eighty-nine "You're too
    /// busy" pairs in ten minutes taking from a body with a spell in the
    /// air.
    ///
    /// A swing is not one of them, whatever this used to say: nothing in
    /// ACE's melee or missile path sets `IsBusy`, and the wield handler
    /// does not read it at all -- the check in
    /// `HandleActionGetAndWieldItem` is commented out. The reason not to
    /// change hands mid-swing is a different one and is stated where it
    /// belongs (see [`Client::wield_must_wait`]).
    pub fn server_busy(&self, now: Instant) -> bool {
        self.autoplay.cast_in_flight(now)
    }

    /// Whether a wield of `guid` sent now would be wasted: the item is
    /// inside the wait a refusal earned it, the server has the character
    /// busy, or there is a swing in the air. Either way the errand keeps
    /// and goes out later.
    ///
    /// The swing is not the server refusing anything -- it would make
    /// the wield, and cancel the attack doing it (see
    /// `autoplay::change_of_hands_waits`). A weapon is not worth a cancelled
    /// swing when the gap between two of them is a few hundred
    /// milliseconds away.
    pub fn wield_must_wait(&self, guid: u32, now: Instant) -> bool {
        self.wield_held_off(guid)
            || self.server_busy(now)
            || attack_unanswered(self.attack_pending, self.last_attack, now)
    }

    /// Whether this item is inside the wait a refused wield earned it.
    ///
    /// The server refuses a wield it will not make with no error code
    /// at all, so there is nothing to read in the refusal and nothing to
    /// do but wait and try again later. Without this the buff pass asked
    /// for +Verity's Training Wand a hundred and fifty times in a
    /// minute, and was refused every one of them.
    pub fn wield_held_off(&self, guid: u32) -> bool {
        self.autoplay.wield_refused.held(&guid, Instant::now())
    }

    /// The arrows, bolts or quarrels wielded, if any. A bow shoots
    /// nothing without them, so autoplay refills the slot from the pack.
    pub fn wielded_ammo(&self) -> Option<u32> {
        self.world
            .wielded()
            .find(|o| o.valid_locations & ac_world::equip::MISSILE_AMMO != 0)
            .map(|o| o.guid)
    }

    /// Wield ammunition from the packs, the largest stack first. False
    /// when none is carried, or some is wielded already. Thrown weapons
    /// are their own ammunition and need none, so this does nothing for
    /// them.
    pub fn wield_ammo(&mut self) -> bool {
        use ac_net::messages::action;
        if self.wielded_ammo().is_some() {
            return false;
        }
        let Some((guid, locations, name)) = self
            .world
            .objects
            .values()
            .filter(|o| self.world.is_carried(o.guid))
            .filter(|o| o.valid_locations & ac_world::equip::MISSILE_AMMO != 0)
            .max_by_key(|o| o.stack_size.max(1))
            .map(|o| (o.guid, o.valid_locations, o.name.clone()))
        else {
            return false;
        };
        tracing::info!("wielding ammunition: {name} ({guid:#010x})");
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid).u32(locations & ac_world::equip::MISSILE_AMMO);
        self.session
            .send_action(action::GET_AND_WIELD_ITEM, &w.finish());
        true
    }
}

impl Client {
    /// Whether a swing or a charge is out and unanswered (see
    /// `attack_unanswered`). While one is, the rules leave the
    /// character's hands and its combat mode alone: every one of those
    /// changes cancels the attack in flight.
    ///
    /// The few changes worth a cancelled attack say so themselves --
    /// dropping to peace to loot a body, fleeing, coming back from a
    /// death -- and they do not ask.
    pub fn mid_attack(&self) -> bool {
        attack_unanswered(self.attack_pending, self.last_attack, Instant::now())
    }

    /// Book the gap after the swing in flight for whatever wanted the
    /// character's hands and found them busy.
    ///
    /// Without this the wait would never end: the client swings again
    /// the moment AttackDone arrives, so the rules would find an attack
    /// in flight every time they looked. Booked, the next swing is held
    /// back one tick and the change goes in between two of them, which
    /// is where the server wanted it all along.
    pub fn wait_for_the_swing(&mut self) {
        self.wants_the_hands = true;
    }

    /// The way this character fights right now, which is decided by
    /// what is in its hands and by nothing else: a wand, orb or staff
    /// means magic, a bow, crossbow or thrown weapon means missile, and
    /// anything else -- a sword, a mace, bare fists -- means melee.
    /// Entering combat enters the stance the hands imply; to fight
    /// another way, wield another weapon.
    pub fn combat_stance(&self) -> Stance {
        if self.wielded_caster().is_some() {
            Stance::Magic
        } else if self.wielded_missile_weapon().is_some() {
            Stance::Missile
        } else {
            Stance::Melee
        }
    }

    /// Enter combat, in whichever stance the wielded weapon gives.
    /// Nothing is sent when we are already in it.
    pub fn enter_combat(&mut self) {
        use ac_net::messages::{action, combat_mode};
        let stance = self.combat_stance();
        let already = match stance {
            Stance::Magic => self.magic,
            Stance::Missile => self.combat && self.missile,
            Stance::Melee => self.combat && !self.missile,
        };
        if already {
            return;
        }
        self.combat = stance != Stance::Magic;
        self.magic = stance == Stance::Magic;
        self.missile = stance == Stance::Missile;
        let mode = match stance {
            Stance::Melee => combat_mode::MELEE,
            Stance::Missile => combat_mode::MISSILE,
            Stance::Magic => combat_mode::MAGIC,
        };
        tracing::info!("combat mode {}", stance.label());
        self.session
            .send_action(action::CHANGE_COMBAT_MODE, &mode.to_le_bytes());
    }

    /// Wield a carried weapon that gives the stance `want`, so the
    /// character fights that way. True when something was sent. Nothing
    /// happens when the hands already give that stance, or when no such
    /// weapon is carried: a character can only fight with what it has.
    pub fn wield_for(&mut self, want: Stance) -> bool {
        use ac_net::messages::action;
        use ac_world::item_type;
        if self.combat_stance() == want {
            return false;
        }
        let mask = match want {
            Stance::Magic => item_type::CASTER,
            Stance::Missile => item_type::MISSILE_WEAPON,
            Stance::Melee => item_type::MELEE_WEAPON,
        };
        let Some((guid, locations, name)) = self
            .world
            .objects
            .values()
            .filter(|o| self.world.is_carried(o.guid))
            .filter(|o| o.item_type & mask != 0)
            // A stack of arrows is a missile weapon by type and worth
            // more than a training bow; the launcher is wanted here.
            .filter(|o| o.valid_locations & ac_world::equip::MISSILE_AMMO == 0)
            // The dearest one is usually the best one, and it is the
            // only ordering the client has before appraising.
            .max_by_key(|o| o.value)
            .map(|o| (o.guid, o.valid_locations, o.name.clone()))
        else {
            return false;
        };
        let now = Instant::now();
        // The same weapon, asked for a tick or two ago and not answered
        // yet. The swap is under way; asking again only buys "You must
        // remove your Slashing Sceptre to wield Slashing Sceptre" and a
        // back-off against a wield that had in fact worked -- which is
        // how nine characters went ten minutes without casting Prodigal
        // Strength once.
        if self.autoplay.wield_in_flight(guid, now) {
            return true;
        }
        if self.wield_must_wait(guid, now) {
            return false;
        }
        // The server will not put a bow in the hands that hold a wand:
        // whatever weapon is held goes back in the pack first, and the
        // new one is wielded on a later tick once it is there.
        let mut sent = self.put_weapons_away();
        // Nor will it put a wand in the hand of a character whose off
        // hand holds a shield -- it refuses the wield outright, with no
        // error to read. The shield comes off with the weapon, the way
        // the arming code already does it.
        let free_offhand = self
            .stats_of(guid)
            .is_some_and(|i| crate::weapons::needs_free_offhand(&i));
        if free_offhand {
            if let (Some(me), Some(shield)) = (self.world.player_guid, self.wielded_shield()) {
                sent |= self.put_in_container(shield, me);
            }
        }
        // When the swap started, so the fight waits for the hands to
        // settle rather than swinging into the moment they are empty
        // (see `Client::hands_changing`). The arming code stamps this
        // for its own swaps; without it here, every swap the buff pass
        // and the softening start was invisible to the fight, which is
        // how +Verity came to punch a Spikey Armoredillo.
        self.autoplay.last_rewield = Some(now);
        if sent {
            // Taken up by the housekeeping the moment the hands are
            // empty, rather than whenever the caller next happens to ask.
            self.autoplay.pending_wield = Some(guid);
            return true;
        }
        tracing::info!("wielding {name} to fight {}", want.label());
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid).u32(locations);
        self.session
            .send_action(action::GET_AND_WIELD_ITEM, &w.finish());
        self.autoplay.wield_asked = Some((guid, now));
        true
    }

    /// Put every weapon in hand back in the pack. True when something
    /// was sent; the hands are empty on a later tick.
    pub fn put_weapons_away(&mut self) -> bool {
        use ac_world::item_type;
        let Some(me) = self.world.player_guid else {
            return false;
        };
        let held: Vec<u32> = self
            .world
            .wielded()
            .filter(|o| {
                o.item_type
                    & (item_type::MELEE_WEAPON | item_type::MISSILE_WEAPON | item_type::CASTER)
                    != 0
                    && o.valid_locations & ac_world::equip::MISSILE_AMMO == 0
            })
            .map(|o| o.guid)
            .collect();
        let mut sent = false;
        for g in held {
            sent |= self.put_in_container(g, me);
        }
        sent
    }

    /// Drop back to peace mode.
    pub fn leave_combat(&mut self) {
        use ac_net::messages::{action, combat_mode};
        if !self.combat && !self.magic {
            return;
        }
        self.combat = false;
        self.magic = false;
        self.missile = false;
        tracing::info!("combat mode peace");
        self.session.send_action(
            action::CHANGE_COMBAT_MODE,
            &combat_mode::NON_COMBAT.to_le_bytes(),
        );
        self.attack_target = None;
        self.attack_pending = false;
    }

    /// The wielded bow, crossbow or thrown weapon, if any.
    pub fn wielded_missile_weapon(&self) -> Option<u32> {
        // Arrows are missile weapons by type too; the launcher is what
        // is wanted here.
        self.world
            .wielded()
            .find(|o| {
                o.item_type & ac_world::item_type::MISSILE_WEAPON != 0
                    && o.valid_locations & ac_world::equip::MISSILE_AMMO == 0
            })
            .map(|o| o.guid)
    }

    /// Enter or leave combat. Which stance it enters is not a choice:
    /// it is whatever the wielded weapon gives (see `combat_stance`).
    pub fn toggle_combat(&mut self) {
        if self.combat || self.magic {
            self.leave_combat();
        } else {
            self.enter_combat();
        }
    }

    fn appraisal(&mut self, body: &[u8]) {
        use ac_net::messages::Appraisal;
        let a = match Appraisal::parse(body) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!("appraisal: {e}");
                return;
            }
        };
        let name = self
            .world
            .objects
            .get(&a.guid)
            .map(|o| o.name.clone())
            .unwrap_or_else(|| format!("{:#010x}", a.guid));
        let mut lines = vec![name.clone()];
        for key in [
            Appraisal::STRING_SHORT_DESC,
            Appraisal::STRING_LONG_DESC,
            Appraisal::STRING_USE,
        ] {
            if let Some(t) = a.string(key) {
                if !t.is_empty() {
                    lines.push(t.to_string());
                }
            }
        }
        if !a.success {
            lines.push("(appraisal failed)".into());
        }
        tracing::info!(
            "appraise {name}: {} properties",
            a.ints.len() + a.strings.len()
        );
        // A background appraisal (appraise_all) fills the cache without
        // opening the window or writing to the chat.
        let asked = self.appraise_inflight.iter().any(|(g, _)| *g == a.guid);
        self.appraise_inflight.retain(|(g, _)| *g != a.guid);
        if !asked {
            for l in lines {
                self.events.push(Event::Chat { text: l, kind: 1 });
            }
            self.last_appraisal = Some(a.guid);
            self.appraisal_seq += 1;
        }
        self.appraisals.insert(a.guid, a);
    }

    /// A creature the server lets go of takes its appraisal with it.
    /// ACE reuses a guid six hours after the thing that had it is gone
    /// (`GuidManager`), and a session runs longer than that: kept, the
    /// answer about a Drudge was read as the answer about the Cow that
    /// got its guid, and the Cow was fought on it. A creature that only
    /// went out of range is let go of the same way and asked about
    /// afresh when it comes back, which costs one question. Corpses and
    /// items keep theirs: the loot rules read those, and a guid that
    /// was a corpse comes back as nothing the loot rules would be asked
    /// about before the answer has long stopped mattering.
    fn forget_the_departed(&mut self, msg: &[u8]) {
        use ac_net::messages::{opcode, split};
        let Some((opcode::OBJECT_DELETE, body)) = split(msg) else {
            return;
        };
        let Some(guid) = body
            .get(..4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        else {
            return;
        };
        let creature = self
            .world
            .objects
            .get(&guid)
            .is_some_and(|o| o.item_type & ac_world::item_type::CREATURE != 0 && !o.is_player);
        if creature {
            self.appraisals.remove(&guid);
        }
    }

    /// Use an object where it is (Use 0x0036), never picking it up: read
    /// a book lying on the ground, open a chest, talk to an NPC. Retail's
    /// "Use Selected" action. False when the object is unknown.
    pub fn use_object(&mut self, guid: u32) -> bool {
        self.interrupt_travel("using something");
        let Some(o) = self.world.objects.get(&guid) else {
            return false;
        };
        tracing::info!("use {} ({guid:#010x}) in place", o.name);
        self.last_used = Some(guid);
        self.session
            .send_action(ac_net::messages::action::USE, &guid.to_le_bytes());
        true
    }

    /// Pick a loose item up into the pack (PutItemInContainer, naming
    /// the pack with room: the main pack while it has a slot, else the
    /// roomiest side pack, since the server puts a thing into the pack
    /// named and nowhere else). False for anything that is not a loose
    /// item.
    pub fn pick_up(&mut self, guid: u32) -> bool {
        self.interrupt_travel("picking something up");
        let Some(me) = self.world.player_guid else {
            return false;
        };
        let Some(o) = self.world.objects.get(&guid) else {
            return false;
        };
        let stuck = o.object_desc_flags & ac_world::object_desc_flags::STUCK != 0
            || o.item_type & ac_world::item_type::CREATURE != 0;
        if stuck || o.position.is_none() || o.container.is_some() || o.wielder.is_some() {
            return false;
        }
        tracing::info!("pick up {} ({guid:#010x})", o.name);
        self.last_used = Some(guid);
        // The same choice a take makes, less the pour (see
        // `Client::where_to_pick_up`). With no room anywhere the main
        // pack is named, and the server says so; a caller that meant to
        // check first has `pack_full`.
        let into = self.where_to_pick_up(guid).unwrap_or(me);
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid).u32(into).u32(0);
        self.session
            .send_action(ac_net::messages::action::PUT_ITEM_IN_CONTAINER, &w.finish());
        true
    }

    /// Ask for one page of the open book (BookPageData 0x00AE); the
    /// answer fills `book.pages[index]`.
    pub fn read_page(&mut self, guid: u32, index: u32) {
        let mut w = ac_net::wire::Writer::new();
        w.u32(guid).i32(index as i32);
        self.session
            .send_action(ac_net::messages::action::BOOK_PAGE_DATA, &w.finish());
    }

    /// Open a carried book without "using" it (BookData 0x00AA).
    pub fn open_book(&mut self, guid: u32) {
        self.session
            .send_action(ac_net::messages::action::BOOK_DATA, &guid.to_le_bytes());
    }

    /// Select an item in a panel and appraise it, what a single click on
    /// a pack, container, vendor or trade row does.
    pub fn inspect(&mut self, guid: u32) {
        self.select(Some(guid));
        self.appraise(guid);
    }

    /// Do a soul emote by word ("wave", "bow", "*cheer*"): play the
    /// motion, send it in the next movement state so others see it, and
    /// say the line (SoulEmote 0x01E1: "waves"). False for unknown words.
    pub fn emote(&mut self, words: &str) -> bool {
        let Some((cmd, text)) = emotes::lookup(words) else {
            return false;
        };
        if let Some(pl) = self.player.as_mut() {
            pl.play_command(&self.assets, cmd, 1.0);
            pl.queue_command(cmd);
        }
        let mut w = ac_net::wire::Writer::new();
        w.string16(text);
        self.session
            .send_action(ac_net::messages::action::SOUL_EMOTE, &w.finish());
        true
    }

    /// Ask the server to appraise an object (IdentifyObject 0x00C8); the
    /// answer lands in `appraisals` and opens the appraisal panel.
    pub fn appraise(&mut self, guid: u32) {
        self.session.send_action(
            ac_net::messages::action::IDENTIFY_OBJECT,
            &guid.to_le_bytes(),
        );
    }

    pub fn tick_loot(&mut self, now: Instant) {
        use ac_net::messages::action;
        let me = self.world.player_guid.unwrap_or(0);
        // What the server said was full is held against what each pack
        // holds now, before anything is judged from it.
        self.note_pack_counts();
        // The in-flight pickup is done once the item is ours, gone, or stale.
        if let Some((guid, since)) = self.loot_inflight {
            let over = match self.loot_merge.as_ref() {
                // A pour off the body onto a carried stack is answered
                // the way a tidying pour is: the target's count grows,
                // or a refusal names one of the two stacks (see
                // `pack::pour_answer`). The source going is no answer --
                // it goes before the target's new count arrives.
                Some(sent) => {
                    let (from, to) = (sent.merge.from, sent.merge.to);
                    let target_now = self.world.objects.get(&to).map(|o| o.stack_size.max(1));
                    let refusal = pack::refusal_of(
                        since,
                        self.move_refused.get(&from).copied(),
                        self.move_refused.get(&to).copied(),
                    );
                    // Waited on as long as a put would be: the server
                    // walks to the body and stoops for a merge off it
                    // exactly as for a take, and one take in ten took
                    // longer than the tidy's two seconds. Given up
                    // sooner, the coin still lying there was asked for
                    // again and a second merge went out behind the
                    // first, to fail on a stack that had gone.
                    let waited = now.saturating_duration_since(since);
                    match pack::pour_answer_within(sent, target_now, refusal, waited, TAKE_LOST) {
                        pack::PourAnswer::InAir => false,
                        pack::PourAnswer::Landed => {
                            // What the pile is for is settled now that
                            // the coin is on it, as a tidying pour's is.
                            self.autoplay.ledger.merged(from, to);
                            true
                        }
                        // The refusal is left where it is for the loot
                        // rules to read (`ac_loot::Open::refused`).
                        pack::PourAnswer::Refused(_) | pack::PourAnswer::Lost => true,
                    }
                }
                None => {
                    // Ours wherever it landed: a take named a side pack
                    // when the main one was full, and an item in a side
                    // pack is not in the character's own container.
                    let landed = self
                        .world
                        .objects
                        .get(&guid)
                        .is_none_or(|o| self.world.is_carried(guid) || o.wielder == Some(me));
                    landed || now.saturating_duration_since(since) > TAKE_LOST
                }
            };
            if over {
                self.loot_inflight = None;
            }
        }
        // What was sent (`loot_sent`, `loot_merge`) is kept past the
        // take's end, until the next goes out: the words that say why
        // a take was refused come as chat, and can come a packet behind
        // the refusal itself.
        //
        // Not while the server has us busy: it refuses the take outright
        // and spends two messages saying so (see [`Client::server_busy`]).
        // The item stays at the head of the queue and goes out on the
        // first free tick, which costs a frame and never a pickup.
        if self.loot_inflight.is_none() && !self.server_busy(now) {
            // Where it goes is decided as it is sent, from the room
            // there is now: the pack with a slot, or the carried pile
            // it pours onto (see `room::how_to_take`).
            let next = self
                .loot_queue
                .front()
                .map(|guid| (*guid, self.how_to_take(*guid)));
            // Not a pour while the tidying's own pour is in the air.
            // The tidying holds off while a take is (see
            // `autoplay::TidyGate`), and this is the other half of it:
            // the pile the coin would join may be the one the tidying is
            // emptying, or the one it is filling, whose count the two
            // answers would then be read off together. A pour within
            // the pack is answered in a tick or two, and this waits the
            // tick.
            let tidy_pouring = self.autoplay.pour.is_some();
            if let Some((guid, how)) = next
                .filter(|(_, how)| !(tidy_pouring && matches!(how, Some(room::Take::Merge { .. }))))
            {
                self.loot_queue.pop_front();
                let retried = self.loot_retry.take() == Some(guid);
                let name = self
                    .world
                    .objects
                    .get(&guid)
                    .map(|o| o.name.clone())
                    .unwrap_or_default();
                let mut w = ac_net::wire::Writer::new();
                self.loot_merge = None;
                let into = match how {
                    Some(room::Take::Merge { to, amount }) => {
                        tracing::info!("take {name} ({guid:#010x}): pouring onto {to:#010x}");
                        let to_before = self
                            .world
                            .objects
                            .get(&to)
                            .map(|o| o.stack_size.max(1))
                            .unwrap_or(0);
                        w.u32(guid).u32(to).i32(amount as i32);
                        self.session
                            .send_action(action::STACKABLE_MERGE, &w.finish());
                        self.loot_merge = Some(pack::PourSent {
                            merge: pack::Merge {
                                from: guid,
                                to,
                                amount,
                                name,
                                frees_a_slot: true,
                            },
                            to_before,
                        });
                        to
                    }
                    put => {
                        // Nowhere the room model can find is the main
                        // pack, as every take once was: the server's
                        // answer says whether it was right.
                        let into = match put {
                            Some(room::Take::Put(into)) => into,
                            _ => me,
                        };
                        tracing::info!("take {name} ({guid:#010x}) into {into:#010x}");
                        w.u32(guid).u32(into).u32(0);
                        self.session
                            .send_action(action::PUT_ITEM_IN_CONTAINER, &w.finish());
                        into
                    }
                };
                self.loot_inflight = Some((guid, now));
                self.loot_sent = Some(TakeSent {
                    item: guid,
                    into,
                    retried,
                    said_full: false,
                    refused: None,
                });
            }
        }
    }

    /// Queue an item of the open container to be picked up.
    pub fn take(&mut self, guid: u32) {
        if !self.loot_queue.contains(&guid) && self.loot_inflight.map(|(g, _)| g) != Some(guid) {
            self.loot_queue.push_back(guid);
        }
    }

    /// Drop a carried item on the ground in front of the character
    /// (DropItem 0x001B). The server answers with InventoryPutObjectIn3D
    /// and creates the object in the world.
    pub fn drop_item(&mut self, guid: u32) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some(o) = self.world.objects.get(&guid) else {
            return false;
        };
        if me.is_none() || (o.container != me && o.wielder != me) {
            return false;
        }
        tracing::info!("drop {} ({guid:#010x})", o.name);
        self.session
            .send_action(action::DROP_ITEM, &guid.to_le_bytes());
        true
    }

    /// Move a carried item into a container (PutItemInContainer 0x0019):
    /// our own guid for the main pack, a carried side pack, or the ground
    /// container we are looking into (a chest; corpses refuse).
    pub fn put_in_container(&mut self, item: u32, container: u32) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some(o) = self.world.objects.get(&item) else {
            return false;
        };
        if me.is_none() || (o.container != me && o.wielder != me) || item == container {
            return false;
        }
        let name = o.name.clone();
        let target = self
            .world
            .objects
            .get(&container)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "pack".into());
        tracing::info!("put {name} ({item:#010x}) in {target} ({container:#010x})");
        // Whatever the server says about this item now is about the put,
        // not about some earlier ask to wield it: a refused put must not
        // be counted against the wield (see `Client::hold_off_wield`).
        if self.autoplay.wield_asked.is_some_and(|(g, _)| g == item) {
            self.autoplay.wield_asked = None;
        }
        let mut w = ac_net::wire::Writer::new();
        w.u32(item).u32(container).u32(0);
        self.session
            .send_action(action::PUT_ITEM_IN_CONTAINER, &w.finish());
        true
    }

    /// Put a carried item into a container standing in the world (a chest,
    /// a house hook or a storage chest). The server only takes items into
    /// an open container ("The container is closed" otherwise), so when it
    /// is not the one we are looking into this uses it first and stores
    /// the item once its contents arrive. False when the item is not ours.
    pub fn store_in(&mut self, item: u32, container: u32) -> bool {
        let me = self.world.player_guid;
        let ours = self
            .world
            .objects
            .get(&item)
            .is_some_and(|o| me.is_some() && (o.container == me || o.wielder == me));
        if !ours {
            return false;
        }
        if self.world.open_container.as_ref().map(|(g, _)| *g) == Some(container) {
            return self.put_in_container(item, container);
        }
        tracing::info!("store {item:#010x} in {container:#010x}: opening it first");
        self.pending_store = Some((item, container, Instant::now()));
        self.interact(container);
        true
    }

    /// Finish a `store_in` once its container is open; give up after a
    /// few seconds (out of reach, no permission).
    fn tick_store(&mut self) {
        let Some((item, container, since)) = self.pending_store else {
            return;
        };
        if self.world.open_container.as_ref().map(|(g, _)| *g) == Some(container) {
            self.pending_store = None;
            self.put_in_container(item, container);
        } else if since.elapsed() > Duration::from_secs(5) {
            tracing::info!("store {item:#010x}: {container:#010x} did not open");
            self.pending_store = None;
        }
    }

    /// Hand a carried item (or `amount` of a stack; the whole stack when
    /// None) to an NPC or another player (GiveObjectRequest 0x00CD). The
    /// target must be in reach; NPCs answer with an emote, players must
    /// allow gifts in their options.
    pub fn give(&mut self, target: u32, item: u32, amount: Option<u32>) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some(o) = self.world.objects.get(&item) else {
            return false;
        };
        if me.is_none() || (o.container != me && o.wielder != me) || Some(target) == me {
            return false;
        }
        let amount = amount.unwrap_or(o.stack_size.max(1)).max(1);
        let name = o.name.clone();
        let who = self
            .world
            .objects
            .get(&target)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        tracing::info!("give {amount} x {name} ({item:#010x}) to {who} ({target:#010x})");
        let mut w = ac_net::wire::Writer::new();
        w.u32(target).u32(item).u32(amount);
        self.session
            .send_action(action::GIVE_OBJECT_REQUEST, &w.finish());
        true
    }

    /// Found a fellowship (FellowshipCreate 0x00A2: name, share XP).
    pub fn fellowship_create(&mut self, name: &str, share_xp: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.string16(name.trim()).u32(u32::from(share_xp));
        self.session
            .send_action(ac_net::messages::action::FELLOWSHIP_CREATE, &w.finish());
    }

    /// Invite a player (FellowshipRecruit 0x00A5); they get a
    /// confirmation to answer.
    pub fn fellowship_recruit(&mut self, player: u32) {
        self.session.send_action(
            ac_net::messages::action::FELLOWSHIP_RECRUIT,
            &player.to_le_bytes(),
        );
    }

    /// Leave the fellowship; the leader may disband it instead.
    pub fn fellowship_quit(&mut self, disband: bool) {
        self.session.send_action(
            ac_net::messages::action::FELLOWSHIP_QUIT,
            &u32::from(disband).to_le_bytes(),
        );
    }

    /// Remove a member (leader only).
    pub fn fellowship_dismiss(&mut self, player: u32) {
        self.session.send_action(
            ac_net::messages::action::FELLOWSHIP_DISMISS,
            &player.to_le_bytes(),
        );
    }

    /// Answer a server confirmation (ConfirmationResponse 0x0275).
    pub fn confirm(&mut self, kind: u32, context: u32, yes: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.i32(kind as i32).u32(context).i32(i32::from(yes));
        self.session
            .send_action(ac_net::messages::action::CONFIRMATION_RESPONSE, &w.finish());
        self.world
            .confirmations
            .retain(|c| !(c.kind == kind && c.context == context));
    }

    /// The Ust we carry, if any: the salvaging tool.
    pub fn salvage_tool(&self) -> Option<u32> {
        self.world
            .inventory()
            .find(|o| o.weenie_class_id == ac_world::material::UST_WCID)
            .map(|o| o.guid)
    }

    /// Carried, unwielded items the server would salvage: those with a
    /// material and a workmanship (loot, not vendor stock).
    pub fn salvageable(&self) -> Vec<u32> {
        let me = self.world.player_guid;
        let mut items: Vec<&ac_world::WorldObject> = self
            .world
            .inventory()
            .filter(|o| {
                o.wielder != me
                    && o.material != 0
                    && o.workmanship > 0.0
                    && !o.name.starts_with("Salvaged ")
            })
            .collect();
        items.sort_by(|a, b| a.name.cmp(&b.name).then(a.guid.cmp(&b.guid)));
        items.into_iter().map(|o| o.guid).collect()
    }

    /// Salvage carried items with the Ust (CreateTinkeringTool 0x027D:
    /// tool, count, guids). The server destroys them and answers with
    /// SalvageOperationsResult per skill used, shown in chat, and the
    /// salvage bags appear in the pack. False without an Ust or items.
    pub fn salvage(&mut self, items: &[u32]) -> bool {
        let Some(tool) = self.salvage_tool() else {
            return false;
        };
        let me = self.world.player_guid;
        // Side packs count: the server looks for each item in them too
        // (ACE `GetInventoryItem`). Leaving them out sent nothing for a
        // batch that was all in a side pack, and the autoplay, which
        // salvages one workmanship grade at a time, was stuck on it.
        let items: Vec<u32> = items
            .iter()
            .copied()
            .filter(|g| {
                self.world.is_carried(*g)
                    || self.world.objects.get(g).is_some_and(|o| o.wielder == me)
            })
            .collect();
        if items.is_empty() {
            return false;
        }
        tracing::info!("salvage {} items with {tool:#010x}", items.len());
        let mut w = ac_net::wire::Writer::new();
        w.u32(tool).u32(items.len() as u32);
        for g in &items {
            w.u32(*g);
        }
        self.session
            .send_action(ac_net::messages::action::CREATE_TINKERING_TOOL, &w.finish());
        true
    }

    /// Free item slots in one of the character's packs: the main pack
    /// when `pack` is the character, a side pack otherwise.
    ///
    /// Packs and Foci sitting in the main pack take a pack slot rather
    /// than an item slot, so they are not counted against it.
    fn free_slots_in(&self, pack: u32) -> u32 {
        let me = self.world.player_guid;
        let capacity = if Some(pack) == me {
            self.world
                .player()
                .map(|p| p.items_capacity)
                .filter(|c| *c > 0)
                .unwrap_or(102)
        } else {
            self.world
                .objects
                .get(&pack)
                .map_or(0, |o| o.items_capacity)
        };
        let used = self
            .world
            .objects
            .values()
            .filter(|o| {
                o.container == Some(pack)
                    && !(Some(pack) == me
                        && ac_world::pack_slot::used_by(
                            o.weenie_class_id,
                            o.item_type & ac_world::item_type::CONTAINER != 0,
                        ))
            })
            .count() as u32;
        capacity.saturating_sub(used)
    }

    /// A pack with room for one more item, `prefer` first: where a piece
    /// cut off a stack can land. None when every pack is full.
    fn pack_with_room(&self, prefer: Option<u32>) -> Option<u32> {
        let me = self.world.player_guid;
        prefer
            .filter(|p| self.free_slots_in(*p) > 0)
            .or_else(|| me.filter(|m| self.free_slots_in(*m) > 0))
            .or_else(|| {
                // The lowest guid among the side packs with room, so
                // that the answer does not wander between frames.
                self.world
                    .objects
                    .values()
                    .filter(|o| {
                        o.container == me
                            && o.item_type & ac_world::item_type::CONTAINER != 0
                            && self.free_slots_in(o.guid) > 0
                    })
                    .map(|o| o.guid)
                    .min()
            })
    }

    /// Split `amount` off a carried stack into a container (the pack the
    /// stack is already in when None; StackableSplitToContainer
    /// 0x0055). The server creates the new stack and updates the old
    /// one's size. False when the amount is not below the stack size,
    /// or when no pack has a slot for the piece.
    pub fn split_stack(&mut self, item: u32, container: Option<u32>, amount: u32) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some(o) = self.world.objects.get(&item) else {
            return false;
        };
        // A stack in a side pack is as much in hand as one in the main
        // pack: the server looks for it anywhere the character can move
        // things from, which is what [`Self::merge_stacks`] already
        // allows. Asking only about the main pack made a pile kept in a
        // sack uncuttable and said nothing about it, and a counter that
        // will not take the pile whole then waited for a piece that
        // could never be cut -- for the length of the visit.
        if me.is_none() || !(self.world.is_carried(item) || o.wielder == me) {
            return false;
        }
        if amount == 0 || amount >= o.stack_size {
            return false;
        }
        // Where the piece goes. The server puts a split into the pack it
        // is told and no other -- `limitToMainPackOnly` -- so naming a
        // full pack is a refusal ("TryAddToInventory failed!") and not a
        // spill into a side one. Left to itself the cut stays where the
        // pile is, and only looks elsewhere if that pack is full.
        let Some(target) = container.or_else(|| self.pack_with_room(o.container)) else {
            return false;
        };
        tracing::info!(
            "split {} off {} ({item:#010x}) into {target:#010x}",
            amount,
            o.name
        );
        let mut w = ac_net::wire::Writer::new();
        w.u32(item).u32(target).i32(0).i32(amount as i32);
        self.session
            .send_action(action::STACKABLE_SPLIT_TO_CONTAINER, &w.finish());
        true
    }

    /// Drop `amount` of a carried stack on the ground
    /// (StackableSplitTo3D 0x0056); the whole stack goes with `drop_item`.
    pub fn split_to_ground(&mut self, item: u32, amount: u32) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some(o) = self.world.objects.get(&item) else {
            return false;
        };
        if me.is_none() || o.container != me || amount == 0 || amount >= o.stack_size {
            return false;
        }
        tracing::info!("drop {} of {} ({item:#010x})", amount, o.name);
        let mut w = ac_net::wire::Writer::new();
        w.u32(item).i32(amount as i32);
        self.session
            .send_action(action::STACKABLE_SPLIT_TO_3D, &w.finish());
        true
    }

    /// Merge a carried stack into another of the same kind, and settle
    /// what the surviving stack was taken for.
    ///
    /// This is the way in for anything that asks for a merge and does
    /// not follow it up: a panel's drag, a script, the shopping. The
    /// rules' own tidying sends it with `Self::send_merge` and settles
    /// the ledger when the server says the pour landed, since a refused
    /// pour settles nothing.
    pub fn merge_stacks(&mut self, from: u32, to: u32, amount: Option<u32>) -> bool {
        if !self.send_merge(from, to, amount) {
            return false;
        }
        // While both entries are still there to read: the source's goes
        // when the thing itself does.
        self.autoplay.ledger.merged(from, to);
        true
    }

    /// The merge itself (StackableMerge 0x0054: from, to, amount; the
    /// whole source when `amount` is None). The server caps at the
    /// target's maximum stack and leaves the rest in the source.
    pub(crate) fn send_merge(&mut self, from: u32, to: u32, amount: Option<u32>) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let (Some(a), Some(b)) = (self.world.objects.get(&from), self.world.objects.get(&to))
        else {
            return false;
        };
        // Either stack may sit in a side pack: the server searches
        // everywhere the character can move things from, and a purchase
        // that landed in the main pack often belongs with a pile kept
        // in a side one. The target may also be in hand, which is how a
        // quiver gets topped up; the source may not, since emptying
        // what the character is holding is not tidying.
        let to_in_hand = b.wielder == me;
        if me.is_none()
            || from == to
            || !self.world.is_carried(from)
            || !(self.world.is_carried(to) || to_in_hand)
            || a.weenie_class_id != b.weenie_class_id
            || b.max_stack_size <= 1
        {
            return false;
        }
        let amount = amount.unwrap_or(a.stack_size).clamp(1, a.stack_size);
        tracing::info!(
            "merge {} of {} ({from:#010x}) into {to:#010x}",
            amount,
            a.name
        );
        let mut w = ac_net::wire::Writer::new();
        w.u32(from).u32(to).i32(amount as i32);
        self.session
            .send_action(action::STACKABLE_MERGE, &w.finish());
        true
    }

    /// Add a friend by name (AddFriend 0x0018); the server answers with a
    /// FriendsListUpdate or "X not found" in chat.
    pub fn add_friend(&mut self, name: &str) {
        let mut w = ac_net::wire::Writer::new();
        w.string16(name.trim());
        self.session
            .send_action(ac_net::messages::action::ADD_FRIEND, &w.finish());
    }

    /// Drop a friend (RemoveFriend 0x0017: guid), or everyone with
    /// `None` (RemoveAllFriends 0x0025).
    pub fn remove_friend(&mut self, guid: Option<u32>) {
        match guid {
            Some(g) => self
                .session
                .send_action(ac_net::messages::action::REMOVE_FRIEND, &g.to_le_bytes()),
            None => self
                .session
                .send_action(ac_net::messages::action::REMOVE_ALL_FRIENDS, &[]),
        }
    }

    /// Show one of our titles (TitleSet 0x002C); the server confirms
    /// with UpdateTitle.
    pub fn set_title(&mut self, title: u32) {
        self.session
            .send_action(ac_net::messages::action::TITLE_SET, &title.to_le_bytes());
    }

    /// Squelch or unsquelch a character (ModifyCharacterSquelch 0x0058:
    /// flag, guid, name, ChatMessageType; guid 0 with a name looks the
    /// player up, chat type 1 = all channels), their whole account
    /// (ModifyAccountSquelch 0x0059: flag, name), or a chat type from
    /// everyone (ModifyGlobalSquelch 0x005B: flag, ChatMessageType). The
    /// server confirms in chat and re-sends the squelch list.
    pub fn squelch(&mut self, guid: u32, name: &str, on: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.u32(u32::from(on)).u32(guid).string16(name.trim()).u32(1);
        self.session.send_action(
            ac_net::messages::action::MODIFY_CHARACTER_SQUELCH,
            &w.finish(),
        );
    }

    pub fn squelch_account(&mut self, name: &str, on: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.u32(u32::from(on)).string16(name.trim());
        self.session.send_action(
            ac_net::messages::action::MODIFY_ACCOUNT_SQUELCH,
            &w.finish(),
        );
    }

    pub fn squelch_global(&mut self, chat_type: u32, on: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.u32(u32::from(on)).u32(chat_type);
        self.session
            .send_action(ac_net::messages::action::MODIFY_GLOBAL_SQUELCH, &w.finish());
    }

    /// Ask the server about our house (HouseQuery 0x021E): HouseData
    /// when we own one, HouseStatus when not.
    pub fn house_query(&mut self) {
        self.session
            .send_action(ac_net::messages::action::HOUSE_QUERY, &[]);
    }

    /// The carried items that cover a payment list, largest stacks
    /// first: guids of stacks of each wanted weenie until the
    /// outstanding amount is covered. `None` when something is short.
    pub fn payment_items(&self, payments: &[ac_world::housing::Payment]) -> Option<Vec<u32>> {
        let me = self.world.player_guid;
        let mut out = Vec::new();
        for p in payments {
            let mut need = p.outstanding();
            if need == 0 {
                continue;
            }
            let mut stacks: Vec<&ac_world::WorldObject> = self
                .world
                .inventory()
                .filter(|o| o.weenie_class_id == p.wcid && o.wielder != me)
                .collect();
            stacks.sort_by_key(|o| std::cmp::Reverse(o.stack_size));
            for o in stacks {
                if need == 0 {
                    break;
                }
                out.push(o.guid);
                need = need.saturating_sub(o.stack_size.max(1));
            }
            if need > 0 {
                return None;
            }
        }
        Some(out)
    }

    /// Buy the house whose sign we last used (BuyHouse 0x021C: slumlord,
    /// item guid list), paying with what the pack holds. False when the
    /// profile is missing, the house is owned, or an item is short; the
    /// server answers "Congratulations!  You now own this dwelling." and
    /// the house data, or a refusal in chat.
    pub fn buy_house(&mut self) -> bool {
        let Some(p) = self.world.house_profile.clone() else {
            return false;
        };
        if p.owner != 0 {
            return false;
        }
        let Some(items) = self.payment_items(&p.buy) else {
            tracing::info!("buy house: missing purchase items");
            return false;
        };
        tracing::info!(
            "buy house at {:#010x} with {} items",
            p.slumlord,
            items.len()
        );
        let mut w = ac_net::wire::Writer::new();
        w.u32(p.slumlord).u32(items.len() as u32);
        for g in &items {
            w.u32(*g);
        }
        self.session
            .send_action(ac_net::messages::action::BUY_HOUSE, &w.finish());
        true
    }

    /// Pay the maintenance of the house whose sign we last used
    /// (RentHouse 0x0221), with what the pack holds toward what is still
    /// outstanding. Anyone may pay; the sign must be in the landblock.
    pub fn rent_house(&mut self) -> bool {
        let Some(p) = self.world.house_profile.clone() else {
            return false;
        };
        let Some(items) = self.payment_items(&p.rent) else {
            tracing::info!("rent house: missing items");
            return false;
        };
        if items.is_empty() {
            return false;
        }
        tracing::info!(
            "pay rent at {:#010x} with {} items",
            p.slumlord,
            items.len()
        );
        let mut w = ac_net::wire::Writer::new();
        w.u32(p.slumlord).u32(items.len() as u32);
        for g in &items {
            w.u32(*g);
        }
        self.session
            .send_action(ac_net::messages::action::RENT_HOUSE, &w.finish());
        true
    }

    /// Give the house up (AbandonHouse 0x021F); the server boots
    /// everyone, clears the guest list and answers HouseStatus.
    pub fn abandon_house(&mut self) {
        self.session
            .send_action(ac_net::messages::action::ABANDON_HOUSE, &[]);
    }

    /// Ask for our house's guest list (RequestFullGuestList 0x024D);
    /// it lands in `world.house_access`.
    pub fn house_guest_list(&mut self) {
        self.session
            .send_action(ac_net::messages::action::REQUEST_FULL_GUEST_LIST, &[]);
    }

    /// Add or remove a guest by name (AddPermanentGuest 0x0245 /
    /// RemovePermanentGuest 0x0246); the server confirms in chat, and
    /// the guest list is asked for again.
    pub fn house_guest(&mut self, name: &str, add: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.string16(name.trim());
        let op = if add {
            ac_net::messages::action::ADD_PERMANENT_GUEST
        } else {
            ac_net::messages::action::REMOVE_PERMANENT_GUEST
        };
        self.session.send_action(op, &w.finish());
        self.house_guest_list();
    }

    /// Let a guest use the storage chests, or not (ChangeStoragePermission
    /// 0x0249: name, flag).
    pub fn house_storage(&mut self, name: &str, allow: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.string16(name.trim()).u32(u32::from(allow));
        self.session.send_action(
            ac_net::messages::action::CHANGE_STORAGE_PERMISSION,
            &w.finish(),
        );
        self.house_guest_list();
    }

    /// Open the house to everyone, or make it private (SetOpenHouseStatus
    /// 0x0247).
    pub fn house_open(&mut self, open: bool) {
        self.session.send_action(
            ac_net::messages::action::SET_OPEN_HOUSE_STATUS,
            &u32::from(open).to_le_bytes(),
        );
        self.house_guest_list();
    }

    /// Give or take the allegiance's access to the house, or to its
    /// storage (ModifyAllegianceGuestPermission 0x0267 /
    /// ModifyAllegianceStoragePermission 0x0268).
    pub fn house_allegiance(&mut self, storage: bool, add: bool) {
        let op = if storage {
            ac_net::messages::action::MODIFY_ALLEGIANCE_STORAGE_PERMISSION
        } else {
            ac_net::messages::action::MODIFY_ALLEGIANCE_GUEST_PERMISSION
        };
        self.session.send_action(op, &u32::from(add).to_le_bytes());
        self.house_guest_list();
    }

    /// Throw a visitor out by name (BootSpecificHouseGuest 0x024A), or
    /// everyone with an empty name (BootEveryone 0x025F).
    pub fn house_boot(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.session
                .send_action(ac_net::messages::action::BOOT_EVERYONE, &[]);
            return;
        }
        let mut w = ac_net::wire::Writer::new();
        w.string16(name);
        self.session.send_action(
            ac_net::messages::action::BOOT_SPECIFIC_HOUSE_GUEST,
            &w.finish(),
        );
    }

    /// Clear the guest list (RemoveAllPermanentGuests 0x025E) or every
    /// storage permission (RemoveAllStoragePermission 0x024C).
    pub fn house_clear(&mut self, storage_only: bool) {
        let op = if storage_only {
            ac_net::messages::action::REMOVE_ALL_STORAGE_PERMISSION
        } else {
            ac_net::messages::action::REMOVE_ALL_PERMANENT_GUESTS
        };
        self.session.send_action(op, &[]);
        self.house_guest_list();
    }

    /// Say something in a Turbine chat room (message 0xF7DE): General,
    /// Trade, LFG, Roleplay, or `turbine::ALLEGIANCE` for our
    /// allegiance's own room. Everyone in the room, us included, gets
    /// the line back. False without an allegiance room to speak in.
    pub fn turbine_say(&mut self, room: u32, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        let room = if room == ac_net::messages::turbine::ALLEGIANCE {
            if self.allegiance_room == 0 {
                self.events.push(Event::Chat {
                    text: "You are not in an allegiance.".into(),
                    kind: 0,
                });
                return false;
            }
            self.allegiance_room
        } else {
            room
        };
        let me = self.world.player_guid.unwrap_or(0);
        let context = self.turbine_context;
        self.turbine_context = (self.turbine_context % 0x70) + 1;
        let msg = ac_net::messages::turbine::encode(room, me, text, context);
        self.session
            .send_message(ac_net::messages::queue::WEENIE, msg);
        true
    }

    /// Say something on a group channel (ChatChannel 0x0147): fellowship,
    /// vassals, patron, monarch or co-vassals (`ac_net::messages::channel`).
    /// Everyone on it, us included, hears it as a ChannelBroadcast.
    pub fn chat_channel(&mut self, channel: u32, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let mut w = ac_net::wire::Writer::new();
        w.u32(channel).string16(text);
        self.session
            .send_action(ac_net::messages::action::CHAT_CHANNEL, &w.finish());
    }

    /// Ask the server for our allegiance profile (AllegianceUpdateRequest
    /// 0x001F); it answers with AllegianceUpdate. `panel` is what the
    /// original client sent when its panel opened; ACE ignores it.
    pub fn allegiance_update_request(&mut self, panel: bool) {
        self.session.send_action(
            ac_net::messages::action::ALLEGIANCE_UPDATE_REQUEST,
            &u32::from(panel).to_le_bytes(),
        );
    }

    /// Swear allegiance to a player in reach (SwearAllegiance 0x001D).
    /// The server walks us over, asks the patron (a kind 1
    /// confirmation) and, on yes, sends both an AllegianceUpdate. Refused
    /// when we already have a patron, they ignore allegiance requests,
    /// have 11 vassals, or are our own vassal.
    pub fn swear_allegiance(&mut self, patron: u32) -> bool {
        let Some(o) = self.world.objects.get(&patron) else {
            return false;
        };
        if o.object_desc_flags & ac_world::object_desc_flags::PLAYER == 0
            || Some(patron) == self.world.player_guid
        {
            return false;
        }
        tracing::info!("swear allegiance to {} ({patron:#010x})", o.name);
        self.session.send_action(
            ac_net::messages::action::SWEAR_ALLEGIANCE,
            &patron.to_le_bytes(),
        );
        true
    }

    /// Break with our patron or one of our vassals (BreakAllegiance
    /// 0x001E); they need not be online or in view.
    pub fn break_allegiance(&mut self, member: u32) -> bool {
        let known = self
            .world
            .allegiance
            .as_ref()
            .map(|a| {
                a.patron.as_ref().map(|p| p.guid) == Some(member)
                    || a.vassals.iter().any(|v| v.guid == member)
            })
            .unwrap_or(false);
        if !known {
            return false;
        }
        tracing::info!("break allegiance with {member:#010x}");
        self.session.send_action(
            ac_net::messages::action::BREAK_ALLEGIANCE,
            &member.to_le_bytes(),
        );
        true
    }

    /// Ask for another member's profile by name (AllegianceInfoRequest
    /// 0x027B; officers only). The answer lands in
    /// `world.allegiance_info`.
    pub fn allegiance_info_request(&mut self, name: &str) {
        let mut w = ac_net::wire::Writer::new();
        w.string16(name.trim());
        self.session.send_action(
            ac_net::messages::action::ALLEGIANCE_INFO_REQUEST,
            &w.finish(),
        );
    }

    /// Name the allegiance (SetAllegianceName 0x0033, monarch only) or
    /// clear the name with an empty string (ClearAllegianceName 0x0031).
    /// The server answers in chat and re-sends the profile.
    pub fn set_allegiance_name(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.session
                .send_action(ac_net::messages::action::CLEAR_ALLEGIANCE_NAME, &[]);
            return;
        }
        let mut w = ac_net::wire::Writer::new();
        w.string16(name);
        self.session
            .send_action(ac_net::messages::action::SET_ALLEGIANCE_NAME, &w.finish());
        // The server only answers in chat; ask for the renamed profile.
        self.allegiance_update_request(true);
    }

    /// Set (SetMotd 0x0254) or, with an empty string, clear (ClearMotd
    /// 0x0256) the allegiance message of the day; officers only.
    pub fn set_allegiance_motd(&mut self, motd: &str) {
        let motd = motd.trim();
        if motd.is_empty() {
            self.session
                .send_action(ac_net::messages::action::CLEAR_MOTD, &[]);
            return;
        }
        let mut w = ac_net::wire::Writer::new();
        w.string16(motd);
        self.session
            .send_action(ac_net::messages::action::SET_MOTD, &w.finish());
        self.allegiance_update_request(true);
    }

    /// Apply a carried item to a target (UseWithTarget 0x0035): a healing
    /// kit on yourself or a fellow, a mana stone on an item, a key or
    /// lockpick on a chest or door. The server walks us into reach first.
    pub fn use_on(&mut self, item: u32, target: u32) -> bool {
        use ac_net::messages::action;
        let me = self.world.player_guid;
        let Some(o) = self.world.objects.get(&item) else {
            return false;
        };
        // Anything carried will do, a side pack included: a bundle of
        // arrowheads lives in one as often as not.
        if me.is_none() || (!self.world.is_carried(item) && o.wielder != me) {
            return false;
        }
        let what = o.name.clone();
        let who = self
            .world
            .objects
            .get(&target)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        tracing::info!("use {what} ({item:#010x}) on {who} ({target:#010x})");
        self.last_used = Some(target);
        let mut w = ac_net::wire::Writer::new();
        w.u32(item).u32(target);
        self.session
            .send_action(action::USE_WITH_TARGET, &w.finish());
        true
    }

    /// Ask another player to trade (OpenTradeNegotiations 0x01F6). Both
    /// must be in peace mode and close by; the server answers with
    /// RegisterTrade for both (`world.trade`).
    pub fn open_trade(&mut self, player: u32) {
        self.interrupt_travel("trading");
        use ac_net::messages::action;
        tracing::info!("trade with {player:#010x}");
        self.session
            .send_action(action::OPEN_TRADE_NEGOTIATIONS, &player.to_le_bytes());
    }

    /// Put a carried item in the trade window (AddToTrade 0x01F8).
    pub fn add_to_trade(&mut self, item: u32) -> bool {
        use ac_net::messages::action;
        let Some(t) = self.world.trade.as_ref() else {
            return false;
        };
        let slot = t.mine.len() as u32;
        let mut w = ac_net::wire::Writer::new();
        w.u32(item).u32(slot);
        self.session.send_action(action::ADD_TO_TRADE, &w.finish());
        true
    }

    /// Accept the offers as they stand (AcceptTrade 0x01FA: partner, the
    /// trade stamp, status, initiator, both acceptance flags; the server
    /// only cares that it arrived).
    pub fn accept_trade(&mut self) {
        let Some(t) = self.world.trade.as_ref() else {
            return;
        };
        let me = self.world.player_guid.unwrap_or(0);
        let (i_am_initiator, they) = (t.initiator == me, t.they_accepted);
        let mut w = ac_net::wire::Writer::new();
        w.u32(t.partner)
            .f64(t.stamp as f64)
            .u32(0)
            .u32(t.initiator)
            .u32(u32::from(if i_am_initiator { true } else { they }))
            .u32(u32::from(if i_am_initiator { they } else { true }));
        self.session
            .send_action(ac_net::messages::action::ACCEPT_TRADE, &w.finish());
    }

    pub fn decline_trade(&mut self) {
        self.session
            .send_action(ac_net::messages::action::DECLINE_TRADE, &[]);
    }

    /// Take everything back out of the window (ResetTrade 0x0204).
    pub fn reset_trade(&mut self) {
        self.session
            .send_action(ac_net::messages::action::RESET_TRADE, &[]);
    }

    pub fn close_trade(&mut self) {
        self.session
            .send_action(ac_net::messages::action::CLOSE_TRADE_NEGOTIATIONS, &[]);
        self.world.trade = None;
    }

    /// Stop looking into the open ground container.
    pub fn close_container(&mut self) {
        use ac_net::messages::action;
        if let Some((c, _)) = self.world.open_container.take() {
            self.session
                .send_action(action::NO_LONGER_VIEWING_CONTENTS, &c.to_le_bytes());
        }
    }

    /// Buy one of a vendor's stock items.
    pub fn buy(&mut self, guid: u32) {
        self.buy_amount(guid, 1);
    }

    /// Buy `amount` of a vendor's stock item (a stack for stackables).
    pub fn buy_amount(&mut self, guid: u32, amount: u32) {
        use ac_net::messages::{action, trade};
        let Some(vendor) = self.world.open_vendor.as_ref().map(|v| v.vendor) else {
            return;
        };
        if amount == 0 {
            return;
        }
        tracing::info!("buy {amount} x {guid:#010x} from {vendor:#010x}");
        self.session
            .send_action(action::BUY, &trade(vendor, &[(guid, amount as i32)]));
        // The counter answers with `UseDone`; until it does, what was handed
        // over is still in the pack, and reading that as a refusal is how a
        // run left a counter before its sale landed.
        self.autoplay.cast_sent = Some(Instant::now());
    }

    /// Sell several pack items (each its whole stack) to the open
    /// vendor in one go.
    ///
    /// The sell action has always carried a list and the server has
    /// always handled one; sending them singly was a round trip per
    /// dagger, and a round trip is where a run finds new ways to lose
    /// track of what it has offered.
    pub fn sell_many(&mut self, guids: &[u32]) {
        use ac_net::messages::{action, trade};
        let Some(vendor) = self.world.open_vendor.as_ref().map(|v| v.vendor) else {
            return;
        };
        let lot: Vec<(u32, i32)> = guids
            .iter()
            .filter_map(|g| {
                let o = self.world.objects.get(g)?;
                Some((*g, o.stack_size.max(1) as i32))
            })
            .collect();
        if lot.is_empty() {
            return;
        }
        tracing::info!("sell {} item(s) to {vendor:#010x}", lot.len());
        self.session.send_action(action::SELL, &trade(vendor, &lot));
        // The counter answers with `UseDone`; until it does, what was handed
        // over is still in the pack, and reading that as a refusal is how a
        // run left a counter before its sale landed.
        self.autoplay.cast_sent = Some(Instant::now());
    }

    /// Sell a pack item (its whole stack) to the open vendor.
    pub fn sell(&mut self, guid: u32) {
        use ac_net::messages::{action, trade};
        let Some(vendor) = self.world.open_vendor.as_ref().map(|v| v.vendor) else {
            return;
        };
        let amount = self
            .world
            .objects
            .get(&guid)
            .map(|o| o.stack_size.max(1) as i32)
            .unwrap_or(1);
        tracing::info!("sell {guid:#010x} to {vendor:#010x}");
        self.session
            .send_action(action::SELL, &trade(vendor, &[(guid, amount)]));
        // The counter answers with `UseDone`; until it does, what was handed
        // over is still in the pack, and reading that as a refusal is how a
        // run left a counter before its sale landed.
        self.autoplay.cast_sent = Some(Instant::now());
    }

    pub fn close_vendor(&mut self) {
        self.world.open_vendor = None;
    }

    /// A chat line starting with `/`: the retail client's own commands
    /// become their game actions; anything else goes to the server as an
    /// `@command` (ACE runs its command manager on those and answers
    /// "Unknown command" for the rest). Returns false for an empty line.
    pub fn slash_command(&mut self, line: &str) -> bool {
        use ac_net::messages::action;
        let body = line.trim_start_matches(['/', '@']).trim();
        if body.is_empty() {
            return false;
        }
        let (name, args) = body
            .split_once(char::is_whitespace)
            .map(|(n, a)| (n, a.trim()))
            .unwrap_or((body, ""));
        let name = name.to_ascii_lowercase();
        // A logout the player asked for is a session ending on purpose,
        // however the server ends up ending it: never reconnected (see
        // [`reconnect`]). The command itself is passed on as a server
        // command like any other.
        if matches!(name.as_str(), "logout" | "logoff" | "quit" | "exit") {
            self.quitting = true;
        }
        let mut w = ac_net::wire::Writer::new();
        match name.as_str() {
            "lifestone" | "ls" => self.session.send_action(action::TELE_TO_LIFESTONE, &[]),
            "die" => self.session.send_action(action::DIE, &[]),
            "house" | "home" => self.session.send_action(action::TELE_TO_HOUSE, &[]),
            "mansion" | "mp" => self.session.send_action(action::TELE_TO_MANSION, &[]),
            "hometown" => self
                .session
                .send_action(action::RECALL_ALLEGIANCE_HOMETOWN, &[]),
            "marketplace" | "mkt" => self.session.send_action(action::TELE_TO_MARKETPLACE, &[]),
            "pklite" => self.session.send_action(action::ENTER_PK_LITE, &[]),
            "afk" => {
                if !args.is_empty() {
                    w.string16(args);
                    self.session
                        .send_action(action::SET_AFK_MESSAGE, &w.finish());
                    w = ac_net::wire::Writer::new();
                }
                w.u32(1);
                self.session.send_action(action::SET_AFK_MODE, &w.finish());
            }
            "back" => {
                w.u32(0);
                self.session.send_action(action::SET_AFK_MODE, &w.finish());
            }
            "tell" | "t" => {
                // `/tell Name, message` or `/tell Name message`.
                let (target, msg) = match args.split_once(',') {
                    Some((t, m)) => (t.trim(), m.trim()),
                    None => args
                        .split_once(char::is_whitespace)
                        .map(|(t, m)| (t.trim(), m.trim()))
                        .unwrap_or((args, "")),
                };
                if target.is_empty() || msg.is_empty() {
                    return false;
                }
                w.string16(msg).string16(target);
                self.session.send_action(action::TELL, &w.finish());
            }
            "emote" | "e" | "me" => {
                w.string16(args);
                self.session.send_action(action::EMOTE, &w.finish());
            }
            // `/wave`, `/bow`...: the soul emotes (retail typed them as
            // `*wave*`; both work).
            n if emotes::lookup(n).is_some() && args.is_empty() => {
                self.emote(n);
            }
            // `/g`, `/trade`, `/lfg`, `/rp`, `/a`: the Turbine chat rooms.
            n if ac_net::messages::turbine::from_prefix(n).is_some() => {
                if args.is_empty() {
                    return false;
                }
                let room = ac_net::messages::turbine::from_prefix(n).unwrap_or(0);
                return self.turbine_say(room, args);
            }
            // `/v`, `/p`, `/m`, `/c`, `/f`: vassals, patron, monarch,
            // co-vassals and fellowship group chat.
            n if ac_net::messages::channel::from_prefix(n).is_some() => {
                if args.is_empty() {
                    return false;
                }
                let channel = ac_net::messages::channel::from_prefix(n).unwrap_or(0);
                self.chat_channel(channel, args);
            }
            _ => {
                // The server's own commands (@acehelp, @myquests, admin
                // commands...): ACE reads them from Talk with an @ prefix.
                w.string16(&format!("@{body}"));
                self.session.send_action(action::TALK, &w.finish());
            }
        }
        true
    }

    pub fn say(&mut self, text: &str) {
        if emotes::from_chat_line(text).is_some() && self.emote(text) {
            return;
        }
        let mut w = ac_net::wire::Writer::new();
        w.string16(text);
        self.session
            .send_action(ac_net::messages::action::TALK, &w.finish());
    }

    /// Select an object (what the target bar and appraisal refer to).
    pub fn select(&mut self, guid: Option<u32>) {
        if guid != self.selected {
            self.previous_selected = self.selected;
        }
        self.selected = guid;
    }

    /// The thing a targetable item is applied to when used: the most
    /// recently selected object other than the item itself (selecting
    /// the item to use it must not lose the target picked before).
    pub fn use_target(&self, item: u32) -> Option<u32> {
        [self.selected, self.previous_selected]
            .into_iter()
            .flatten()
            .find(|t| *t != item && self.world.objects.contains_key(t))
    }

    /// Events produced since the last drain (autoplay's status changes
    /// included, see `Event::Autoplay`).
    pub fn drain_events(&mut self) -> Vec<Event> {
        self.events.append(&mut self.autoplay.announced);
        std::mem::take(&mut self.events)
    }
}

/// The chat line for a salvage result: "You obtain 3 Oak (workmanship
/// 8.00) using your Salvaging skill." per material.
pub fn salvage_text(res: &ac_net::messages::SalvageResult) -> String {
    let skill = match res.skill {
        40 => "Salvaging",
        18 => "Item Tinkering",
        28 => "Weapon Tinkering",
        29 => "Armor Tinkering",
        30 => "Magic Item Tinkering",
        _ => "salvaging",
    };
    if res.yields.is_empty() {
        return format!("You salvaged nothing using your {skill} skill.");
    }
    let parts: Vec<String> = res
        .yields
        .iter()
        .map(|y| {
            format!(
                "{} {} (workmanship {:.2})",
                y.units,
                ac_world::material::name(y.material),
                y.workmanship
            )
        })
        .collect();
    let mut text = format!("You obtain {} using your {skill} skill.", parts.join(", "));
    if res.bonus_percent > 0 {
        text.push_str(&format!(" ({}% from augmentations)", res.bonus_percent));
    }
    if !res.skipped.is_empty() {
        text.push_str(&format!(
            " {} item(s) could not be salvaged.",
            res.skipped.len()
        ));
    }
    text
}

#[cfg(test)]
mod salvage_tests {
    use super::*;
    use ac_net::messages::{SalvageResult, SalvageYield};

    #[test]
    fn salvage_text_lists_materials() {
        let res = SalvageResult {
            skill: 40,
            skipped: vec![1],
            yields: vec![
                SalvageYield {
                    material: 0x4B,
                    workmanship: 8.0,
                    units: 3,
                },
                SalvageYield {
                    material: 0x3D,
                    workmanship: 5.5,
                    units: 1,
                },
            ],
            bonus_percent: 0,
        };
        assert_eq!(
            salvage_text(&res),
            "You obtain 3 Oak (workmanship 8.00), 1 Iron (workmanship 5.50) using your Salvaging skill. 1 item(s) could not be salvaged."
        );
        let mut w = ac_net::wire::Writer::new();
        w.u32(28).u32(0).u32(1).u32(0x40).f64(3.0).u32(2).u32(0);
        let parsed = SalvageResult::parse(&w.finish()).unwrap();
        assert_eq!(parsed.skill, 28);
        assert_eq!(parsed.yields[0].units, 2);
        assert_eq!(
            salvage_text(&parsed),
            "You obtain 2 Steel (workmanship 3.00) using your Weapon Tinkering skill."
        );
    }
}
