//! What a front end asks the character to do, in one vocabulary: the chat box,
//! a key, a Rhai call, the bus, the CLI and a panel button all build an
//! [`Action`] and hand it to [`Client::act`]. `act` carries external intent
//! only -- autoplay decides for itself and calls the same `Client` methods
//! directly, as does every read.

use serde::{Deserialize, Serialize};

use crate::autoplay::Role;
use crate::Client;

mod chat;
mod combat;
mod fellow;
mod item;
mod magic;
mod recall;
pub mod resolve;
mod team;
mod trade;

/// Why an action did not happen: the words a front end shows, and the server's
/// own code when it gave one (the vocabulary every system refuses in).
pub type Refused = ac_agent::did::Because;

/// What came of an [`Action`]: the request went out, or it was refused here
/// before it went. The server's own answer arrives later, as an `Event`.
pub type Outcome = Result<(), Refused>;

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
    Tell { who: String, text: String },
    /// A freeform emote ("/emote grins"), said as an action of ours.
    Emote(String),
    /// A soul emote by word ("wave", "*bow*"): the motion and its line.
    SoulEmote(String),
    /// Speak in a Turbine chat room (`ac_net::messages::turbine`).
    Room { room: u32, text: String },
    /// Speak on a group channel (`ac_net::messages::channel`).
    Channel { channel: u32, text: String },
    /// Away from keyboard, with the message others get when they tell us.
    Afk { away: bool, message: String },
    /// A command for the server itself, sent as its `@command` line.
    ServerCommand(String),

    // ---- item ----
    /// Use it: wield it, pick it up, open it, talk to it (what a
    /// double-click did).
    Use(Target),
    /// Apply one carried thing to another (a kit, a stone, a key).
    UseOn { item: Target, target: Target },
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
    PutIn { item: Target, container: Target },
    /// Take `amount` off a stack, into the pack.
    Split { item: Target, amount: u32 },
    /// Pour one stack into another.
    Merge { from: Target, to: Target },
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
    Cast { spell: SpellRef, at: Option<Target> },
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
    Buy { ware: Target, amount: u32 },
    /// Sell to the open counter.
    Sell(Target),
    /// Leave the counter.
    CloseVendor,

    // ---- fellow ----
    /// Found a fellowship.
    FellowCreate { name: String, share_xp: bool },
    /// Invite a player into ours.
    FellowRecruit(Target),
    /// Put a fellow out of it.
    FellowDismiss(Target),
    /// Leave ours, disbanding it if we lead.
    FellowQuit { disband: bool },
    /// Answer the question the server asked (a recruit, an allegiance).
    Confirm(bool),

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
            Action::Afk { away, message } => chat::afk(self, away, &message),
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
        }
    }
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
