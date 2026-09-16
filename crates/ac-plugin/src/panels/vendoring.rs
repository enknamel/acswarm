//! The shopping, laid out so it can be watched and driven by hand.
//!
//! A run to town is autoplay's: the counter chosen and walked to, its
//! window opened, the pack appraised and put together, then the
//! selling and buying decided in `ac-vendor` from a plain description
//! of the moment. This panel shows where that run stands and the one
//! thing it would do next -- *before* it is done -- and offers to do
//! it.
//!
//! The point is to be able to test just the shopping, exactly as
//! autoplay runs it, without turning autoplay on. **Run** starts the
//! very run autoplay would start and takes it turn by turn at
//! autoplay's own pace, waiting on the counter's answer rather than on
//! a clock. **Step** carries the run on until one act has gone out --
//! a walk begun, the counter used, an armful handed over -- and then
//! holds, which is how a trip is read a line at a time; a Step that
//! lands on a wait waits it out (see `ac_client::growth::Turn`).
//! **Stop** ends the run and leaves the character where it stands.
//! None of it changes the rules: the same code runs here and under
//! autoplay, and the same `Run::step` answers in both and in
//! `cargo run -p ac-vendor --example trip`.
//!
//! With autoplay on and running town runs, the run is autoplay's to
//! step: the panel shows it, Run starts one when there is none, and
//! Step stands down, since autoplay would take the next act before the
//! hold could. A run the panel was driving and has left held -- a Step
//! that has had its act, the panel closed on one -- is autoplay's
//! again after a moment, so the character is not stood at a counter
//! for good by a button nobody is pressing.
//!
//! Each session drives its own run: the host ticks the panel once a
//! frame for every session, and the button pressed on the one being
//! looked at is not a button pressed on the rest.

use std::collections::BTreeMap;

use ac_client::growth::{Driver, Turn};
use ac_vendor::{Act, Snapshot};

use crate::{Ctx, Plugin, Settings};

pub(crate) const OPEN_KEY: &str = "vendoring.open";

/// What the panel draws, read fresh from the character each frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct VendorView {
    pub counter: Option<String>,
    /// How far the character is standing from it.
    pub away: f32,
    pub open: bool,
    /// The run to town under way: where it is going and why, and who
    /// is stepping it.
    pub trip: Option<String>,
    pub driver: Option<Driver>,
    /// Autoplay is on and running town runs, so a run started here is
    /// its to step.
    pub autoplay_drives: bool,
    pub phase: String,
    /// What the rules would do next, and how they put it.
    pub next: Option<(String, String)>,
    pub coin: u32,
    pub notes: u32,
    pub slots_free: u32,
    pub keep_slots: u32,
    pub carried: u32,
    pub ceiling: u32,
    /// Things in the pack, and how many of those are for sale here.
    pub items: usize,
    pub for_sale: usize,
    pub sold: u32,
    pub waiting: usize,
    /// The first few things it will not sell, and why.
    pub kept: Vec<(String, String)>,
}

/// How the panel is driving the run, if it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Drive {
    /// Not at all: the run waits for the next press.
    #[default]
    Held,
    /// Until one act has gone out, then held.
    Once,
    /// Turn after turn until the run is over or Stop is pressed.
    Running,
}

/// What the drive comes to after a turn of the run: Step holds once an
/// act has gone out, and both stop when the run is over. A wait
/// changes nothing -- that is what makes a Step that lands on one wait
/// it out rather than skip it.
fn after_turn(drive: Drive, turn: Turn) -> Drive {
    match (drive, turn) {
        (_, Turn::Over) => Drive::Held,
        (Drive::Once, Turn::Acted) => Drive::Held,
        (drive, _) => drive,
    }
}

/// Which of the three buttons can be pressed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Buttons {
    step: bool,
    run: bool,
    stop: bool,
}

/// Which buttons a moment offers.
///
/// With no run, Run starts one, and so does Step -- setting off is
/// its one act -- unless autoplay would then take every act after it.
/// A run the panel is driving takes a Step while held, a Run while not
/// already running, and a Stop always. A run autoplay is driving takes
/// only a Stop: the panel shows it and does not start a second, and a
/// Step would be overtaken by autoplay's own next turn.
fn buttons(live: bool, driver: Option<Driver>, autoplay_drives: bool, drive: Drive) -> Buttons {
    if !live {
        return Buttons::default();
    }
    match driver {
        None => Buttons {
            step: !autoplay_drives,
            run: true,
            stop: false,
        },
        Some(Driver::Hand) => Buttons {
            step: drive == Drive::Held,
            run: drive != Drive::Running,
            stop: true,
        },
        Some(Driver::Autoplay) => Buttons {
            step: false,
            run: false,
            stop: true,
        },
    }
}

/// How one session's run is being driven from here, and what the
/// last press on it came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Hand {
    drive: Drive,
    /// Why the last press came to nothing, for the panel.
    word: String,
}

#[derive(Default)]
pub(crate) struct Vendoring {
    show: bool,
    /// One hand per session, by index: the host ticks every session
    /// through this one panel, and one drive for all of them let any
    /// session without a run hold the one that had.
    hands: BTreeMap<usize, Hand>,
    /// A made-up trip, for the layout gallery and for looking at the
    /// panel without a server.
    demo: Option<VendorView>,
}

impl Vendoring {
    /// A character halfway through a trip: low on room, a fortune in
    /// coin waiting to become notes, and two things it will not part
    /// with.
    pub(crate) fn demo() -> Self {
        Vendoring {
            show: false,
            hands: BTreeMap::new(),
            demo: Some(VendorView {
                counter: Some("Rakk the Peddler".into()),
                away: 1.2,
                open: true,
                trip: Some("to Rakk the Peddler, stop 1: asked for from the panel".into()),
                driver: Some(Driver::Hand),
                autoplay_drives: false,
                phase: "Selling".into(),
                next: Some(("buy 3".into(), "packing the takings into 3 note(s)".into())),
                coin: 1_012_400,
                notes: 14,
                slots_free: 3,
                keep_slots: 3,
                carried: 21_480,
                ceiling: 24_300,
                items: 27,
                for_sale: 9,
                sold: 7,
                waiting: 0,
                kept: vec![
                    ("Tinkered Sword".into(), "it has been tinkered".into()),
                    ("Worn Breastplate".into(), "it is being worn".into()),
                ],
            }),
        }
    }

    /// Session `index`'s hand, made as needed.
    fn hand(&mut self, index: usize) -> &mut Hand {
        self.hands.entry(index).or_default()
    }

    /// How session `index`'s run is being driven.
    fn drive(&self, index: usize) -> Drive {
        self.hands.get(&index).map(|h| h.drive).unwrap_or_default()
    }
}

/// Read the character into the panel's view, without deciding
/// anything that would change it.
fn view(c: &mut ac_client::Client, now: std::time::Instant) -> Option<VendorView> {
    let cfg = c.autoplay.config.growth.clone();
    let snap: Snapshot = c.vendor_snapshot(&cfg);
    let counter = snap.counter.clone();
    let for_sale = for_sale_here(&snap);
    let kept = snap
        .items
        .iter()
        .filter_map(|i| i.keep.why().map(|w| (i.name.clone(), w.to_string())))
        .take(6)
        .collect();
    let run = c.town_run_view(now);
    let (trip, driver, phase, next, sold, waiting) = match run {
        Some(r) => (
            Some(format!("to {}, stop {}: {}", r.vendor, r.stop, r.reason)),
            Some(r.driver),
            r.phase,
            Some((r.next.as_ref().map(act_label).unwrap_or_default(), r.saying)),
            r.sold,
            r.waiting,
        ),
        // No run: a counter the player opened by hand is still shown
        // what the rules would do at it. Asking must not move anything
        // on, so the question is put to a fresh copy of the rules.
        None => {
            let next = counter.as_ref().map(|_| {
                let peek = ac_vendor::Run::new().step(&snap, now);
                (
                    peek.act.as_ref().map(act_label).unwrap_or_default(),
                    peek.saying,
                )
            });
            (None, None, "no run to town".into(), next, 0, 0)
        }
    };
    Some(VendorView {
        counter: counter.as_ref().map(|c| c.name.clone()),
        away: counter.as_ref().map(|c| c.away).unwrap_or(0.0),
        open: counter.is_some(),
        trip,
        driver,
        autoplay_drives: c.autoplay_drives_town_runs(),
        phase,
        next,
        coin: snap.coin,
        notes: snap.notes.values().sum(),
        slots_free: snap.slots_free,
        keep_slots: snap.rules.keep_slots,
        carried: snap.carried,
        ceiling: snap.capacity.saturating_mul(3),
        items: snap.items.len(),
        for_sale,
        sold,
        waiting,
        kept,
    })
}

/// How many carried things the counter in front of the character
/// would be offered: the selling's own rule, so the count says what the
/// counter will take rather than what the pack holds. The panel used to
/// count everything not forbidden and showed nine for sale at a tailor
/// who buys none of them.
fn for_sale_here(snap: &Snapshot) -> usize {
    snap.items.iter().filter(|i| snap.offers(i)).count()
}

/// A short name for an act, for the panel's line.
fn act_label(a: &Act) -> String {
    match a {
        Act::Approach { .. } => "walk up".into(),
        Act::Open { .. } => "open".into(),
        Act::Merge { amount, .. } => format!("merge {amount}"),
        Act::Split { amount, .. } => format!("split {amount}"),
        Act::Sell { .. } => "sell".into(),
        Act::Buy { count, .. } => format!("buy {count}"),
        Act::Cash { count, .. } => format!("cash {count}"),
        Act::Close => "close".into(),
    }
}

impl Plugin for Vendoring {
    fn name(&self) -> &str {
        "vendoring"
    }

    fn load(&mut self, settings: &Settings) {
        if let Some(v) = settings.get("vendoring.show") {
            self.show = v;
        }
    }

    fn save(&self, settings: &mut Settings) {
        settings.set("vendoring.show", self.show);
    }

    /// The sessions above the one dropped move down, each with its
    /// hand.
    fn session_removed(&mut self, index: usize) {
        crate::shift_removed(&mut self.hands, index);
    }

    /// The driving, once a frame per session, whether or not the window
    /// is showing: a run set going and then hidden goes on. Only this
    /// session's drive is read and written: the tick for a session
    /// with no run of its own is no word on the others'.
    fn tick(&mut self, cx: &mut Ctx) {
        let index = cx.index;
        let drive = self.drive(index);
        if self.demo.is_some() || drive == Drive::Held {
            return;
        }
        let now = cx.now;
        let Some(c) = cx.try_client() else {
            self.hand(index).drive = Drive::Held;
            return;
        };
        // Autoplay stepping the same run would send the next act before
        // Step could hold, or send it twice: this side stands down.
        if c.town_run_driver() != Some(Driver::Hand) {
            self.hand(index).drive = Drive::Held;
            return;
        }
        let turn = c.town_run_step_by_hand(now);
        self.hand(index).drive = after_turn(drive, turn);
    }

    fn ui(&mut self, cx: &mut Ctx, egui: &egui::Context) {
        if let Some(ask) = super::take_open(cx.board, "vendoring") {
            self.show = ask.apply(self.show);
        }
        cx.board.set(OPEN_KEY, self.show);
        if !self.show {
            return;
        }
        let now = cx.now;
        let index = cx.index;
        let v = match &self.demo {
            Some(d) => Some(d.clone()),
            None => cx.try_client().and_then(|c| view(c, now)),
        };
        let Some(v) = v else { return };
        let hand = self.hand(index).clone();
        let can = buttons(self.demo.is_none(), v.driver, v.autoplay_drives, hand.drive);
        let mut step = false;
        let mut go = false;
        let mut halt = false;
        egui::Window::new("Vendoring")
            .default_width(340.0)
            .show(egui, |ui| {
                match &v.counter {
                    Some(name) => {
                        ui.label(format!("{name}, {:.1} m away", v.away));
                    }
                    None => {
                        ui.label("no counter open");
                    }
                }
                ui.separator();
                match &v.trip {
                    Some(trip) => {
                        ui.label(format!("run {trip}"));
                    }
                    None => {
                        ui.label("no run to town: Run or Step starts one");
                    }
                }
                ui.label(format!("phase: {}", v.phase));
                match &v.next {
                    Some((what, saying)) if what.is_empty() => {
                        ui.label(format!("next: {saying}"));
                    }
                    Some((what, saying)) => {
                        ui.label(format!("next: {what} -- {saying}"));
                    }
                    None => {
                        ui.label("next: nothing to do here");
                    }
                }
                ui.separator();
                ui.label(format!(
                    "{} slot(s) free, reserve {}",
                    v.slots_free, v.keep_slots
                ));
                ui.label(format!("{} coin, {} note(s)", v.coin, v.notes));
                ui.label(format!("burden {} of {}", v.carried, v.ceiling));
                ui.label(format!(
                    "{} item(s), {} for sale here, {} sold, {} waiting",
                    v.items, v.for_sale, v.sold, v.waiting
                ));
                if !v.kept.is_empty() {
                    ui.separator();
                    ui.label("never sold:");
                    for (name, why) in &v.kept {
                        ui.label(format!("   {name} -- {why}"));
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    // One act, then hold: the way to read a trip a line
                    // at a time without handing the character over.
                    if ui
                        .add_enabled(can.step, egui::Button::new("Step"))
                        .clicked()
                    {
                        step = true;
                    }
                    if ui.add_enabled(can.run, egui::Button::new("Run")).clicked() {
                        go = true;
                    }
                    if ui
                        .add_enabled(can.stop, egui::Button::new("Stop"))
                        .clicked()
                    {
                        halt = true;
                    }
                    match (v.driver, hand.drive) {
                        (Some(Driver::Autoplay), _) => {
                            ui.label("autoplay is driving");
                        }
                        (Some(Driver::Hand), Drive::Running) => {
                            ui.label("running");
                        }
                        (Some(Driver::Hand), Drive::Once) => {
                            ui.label("stepping");
                        }
                        (Some(Driver::Hand), Drive::Held) => {
                            ui.label("held");
                        }
                        (None, _) if v.autoplay_drives => {
                            ui.label("autoplay will drive it");
                        }
                        (None, _) => {}
                    }
                });
                if !hand.word.is_empty() {
                    ui.label(&hand.word);
                }
            });
        if step || go || halt {
            let mut hand = hand;
            hand.word.clear();
            if let Some(c) = cx.try_client() {
                if halt {
                    c.town_run_stop(now);
                    hand.drive = Drive::Held;
                }
                // Setting off is the run's first act, so a Step with no
                // run starts one and holds there; with one it carries
                // the run on to its next act.
                if step || go {
                    let had_run = v.driver.is_some();
                    match c.town_run_by_hand(now) {
                        Ok(()) => {
                            if c.town_run_driver() == Some(Driver::Hand) {
                                hand.drive = if go {
                                    Drive::Running
                                } else if had_run {
                                    Drive::Once
                                } else {
                                    Drive::Held
                                };
                            }
                        }
                        Err(why) => hand.word = why,
                    }
                }
            }
            *self.hand(index) = hand;
        }
        if super::closed("vendoring") {
            self.show = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ac_vendor::counter::{Counter, Item};

    #[test]
    fn the_for_sale_count_is_what_the_counter_will_take() {
        // A pea and a tunic in the pack, at a tailor's window: one for
        // sale here, not two.
        let thing = |guid: u32, name: &str, item_type: u32| Item {
            guid,
            wcid: guid,
            name: name.into(),
            value: 500,
            stack: 1,
            max_stack: 1,
            item_type,
            ..Default::default()
        };
        let mut snap = Snapshot {
            items: vec![thing(1, "Lead Pea", 0x1000), thing(2, "Tunic", 0x8)],
            slots_free: 10,
            counter: Some(Counter {
                guid: 900,
                name: "Rakk the Peddler".into(),
                open: true,
                buys: 0x8 | 0x4,
                max_value: 0,
                wares: Vec::new(),
                note_face: None,
                away: 1.0,
            }),
            ..Default::default()
        };
        assert_eq!(for_sale_here(&snap), 1);
        snap.counter.as_mut().unwrap().buys = 0;
        assert_eq!(for_sale_here(&snap), 2, "a counter that takes anything");
        snap.counter = None;
        assert_eq!(for_sale_here(&snap), 0, "no counter to offer them to");
    }

    #[test]
    fn a_step_holds_after_one_act_and_waits_out_a_wait() {
        // The walk, the window, the appraisals and the counter's answer
        // are all waits: a Step that lands on one keeps the run going
        // until the act that follows, and holds on that.
        assert_eq!(after_turn(Drive::Once, Turn::Waited), Drive::Once);
        assert_eq!(after_turn(Drive::Once, Turn::Acted), Drive::Held);
        assert_eq!(after_turn(Drive::Once, Turn::Over), Drive::Held);
        // Run goes on through acts and waits alike, and stops with the
        // run.
        assert_eq!(after_turn(Drive::Running, Turn::Waited), Drive::Running);
        assert_eq!(after_turn(Drive::Running, Turn::Acted), Drive::Running);
        assert_eq!(after_turn(Drive::Running, Turn::Over), Drive::Held);
        // Held stays held, whatever the run did for somebody else.
        assert_eq!(after_turn(Drive::Held, Turn::Acted), Drive::Held);
    }

    #[test]
    fn each_session_has_a_hand_of_its_own_and_they_move_down_with_the_sessions() {
        // One drive for the whole panel let the tick for any session
        // without a run hold the run another session was driving:
        // with two sessions open, Run lived one frame and Step never
        // got past its first wait.
        let mut p = Vendoring::default();
        assert_eq!(p.drive(0), Drive::Held);
        p.hand(0).drive = Drive::Running;
        p.hand(2).drive = Drive::Once;
        p.hand(2).word = "busy fighting".into();
        assert_eq!(p.drive(1), Drive::Held, "session 1 took session 0's drive");
        assert_eq!(p.drive(0), Drive::Running);
        // Session 1 goes: session 2's hand is session 1's now.
        p.session_removed(1);
        assert_eq!(p.drive(0), Drive::Running);
        assert_eq!(p.drive(1), Drive::Once);
        assert_eq!(p.hand(1).word, "busy fighting");
        assert_eq!(p.drive(2), Drive::Held);
    }

    #[test]
    fn the_buttons_follow_who_is_driving() {
        let on = |b: Buttons| (b.step, b.run, b.stop);
        // Nothing to press in the gallery.
        assert_eq!(
            on(buttons(false, None, false, Drive::Held)),
            (false, false, false)
        );
        // No run: either button starts one; Stop has nothing to stop.
        assert_eq!(
            on(buttons(true, None, false, Drive::Held)),
            (true, true, false)
        );
        // With autoplay driving town runs, Run starts one for it and Step
        // stands down: autoplay would take every act after the first.
        assert_eq!(
            on(buttons(true, None, true, Drive::Held)),
            (false, true, false)
        );
        // The panel's own run: Step while held, Run unless running, Stop.
        let hand = Some(Driver::Hand);
        assert_eq!(
            on(buttons(true, hand, false, Drive::Held)),
            (true, true, true)
        );
        assert_eq!(
            on(buttons(true, hand, false, Drive::Once)),
            (false, true, true)
        );
        assert_eq!(
            on(buttons(true, hand, false, Drive::Running)),
            (false, false, true)
        );
        // Autoplay's run: shown, not started again, and only stopped.
        let auto = Some(Driver::Autoplay);
        assert_eq!(
            on(buttons(true, auto, true, Drive::Held)),
            (false, false, true)
        );
    }
}
