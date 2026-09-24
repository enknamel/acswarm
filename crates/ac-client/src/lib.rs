//! A headless game session: the connection, the world the server describes,
//! our character, and the gameplay commands a UI or a script can issue.
//! Nothing here renders; several `Client`s can live in one process.

#![warn(unreachable_pub)]

pub mod academy;
pub mod action;
mod actions;
pub use actions::items::salvage_text;
pub(crate) use actions::items::TakeSent;
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
pub mod tally;
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
    /// What `/log` asks of the front end: copy the chat to this file
    /// from now on, or stop with None. The chat window is the front
    /// end's, and only it knows what it has shown.
    ChatToFile(Option<String>),
    /// What `/clear` asks of it: empty the chat window, or every one.
    ChatClear {
        all: bool,
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
    /// A line autoplay logged beside its status ("town run done: sold 3 item(s)"), each once a
    /// while at most: the record telemetry keeps of what the status line passed over.
    Noted(String),
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
    /// This frame's walk, while one is steered: for telemetry.
    pub walk_frame: Option<tally::WalkFrame>,
    /// Blows traded this session: for telemetry.
    pub blows: tally::Blows,
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
    /// Our society's Turbine chat room (the seventh id of the same
    /// message, ACE GameEventSetTurbineChatChannels.cs:16), 0 without one.
    pub society_room: u32,
    /// The last player who told us, for `/reply`: their guid and name.
    pub last_teller: Option<(u32, String)>,
    /// The last player we told, for `/retell`; only `/tell` sets it.
    pub last_told: Option<String>,
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
    /// Whether the window shows the frame rate, which `/framerate` flips.
    /// It starts on, as acswarm's status line has always shown it.
    pub show_framerate: bool,
}

impl Client {
    /// The character is in the world and our physics body exists.
    pub fn placed(&self) -> bool {
        self.scene_block.is_some()
    }
}

impl Client {
    /// Events produced since the last drain (autoplay's status changes
    /// included, see `Event::Autoplay`).
    pub fn drain_events(&mut self) -> Vec<Event> {
        self.events.append(&mut self.autoplay.announced);
        std::mem::take(&mut self.events)
    }
}
