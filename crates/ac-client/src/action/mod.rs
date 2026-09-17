//! What a front end asks the character to do, in one vocabulary: the chat box,
//! a key, a Rhai call, the bus, the CLI and a panel button all build an
//! [`Action`] and hand it to [`Client::act`]. `act` carries external intent
//! only -- autoplay decides for itself and calls the same `Client` methods
//! directly, as does every read.

use ac_net::messages::{channel, turbine};
use serde::{Deserialize, Serialize};

use crate::autoplay::Role;
use crate::Client;

mod allegiance;
mod chat;
mod combat;
mod fellow;
mod item;
mod magic;
mod recall;
pub mod resolve;
mod status;
mod team;
mod trade;

pub use status::{Friends, HouseKind};

/// Why an action did not happen: the words a front end shows, and the server's
/// own code when it gave one (the vocabulary every system refuses in).
pub type Refused = ac_agent::did::Because;

/// What came of an [`Action`]: the request went out, or it was refused here
/// before it went. The server's own answer arrives later, as an `Event`.
pub type Outcome = Result<(), Refused>;

/// One thing `@allegiance` asks of the server, a subcommand each. The ranks
/// they want are the server's to check (ACE `Player_Allegiance.cs:1478-1515`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Manage {
    /// Throw a member out, by character or by every character on the account.
    Boot { name: String, account: bool },
    /// Another member's profile, by name.
    Info(String),
    /// Put somebody out of allegiance chat, with the reason they are told.
    ChatBoot { name: String, reason: String },
    /// Silence a member in allegiance chat for five minutes, or let them back.
    Gag { name: String, gagged: bool },
    /// Who is banned from the allegiance.
    BanList,
    /// Put a name on the ban list, or take it off.
    Ban { name: String, banned: bool },
    /// Who the officers are.
    OfficerList,
    /// Make a member an officer of level 1 to 3.
    OfficerSet { name: String, level: u32 },
    /// Take a member out of the officers.
    OfficerRemove(String),
    /// Leave the allegiance with no officers at all.
    OfficerClear,
    /// The titles the three officer levels carry.
    TitleList,
    /// Name an officer level (1 to 3).
    TitleSet { level: u32, title: String },
    /// Back to the titles the server names them by.
    TitleClear,
    /// The allegiance's message of the day, as it stands.
    Motd,
    /// Set the message of the day.
    SetMotd(String),
    /// Leave the allegiance with no message of the day.
    ClearMotd,
    /// The allegiance's name, as it stands.
    Name,
    /// Name the allegiance.
    SetName(String),
    /// Leave the allegiance unnamed.
    ClearName,
    /// Lock the allegiance against new vassals, unlock it, or ask.
    Lock(Lock),
    /// Let one name swear in while the allegiance is locked.
    ApproveVassal(String),
    /// The monarch's house, and what the allegiance may do in it.
    House(HouseAccess),
}

/// What `@allegiance lock` asks (ACE `AllegianceLockAction.cs:5-11`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lock {
    Off,
    On,
    Toggle,
    Check,
    /// The names allowed in while it is locked.
    Approved,
    /// Let none of them in any more.
    ClearApproved,
}

/// What `@allegiance house` asks (ACE `AllegianceHouseAction.cs:8-14`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HouseAccess {
    /// What the allegiance may do there, as it stands.
    Check,
    GuestOpen,
    GuestClose,
    StorageOpen,
    StorageClose,
}

/// What a thing is named by. A front end that already has a guid sends one;
/// one with a word the player typed sends [`Target::Name`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Target {
    /// The object with this guid.
    Guid(u32),
    /// The nearest thing named this, what is carried counting as nearest.
    Name(String),
    /// Our own character.
    Me,
    /// What the target bar and appraisal refer to.
    Selected,
    /// The corpse of the last creature fought.
    LastCorpse,
}

/// What a spell is named by: its id, or a name to look up in the spellbook.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpellRef {
    Id(u32),
    Name(String),
}

/// One thing a front end asks for. Every variant is carried out by
/// [`Client::act`] and belongs to one family, the module named beside it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    // ---- chat ----
    /// Say it aloud, to everyone in earshot.
    Say(String),
    /// Tell one player, wherever they are.
    Tell {
        who: String,
        text: String,
    },
    /// A freeform emote ("/emote grins"), said as an action of ours.
    Emote(String),
    /// A soul emote by word ("wave", "*bow*"): the motion and its line.
    SoulEmote(String),
    /// Speak in a Turbine chat room (`ac_net::messages::turbine`).
    Room {
        room: u32,
        text: String,
    },
    /// Speak on a group channel (`ac_net::messages::channel`).
    Channel {
        channel: u32,
        text: String,
    },
    /// Tell the last player who told us.
    Reply(String),
    /// Tell again the last player we told.
    Retell(String),
    /// Away from keyboard, with the message others get when they tell us.
    Afk {
        away: bool,
        message: String,
    },
    /// Stop hearing one player, or their whole account, on one chat
    /// category (`ac_net::messages::squelch`); `who` None is the last
    /// player who told us. `on` false hears them again.
    Squelch {
        who: Option<String>,
        account: bool,
        kind: u32,
        on: bool,
    },
    /// Stop hearing one chat category from everyone, which is what
    /// `/filter`, `/chat` and `/notell` each ask for.
    Filter {
        kind: u32,
        on: bool,
    },
    /// Show whom we squelch, or the categories we filter from everyone.
    ShowSquelches {
        global: bool,
    },
    /// Show the categories a squelch or a filter can name.
    ShowMessageTypes,
    /// Join a chat channel, leave it, or ask who is on it
    /// (`ac_net::messages::channel`); these are the staff channels, and
    /// ACE answers everyone else with silence.
    ChannelJoin(u32),
    ChannelLeave(u32),
    ChannelList(u32),
    /// Ask which channels we may join.
    ChannelIndex,
    /// Copy the chat to this file from now on, or stop with None.
    ChatToFile(Option<String>),
    /// Empty the chat window, or every one of them.
    ChatClear {
        all: bool,
    },
    /// Rename the chat window the line was typed in; not the character's
    /// title, which `Client::set_title` shows.
    ChatTitle(String),
    /// A command for the server itself, sent as its `@command` line.
    ServerCommand(String),

    // ---- item ----
    /// Use it: wield it, pick it up, open it, talk to it (what a
    /// double-click did).
    Use(Target),
    /// Apply one carried thing to another (a kit, a stone, a key).
    UseOn {
        item: Target,
        target: Target,
    },
    /// Take it out of the open container.
    Take(Target),
    /// Take everything in the open container.
    TakeAll,
    /// Put it on the ground.
    Drop(Target),
    /// Hand it to somebody.
    Give {
        item: Target,
        to: Target,
        amount: Option<u32>,
    },
    /// Move it into a pack.
    PutIn {
        item: Target,
        container: Target,
    },
    /// Take `amount` off a stack, into the pack.
    Split {
        item: Target,
        amount: u32,
    },
    /// Pour one stack into another.
    Merge {
        from: Target,
        to: Target,
    },
    /// Break these down at the Ust.
    Salvage(Vec<Target>),
    /// Ask the server what it is.
    Appraise(Target),
    /// Shut the open container.
    CloseContainer,

    // ---- combat ----
    /// Attack it, entering combat first.
    Attack(Target),
    /// Stop the attack in progress; combat mode stays on.
    CancelAttack,
    /// Weapons out or away.
    Combat(bool),
    /// Wield it, whatever it is held in.
    Wield(Target),
    /// Select it, or nothing.
    Select(Option<Target>),

    // ---- magic ----
    /// Cast a spell, on `at` or on whatever is selected.
    Cast {
        spell: SpellRef,
        at: Option<Target>,
    },
    /// Buy the components the spellbook burns, at the open counter.
    FillComponents,

    // ---- trade ----
    /// Open a secure trade with a player.
    TradeOpen(Target),
    /// Put a carried thing on our side of the trade.
    TradeAdd(Target),
    /// Accept the trade as it stands.
    TradeAccept,
    /// Refuse the trade.
    TradeDecline,
    /// Take everything back off the table.
    TradeReset,
    /// Shut the trade window.
    TradeClose,
    /// Buy from the open counter.
    Buy {
        ware: Target,
        amount: u32,
    },
    /// Sell to the open counter.
    Sell(Target),
    /// Leave the counter.
    CloseVendor,

    // ---- fellow ----
    /// Found a fellowship.
    FellowCreate {
        name: String,
        share_xp: bool,
    },
    /// Invite a player into ours.
    FellowRecruit(Target),
    /// Put a fellow out of it.
    FellowDismiss(Target),
    /// Leave ours, disbanding it if we lead.
    FellowQuit {
        disband: bool,
    },
    /// Answer the question the server asked (a recruit, an allegiance).
    Confirm(bool),

    // ---- allegiance ----
    /// Manage the allegiance: one of retail's `@allegiance` subcommands.
    Allegiance(Manage),
    /// Hear a Turbine chat room (`ac_net::messages::turbine`), or stop.
    Listen {
        room: u32,
        on: bool,
    },

    // ---- team ----
    /// Play on its own.
    Autoplay(bool),
    /// Hunt with the other characters being played.
    Team(bool),
    /// Lead them.
    TeamLead(bool),
    /// What this character does for the others.
    TeamRole(Role),
    /// Follow the leader.
    Follow(bool),
    /// Fight what comes.
    Fight(bool),

    // ---- recall ----
    /// A recall the server makes for us.
    RecallLifestone,
    /// To our own cottage or mansion.
    RecallHouse,
    /// To the allegiance's mansion.
    RecallMansion,
    /// To the Marketplace.
    RecallMarketplace,
    /// To the allegiance's hometown.
    RecallHometown,
    /// Die where we stand.
    Die,
    /// Enter PK Lite.
    PkLite,

    // ---- status ----
    /// What the endurance attribute is for.
    Endurance,
    /// Where the body stands, as a cell id and a frame.
    Location,
    /// Which client this is, and whether Turbine chat is on.
    Version,
    /// Show the frame rate, or stop.
    Framerate,
    /// Daylight outdoors whatever the hour, or the world's own time.
    Daylight,

    /// Ask how long this character has been played.
    QueryAge,
    /// Ask what day it was made.
    QueryBirth,

    /// The friends list: show it, add to it, take from it.
    Friends(Friends),

    /// Ask how many dwellings of a kind are for sale, and where.
    HousesAvailable(HouseKind),
}

impl Client {
    /// Carry out one thing a front end asked for. `Ok` means the request went
    /// out, not that the server took it: its answer comes back as an `Event`.
    pub fn act(&mut self, action: Action) -> Outcome {
        match action {
            Action::Say(text) => chat::say(self, &text),
            Action::Tell { who, text } => chat::tell(self, &who, &text),
            Action::Emote(text) => chat::emote(self, &text),
            Action::SoulEmote(words) => chat::soul_emote(self, &words),
            Action::Room { room, text } => chat::room(self, room, &text),
            Action::Channel { channel, text } => chat::channel(self, channel, &text),
            Action::Reply(text) => chat::reply(self, &text),
            Action::Retell(text) => chat::retell(self, &text),
            Action::Afk { away, message } => chat::afk(self, away, &message),
            Action::Squelch {
                who,
                account,
                kind,
                on,
            } => chat::squelch(self, who.as_deref(), account, kind, on),
            Action::Filter { kind, on } => chat::filter(self, kind, on),
            Action::ShowSquelches { global } => chat::show_squelches(self, global),
            Action::ShowMessageTypes => chat::show_message_types(self),
            Action::ChannelJoin(id) => chat::channel_join(self, id),
            Action::ChannelLeave(id) => chat::channel_leave(self, id),
            Action::ChannelList(id) => chat::channel_list(self, id),
            Action::ChannelIndex => chat::channel_index(self),
            Action::ChatToFile(file) => chat::chat_to_file(self, file.as_deref()),
            Action::ChatClear { all } => chat::chat_clear(self, all),
            Action::ChatTitle(name) => chat::chat_title(self, &name),
            Action::ServerCommand(line) => chat::server_command(self, &line),

            Action::Use(t) => item::use_thing(self, &t),
            Action::UseOn { item: i, target } => item::use_on(self, &i, &target),
            Action::Take(t) => item::take(self, &t),
            Action::TakeAll => item::take_all(self),
            Action::Drop(t) => item::drop_thing(self, &t),
            Action::Give {
                item: i,
                to,
                amount,
            } => item::give(self, &i, &to, amount),
            Action::PutIn { item: i, container } => item::put_in(self, &i, &container),
            Action::Split { item: i, amount } => item::split(self, &i, amount),
            Action::Merge { from, to } => item::merge(self, &from, &to),
            Action::Salvage(items) => item::salvage(self, &items),
            Action::Appraise(t) => item::appraise(self, &t),
            Action::CloseContainer => item::close_container(self),

            Action::Attack(t) => combat::attack(self, &t),
            Action::CancelAttack => combat::cancel_attack(self),
            Action::Combat(on) => combat::combat(self, on),
            Action::Wield(t) => combat::wield(self, &t),
            Action::Select(t) => combat::select(self, t.as_ref()),

            Action::Cast { spell, at } => magic::cast(self, &spell, at.as_ref()),
            Action::FillComponents => magic::fill_components(self),

            Action::TradeOpen(t) => trade::open(self, &t),
            Action::TradeAdd(t) => trade::add(self, &t),
            Action::TradeAccept => trade::accept(self),
            Action::TradeDecline => trade::decline(self),
            Action::TradeReset => trade::reset(self),
            Action::TradeClose => trade::close(self),
            Action::Buy { ware, amount } => trade::buy(self, &ware, amount),
            Action::Sell(t) => trade::sell(self, &t),
            Action::CloseVendor => trade::close_vendor(self),

            Action::FellowCreate { name, share_xp } => fellow::create(self, &name, share_xp),
            Action::FellowRecruit(t) => fellow::recruit(self, &t),
            Action::FellowDismiss(t) => fellow::dismiss(self, &t),
            Action::FellowQuit { disband } => fellow::quit(self, disband),
            Action::Confirm(yes) => fellow::confirm(self, yes),

            Action::Allegiance(what) => allegiance::manage(self, &what),
            Action::Listen { room, on } => allegiance::listen(self, room, on),

            Action::Autoplay(on) => team::autoplay(self, on),
            Action::Team(on) => team::team(self, on),
            Action::TeamLead(on) => team::lead(self, on),
            Action::TeamRole(role) => team::role(self, role),
            Action::Follow(on) => team::follow(self, on),
            Action::Fight(on) => team::fight(self, on),

            Action::RecallLifestone => recall::lifestone(self),
            Action::RecallHouse => recall::house(self),
            Action::RecallMansion => recall::mansion(self),
            Action::RecallMarketplace => recall::marketplace(self),
            Action::RecallHometown => recall::hometown(self),
            Action::Die => recall::die(self),
            Action::PkLite => recall::pk_lite(self),

            Action::Endurance => status::endurance(self),
            Action::Location => status::location(self),
            Action::Version => status::version(self),
            Action::Framerate => status::framerate(self),
            Action::Daylight => status::daylight(self),

            Action::QueryAge => status::age(self),
            Action::QueryBirth => status::birth(self),

            Action::Friends(what) => status::friends(self, &what),

            Action::HousesAvailable(kind) => status::houses_available(self, kind),
        }
    }

    /// The one way in for a line the player typed, in this order: the retail
    /// command table; the plugin and script hooks, which the caller runs on a
    /// [`Line::Offer`] because only it holds them; the server, for a line
    /// beginning `@` or an unknown `/`; else the words are said aloud.
    pub fn chat_line(&mut self, line: &str) -> Line {
        let line = line.trim();
        // Retail's own help: "You may substitute a forward slash (/) for the
        // at symbol (@)", so both prefixes take the same way through.
        let Some(rest) = line.strip_prefix(['/', '@']) else {
            return Line::Acted(self.act(Action::Say(line.to_string())));
        };
        let body = rest.trim();
        if body.is_empty() {
            return Line::Acted(Err(Refused::ours("no command")));
        }
        let (name, args) = body
            .split_once(char::is_whitespace)
            .map(|(n, a)| (n, a.trim()))
            .unwrap_or((body, ""));
        let name = name.to_ascii_lowercase();
        if let Some(cmd) = command(&name) {
            return Line::Acted(match (cmd.action)(args) {
                Some(a) => self.act(a),
                None => Err(Refused::ours(format!("{}: {}", name, cmd.usage))),
            });
        }
        Line::Offer {
            unclaimed: unclaimed(&name, args, body),
            name,
            args: args.to_string(),
        }
    }
}

/// What a typed line came to. The caller runs the hooks itself, since only it
/// holds the plugins and the scripts.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// Done with: a retail command acted, or the words were said aloud.
    Acted(Outcome),
    /// A `/name args` the table has no row for: offer it to the plugins and
    /// scripts, and when none takes it, `act(unclaimed)`.
    Offer {
        name: String,
        args: String,
        unclaimed: Action,
    },
}

/// One command the retail client registered: its own name and aliases, and
/// what a typed line means. A family fills rows in as it lands; the names with
/// no row yet are in [`PENDING`].
pub struct Command {
    /// Retail's name first, then retail's own aliases, lowercase.
    pub names: &'static [&'static str],
    /// The trimmed argument text as an action, `None` when it makes no sense.
    pub action: fn(&str) -> Option<Action>,
    /// What to type, for the line shown when `action` says `None`.
    pub usage: &'static str,
}

/// The retail commands acswarm answers itself. Never a name retail did not
/// register: a command of ours is a key, a panel or a script (see the
/// project's rules).
pub const RETAIL: &[Command] = &[
    Command {
        names: &["tell", "t", "send", "whisper", "w"],
        action: |args| {
            // `/tell Name, message`, and `/tell Name message` as well.
            let (who, text) = match args.split_once(',') {
                Some((w, t)) => (w.trim(), t.trim()),
                None => args
                    .split_once(char::is_whitespace)
                    .map(|(w, t)| (w.trim(), t.trim()))
                    .unwrap_or((args, "")),
            };
            (!who.is_empty() && !text.is_empty()).then(|| Action::Tell {
                who: who.to_string(),
                text: text.to_string(),
            })
        },
        usage: "/tell NAME, message",
    },
    Command {
        names: &["emote", "e", "em", "me"],
        action: |args| (!args.is_empty()).then(|| Action::Emote(args.to_string())),
        usage: "/emote what you are doing",
    },
    Command {
        names: &["afk"],
        action: |args| {
            Some(Action::Afk {
                away: true,
                message: args.to_string(),
            })
        },
        usage: "/afk [message]",
    },
    Command {
        names: &["lifestone", "lif", "ls"],
        action: |_| Some(Action::RecallLifestone),
        usage: "/lifestone",
    },
    Command {
        names: &["house", "hou"],
        action: |_| Some(Action::RecallHouse),
        usage: "/house",
    },
    Command {
        names: &["marketplace", "mar", "mp"],
        action: |_| Some(Action::RecallMarketplace),
        usage: "/marketplace",
    },
    Command {
        names: &["die"],
        action: |_| Some(Action::Die),
        usage: "/die",
    },
    Command {
        names: &["pklite", "pkl"],
        action: |_| Some(Action::PkLite),
        usage: "/pklite",
    },
    // Each family below fills in its own block, so two families landing at
    // once do not meet in the same lines. Retail's name first in every row,
    // then retail's own aliases, and never a name retail did not register.
    //
    // -- channels and speech (chat, say, the group channels, filtering) --
    Command {
        names: &["say", "s"],
        action: |args| (!args.is_empty()).then(|| Action::Say(args.to_string())),
        usage: "/say what to say aloud",
    },
    Command {
        names: &["reply", "r", "rp"],
        action: |args| (!args.is_empty()).then(|| Action::Reply(args.to_string())),
        usage: "/reply what to tell whoever told us last",
    },
    Command {
        names: &["retell", "rt"],
        action: |args| (!args.is_empty()).then(|| Action::Retell(args.to_string())),
        usage: "/retell what to tell whoever we told last",
    },
    Command {
        names: &["chat"],
        action: |args| {
            // "@chat on" is speech we want, so the squelch comes off.
            let on = chat::on_or_off(args, true)?;
            Some(Action::Filter {
                kind: ac_net::messages::squelch::SPEECH,
                on: !on,
            })
        },
        usage: "/chat on|off",
    },
    Command {
        names: &["notell"],
        action: |args| {
            // And "@notell on" is tells we do not want, so it goes on.
            Some(Action::Filter {
                kind: ac_net::messages::squelch::TELL,
                on: chat::on_or_off(args, false)?,
            })
        },
        usage: "/notell on|off",
    },
    // The Turbine rooms. `/a` says the same as `/guild` and `/gu`, and
    // reaches the room through the router's fallback until the
    // allegiance family takes it.
    Command {
        names: &["guild", "gu"],
        action: |args| chat::room_line(ac_net::messages::turbine::ALLEGIANCE, args),
        usage: "/guild what to say to the allegiance",
    },
    Command {
        names: &["general", "cg"],
        action: |args| chat::room_line(ac_net::messages::turbine::GENERAL, args),
        usage: "/general what to say on the General channel",
    },
    Command {
        names: &["trade", "ct"],
        action: |args| chat::room_line(ac_net::messages::turbine::TRADE, args),
        usage: "/trade what to say on the Trade channel",
    },
    Command {
        names: &["lfg", "clfg"],
        action: |args| chat::room_line(ac_net::messages::turbine::LFG, args),
        usage: "/lfg what to say on the LFG channel",
    },
    Command {
        names: &["roleplay", "crp"],
        action: |args| chat::room_line(ac_net::messages::turbine::ROLEPLAY, args),
        usage: "/roleplay what to say on the Roleplay channel",
    },
    Command {
        names: &["society", "soc"],
        action: |args| chat::room_line(ac_net::messages::turbine::SOCIETY, args),
        usage: "/society what to say to the society",
    },
    Command {
        names: &["olthoi", "o"],
        action: |args| chat::room_line(ac_net::messages::turbine::OLTHOI, args),
        usage: "/olthoi what to say on the Olthoi channel",
    },
    // Squelching and filtering.
    Command {
        names: &["squelch"],
        action: |args| chat::squelch_line(args, true),
        usage: "/squelch [-account] [-CATEGORY] NAME, or /squelch -reply",
    },
    Command {
        names: &["unsquelch"],
        action: |args| chat::squelch_line(args, false),
        usage: "/unsquelch [-account] [-CATEGORY] NAME, or /unsquelch -reply",
    },
    Command {
        names: &["filter"],
        action: |args| chat::filter_line(args, true),
        usage: "/filter -CATEGORY, or /filter for what is filtered",
    },
    Command {
        names: &["unfilter"],
        action: |args| chat::filter_line(args, false),
        usage: "/unfilter -CATEGORY, or /unfilter for what is filtered",
    },
    Command {
        names: &["messagetypes", "message_types", "msgtypes", "msg_types"],
        action: |_| Some(Action::ShowMessageTypes),
        usage: "/messagetypes",
    },
    // The staff channels, which ACE answers for advocates alone.
    Command {
        names: &["on"],
        action: |args| ac_net::messages::channel::from_name(args).map(Action::ChannelJoin),
        usage: "/on CHANNEL",
    },
    Command {
        names: &["off"],
        action: |args| ac_net::messages::channel::from_name(args).map(Action::ChannelLeave),
        usage: "/off CHANNEL",
    },
    Command {
        names: &["clist"],
        action: |args| ac_net::messages::channel::from_name(args).map(Action::ChannelList),
        usage: "/clist CHANNEL",
    },
    Command {
        names: &["index"],
        action: |_| Some(Action::ChannelIndex),
        usage: "/index",
    },
    // The chat window itself.
    Command {
        names: &["log"],
        action: |args| {
            Some(Action::ChatToFile(
                (!args.is_empty()).then(|| args.to_string()),
            ))
        },
        usage: "/log FILE, or /log to stop",
    },
    Command {
        names: &["clear"],
        action: |args| {
            // Retail read the first word and ignored the rest.
            let first = args.split_whitespace().next().unwrap_or_default();
            Some(Action::ChatClear {
                all: first.eq_ignore_ascii_case("all"),
            })
        },
        usage: "/clear [all]",
    },
    Command {
        names: &["title"],
        action: |args| (!args.is_empty()).then(|| Action::ChatTitle(args.to_string())),
        usage: "/title what to call this chat window",
    },
    //
    // -- allegiance and fellowship --
    //
    // Managing the allegiance: one subcommand tree, which `@motd` and `@ah`
    // are two more ways into (retail registers them with the same handlers).
    // Every one sends a game action of its own; only `broadcast`, `hometown`
    // and `chat on|off` land somewhere else.
    Command {
        names: &["allegiance", "all"],
        action: allegiance::subcommand,
        usage: "/allegiance boot|info|chat|broadcast|ban|officer|title|hometown|motd|name|lock|house ...",
    },
    Command {
        names: &["motd"],
        action: |args| allegiance::motd(args).map(Action::Allegiance),
        usage: "/motd [set TEXT|clear]",
    },
    Command {
        names: &["ah", "alh"],
        // Retail refuses arguments here: "This command takes no arguments!".
        action: |args| args.is_empty().then_some(Action::RecallHometown),
        usage: "/ah",
    },
    // The group channels, a ChatChannel (0x0147) each: what a character says
    // to its fellowship, its patron, its vassals, its monarch, its co-vassals
    // or the whole allegiance. `/a` is the odd one out -- retail hands the
    // allegiance over to Turbine chat as it starts (FUN_0057fcc0:17-39), so it
    // speaks in the allegiance's room while `/ab` keeps the broadcast channel.
    Command {
        names: &[
            "fellowship",
            "fellows",
            "fellow",
            "f",
            "group",
            "g",
            "party",
        ],
        action: |args| fellow::say_on(channel::FELLOW, args),
        usage: "/fellowship what you are saying",
    },
    Command {
        names: &["vassals", "vassal", "v"],
        action: |args| fellow::say_on(channel::VASSALS, args),
        usage: "/vassals what you are saying",
    },
    Command {
        names: &["patron", "p"],
        action: |args| fellow::say_on(channel::PATRON, args),
        usage: "/patron what you are saying",
    },
    Command {
        names: &["monarch", "m"],
        action: |args| fellow::say_on(channel::MONARCH, args),
        usage: "/monarch what you are saying",
    },
    Command {
        names: &["covassals", "co-vassals", "covassal", "c"],
        action: |args| fellow::say_on(channel::CO_VASSALS, args),
        usage: "/covassals what you are saying",
    },
    Command {
        names: &["ab"],
        action: |args| fellow::say_on(channel::ALLEGIANCE_BROADCAST, args),
        usage: "/ab what the allegiance is to hear",
    },
    Command {
        names: &["a"],
        action: |args| {
            (!args.is_empty()).then(|| Action::Room {
                room: turbine::ALLEGIANCE,
                text: args.to_string(),
            })
        },
        usage: "/a what you are saying",
    },
    // Hearing a room is one character option, which the server answers with
    // "You have entered the X channel." Only the first word is read.
    Command {
        names: &["join"],
        action: |args| allegiance::room_wanted(args).map(|room| Action::Listen { room, on: true }),
        usage: "/join allegiance|general|trade|lfg|roleplay|society",
    },
    Command {
        names: &["leave"],
        action: |args| allegiance::room_wanted(args).map(|room| Action::Listen { room, on: false }),
        usage: "/leave allegiance|general|trade|lfg|roleplay|society",
    },
    //
    // -- status and who (age, loc, version, friends, the housing list) --
    //
    // `/help` and `/?` are not here: retail's help is every family's own
    // text, and until that is written the console plugin's list answers.
    Command {
        names: &["day"],
        action: |_| Some(Action::Daylight),
        usage: "/day",
    },
    Command {
        names: &["endurance"],
        action: |_| Some(Action::Endurance),
        usage: "/endurance",
    },
    // Retail turned the three below away when words followed them.
    Command {
        names: &["framerate"],
        action: |args| args.is_empty().then_some(Action::Framerate),
        usage: "/framerate",
    },
    Command {
        names: &["loc"],
        action: |args| args.is_empty().then_some(Action::Location),
        usage: "/loc",
    },
    Command {
        names: &["version"],
        action: |args| args.is_empty().then_some(Action::Version),
        usage: "/version",
    },
    Command {
        names: &["age"],
        action: |_| Some(Action::QueryAge),
        usage: "/age",
    },
    Command {
        names: &["birth"],
        action: |_| Some(Action::QueryBirth),
        usage: "/birth",
    },
    Command {
        names: &["friends"],
        action: |args| status::friends_args(args).map(Action::Friends),
        usage: "/friends [online | add NAME | remove NAME | remove -all | old]",
    },
    Command {
        names: &["friends_add"],
        action: |args| status::add_args(args).map(Action::Friends),
        usage: "/friends_add NAME",
    },
    Command {
        names: &["friends_remove"],
        action: |args| status::remove_args(args).map(Action::Friends),
        usage: "/friends_remove NAME | -all",
    },
    Command {
        names: &["hslist"],
        action: |args| status::house_kind(args).map(Action::HousesAvailable),
        usage: "/hslist apartment | cottage | villa | mansion",
    },
    // The two house recalls under their own names; `/house` is the recall
    // family's row for the same action.
    Command {
        names: &["hor", "hr"],
        action: |args| args.is_empty().then_some(Action::RecallHouse),
        usage: "/hor",
    },
    Command {
        names: &["hom", "hoa"],
        action: |args| args.is_empty().then_some(Action::RecallMansion),
        usage: "/hom",
    },
    //
    // -- player killing, consent and items --
];

/// Retail names with no row yet: the list the families work through. Every
/// chat name now has a row, so none of these is reached by the router's
/// fallback any more.
pub const PENDING: &[&str] = &[
    "?",
    "help",
    "consent",
    "corpse",
    "cor",
    "permit",
    "pkarena",
    "pka",
    "pklarena",
    "pla",
    "fillcomps",
    "loadfile",
];

/// Retail names that never left the retail client: help topics with no handler
/// at all, the two that only filled in the chat entry, and the windows and
/// bindings a UI of ours keeps its own way.
pub const CLIENT_UI_ONLY: &[&str] = &[
    "commands",
    "allegiances",
    "channels",
    "chatting",
    "death",
    "status",
    "text",
    "mr",
    "pr",
    "emotes",
    "saveui",
    "loadui",
    "saveautoui",
    "loadautoui",
    "lockui",
];

/// Retail names whose handler did nothing by the end of retail.
pub const RETIRED: &[&str] = &["speaker", "render"];

/// The retail command `name` (case-insensitively), whoever asks.
pub fn command(name: &str) -> Option<&'static Command> {
    let name = name.trim().to_ascii_lowercase();
    RETAIL.iter().find(|c| c.names.contains(&name.as_str()))
}

/// What a `/name` with no row means when no plugin or script takes it: a soul
/// emote, a chat room or a group channel by its prefix, else the server's own.
fn unclaimed(name: &str, args: &str, body: &str) -> Action {
    // `/wave`, `/bow`: retail read these from its emote table, not from the
    // command table (`*wave*` typed in chat does the same).
    if args.is_empty() && crate::emotes::lookup(name).is_some() {
        return Action::SoulEmote(name.to_string());
    }
    if !args.is_empty() {
        if let Some(room) = ac_net::messages::turbine::from_prefix(name) {
            return Action::Room {
                room,
                text: args.to_string(),
            };
        }
        if let Some(channel) = ac_net::messages::channel::from_prefix(name) {
            return Action::Channel {
                channel,
                text: args.to_string(),
            };
        }
    }
    Action::ServerCommand(body.to_string())
}

/// Nothing of that name is there to act on.
pub(crate) fn nothing_named(target: &Target) -> Refused {
    match target {
        Target::Name(n) => Refused::ours(format!("nothing named {n:?} in view")),
        Target::LastCorpse => Refused::ours("no corpse of the last target in view"),
        Target::Selected => Refused::ours("nothing selected"),
        Target::Me | Target::Guid(_) => Refused::ours("it is not there"),
    }
}
