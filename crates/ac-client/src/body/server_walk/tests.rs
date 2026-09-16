use super::*;

const ME: u32 = 0x5000_0001;

const CORPSE: u32 = 0x8000_1809;

/// A MovementEvent for our own character as ACE writes it: sequences,
/// autonomy, padding, movement type, flags, stance, then `rest`.
fn our_movement(movement_type: u8, rest: &[u8]) -> Vec<u8> {
    let mut w = ac_net::wire::Writer::new();
    w.u32(ac_net::messages::opcode::MOVEMENT_EVENT)
        .u32(ME)
        .u16(1)
        .u16(2)
        .u16(3)
        .u8(0)
        .align4()
        .u8(movement_type)
        .u8(0)
        .u16(0x3D)
        .bytes(rest);
    w.finish()
}

/// TurnToObject: face the corpse.
fn turn_to_corpse() -> Vec<u8> {
    let mut w = ac_net::wire::Writer::new();
    w.u32(CORPSE).f32(0.0).u32(0).f32(1.0).f32(90.0);
    our_movement(8, &w.finish())
}

/// MoveToObject: walk to the corpse.
fn walk_to_corpse() -> Vec<u8> {
    let mut w = ac_net::wire::Writer::new();
    w.u32(CORPSE)
        .u32(0x01F6_022F)
        .f32(40.0)
        .f32(-18.0)
        .f32(0.0)
        .u32(0)
        .f32(0.6)
        .f32(0.0)
        .f32(f32::MAX)
        .f32(1.0)
        .f32(15.0)
        .f32(0.0)
        .f32(1.5);
    our_movement(6, &w.finish())
}

/// MoveToObject as ACE sends it for a melee charge: the drudge, and
/// the charge's own parameters (CanCharge, FailWalk, UseFinalHeading,
/// Sticky, MoveAway; given up past fifteen metres).
fn charge_at_drudge() -> Vec<u8> {
    let mut w = ac_net::wire::Writer::new();
    w.u32(DRUDGE)
        .u32(0x01F6_027B)
        .f32(80.0)
        .f32(-36.0)
        .f32(0.0)
        .u32(0x1F0)
        .f32(0.6)
        .f32(0.0)
        .f32(15.0)
        .f32(1.5)
        .f32(1.0)
        .f32(0.0)
        .f32(1.5);
    our_movement(6, &w.finish())
}

const DRUDGE: u32 = 0x8000_30F2;

/// The client's side of a server walk, as `Client` keeps it.
struct Walking {
    world: ac_world::World,
    walk: Option<ac_world::object::MoveTarget>,
    since: Instant,
    answered: bool,
}

impl Walking {
    fn new(t0: Instant) -> Self {
        let mut world = ac_world::World::default();
        world.player_guid = Some(ME);
        world.objects.insert(
            ME,
            ac_world::WorldObject {
                guid: ME,
                ..Default::default()
            },
        );
        Walking {
            world,
            walk: None,
            since: t0,
            answered: false,
        }
    }

    /// What the client does with a movement event for us: the world
    /// takes it, and the walk it names (if any) goes to the server walk.
    fn hear(&mut self, msg: &[u8], now: Instant) -> ServerWalk {
        self.world.apply(msg);
        let target = self.world.player_mut().and_then(|o| o.target.take());
        heard_server_walk(
            &mut self.walk,
            &mut self.since,
            &mut self.answered,
            target,
            now,
        )
    }

    /// What the client does with a UseDone or a refused inventory
    /// action about `about`: `used` is what the last use was sent
    /// for, and `attacked` what the last attack went at.
    fn answer(&mut self, about: &[u32], used: Option<u32>, attacked: Option<u32>) {
        if answers_walk(self.walk, about, used, attacked) {
            self.answered = true;
        }
    }
}

#[test]
fn a_turn_to_face_what_was_used_is_no_server_walk() {
    // Blargerton emptied a corpse in reach, then stood beside it for
    // twelve seconds instead of walking to the next: the turn the use
    // began with was taken for a server walk to the corpse, and the
    // client stood aside for it.
    let t0 = Instant::now();
    let soon = t0 + Duration::from_millis(500);
    let mut w = Walking::new(t0);

    // A use in reach: the turn gives nothing to stand aside for.
    assert_eq!(w.hear(&turn_to_corpse(), t0), ServerWalk::Unchanged);
    assert_eq!(w.walk, None);
    assert!(!standing_aside(w.walk, w.since, soon));

    // A use out of reach: the server walks us, and the client stands
    // aside until the walk ends or has gone on too long.
    assert_eq!(
        w.hear(&walk_to_corpse(), t0),
        ServerWalk::Began(ac_world::object::MoveTarget::Object(CORPSE))
    );
    assert!(standing_aside(w.walk, w.since, soon));
    assert!(!standing_aside(w.walk, w.since, t0 + SERVER_WALK_FOR));

    // A turn while the server walks us: ACE calls the walk off to turn.
    assert_eq!(w.hear(&turn_to_corpse(), soon), ServerWalk::Ended);
    assert_eq!(w.walk, None);
    assert!(!standing_aside(w.walk, w.since, soon));
}

#[test]
fn a_walk_for_a_use_is_over_once_there_and_answered() {
    // ACE runs a use on arriving and answers it, and sends no motion:
    // the client stood aside for the rest of its twelve seconds, and
    // the walk to whatever came next went nowhere.
    let t0 = Instant::now();
    let mut w = Walking::new(t0);
    let corpse = glam::Vec3::new(40.0, -18.0, 0.0);
    let goal = Some((corpse, 1.0));
    let far = glam::Vec3::new(33.0, -18.0, 0.0);
    let there = glam::Vec3::new(39.4, -18.2, 0.0);
    let under = glam::Vec3::new(39.4, -18.2, -3.0);

    // An answer before any walk is not an answer to one.
    w.answer(&[CORPSE], Some(CORPSE), None);
    assert!(!w.answered);
    assert!(matches!(
        w.hear(&walk_to_corpse(), t0),
        ServerWalk::Began(_)
    ));
    assert!(!w.answered);

    // There, but not answered: the use has not run, and a movement of
    // our own now would call it off.
    assert!(!server_walk_over(w.walk, goal, there, w.answered));

    // Answered on the way (a double-click sent again has the first one
    // answered): letting go here stopped the character a stride in.
    w.answer(&[CORPSE], Some(CORPSE), None);
    assert!(!server_walk_over(w.walk, goal, far, w.answered));
    // Nor under its floor.
    assert!(!server_walk_over(w.walk, goal, under, w.answered));

    // There and answered: over, whichever came first.
    assert!(server_walk_over(w.walk, goal, there, w.answered));
    // Gone from view, there is nowhere left to walk to.
    assert!(server_walk_over(w.walk, None, far, w.answered));

    // The walk sent again has had no answer yet.
    assert_eq!(w.hear(&walk_to_corpse(), t0), ServerWalk::Unchanged);
    assert!(!server_walk_over(w.walk, goal, there, w.answered));

    // No walk, nothing to be over.
    assert!(!server_walk_over(None, goal, there, true));
    // Where the steering stops is where a walk is there.
    assert!(reached(there, corpse, 1.0));
    assert!(!reached(far, corpse, 1.0));
    assert!(!reached(under, corpse, 1.0));
}

#[test]
fn an_attack_done_ends_a_charge_at_what_was_attacked() {
    // A charge ACE gave up on ("You charged too far") is answered with
    // AttackDone and no motion: the character stood where it stopped,
    // fighting nothing, for twelve seconds.
    use ac_world::object::MoveTarget;
    let t0 = Instant::now();
    let mut w = Walking::new(t0);
    assert_eq!(
        w.hear(&charge_at_drudge(), t0),
        ServerWalk::Began(MoveTarget::Object(DRUDGE))
    );
    assert!(attack_ended_walk(w.walk, Some(DRUDGE)));

    // A walk for anything else is not the attack's to end: a corpse
    // being walked to, a place, or no attack at all.
    assert!(!attack_ended_walk(
        Some(MoveTarget::Object(CORPSE)),
        Some(DRUDGE)
    ));
    let place = MoveTarget::Position {
        cell: 0x01F6_027B,
        local: glam::Vec3::new(80.0, -36.0, 0.0),
    };
    assert!(!attack_ended_walk(Some(place), Some(DRUDGE)));
    assert!(!attack_ended_walk(w.walk, None));
    assert!(!attack_ended_walk(None, Some(DRUDGE)));
}

#[test]
fn nothing_is_wielded_while_an_attack_is_unanswered() {
    // Every "Action cancelled" in run18 and run20 follows a change of
    // the character's own hands within a fraction of a second: the
    // buff pass reaching for a wand, the arming taking the mace back,
    // peace mode for a corpse. ACE turns each into a cancelled attack.
    let sent = Instant::now();
    assert!(attack_unanswered(true, sent, sent));
    assert!(attack_unanswered(
        true,
        sent,
        sent + Duration::from_millis(1500)
    ));

    // Answered: the gap between two swings, and the hands are the
    // character's own again.
    assert!(!attack_unanswered(
        false,
        sent,
        sent + Duration::from_millis(1500)
    ));

    // An answer that never came does not hold the hands for the
    // session: one lost AttackDone must not leave a character unable
    // to change weapon for as long as it stays logged in.
    assert!(!attack_unanswered(
        true,
        sent,
        sent + ATTACK_ANSWERED_WITHIN
    ));
}

#[test]
fn a_refused_wield_mid_charge_does_not_end_the_charge() {
    // +Verity (run18) charged a Drudge Slave while her buffing asked
    // for the Training Wand every second and a half and was refused
    // each time. Each refusal was taken for the charge's answer: a
    // metre from the drudge she took the controls back, ran for her
    // own goal, and was told "You charged too far".
    use ac_world::object::MoveTarget;
    const WAND: u32 = 0x8000_00C3;
    const PYREAL: u32 = 0x8000_3421;
    const PORTAL: u32 = 0x7000_0101;
    let t0 = Instant::now();
    let mut w = Walking::new(t0);
    let drudge = glam::Vec3::new(80.0, -36.0, 0.0);
    let there = glam::Vec3::new(79.6, -36.2, 0.0);
    assert!(matches!(
        w.hear(&charge_at_drudge(), t0),
        ServerWalk::Began(_)
    ));

    // The wand refused: about the wand, and the pack it is in.
    w.answer(&[WAND, ME], Some(CORPSE), Some(DRUDGE));
    // A cast's UseDone: the cast cleared what was used.
    w.answer(&[], None, Some(DRUDGE));
    // Nor the answer to a use of the last corpse.
    w.answer(&[CORPSE], Some(CORPSE), Some(DRUDGE));
    // Nor anything about the creature itself: AttackDone or a motion
    // ends a charge.
    w.answer(&[DRUDGE], Some(DRUDGE), Some(DRUDGE));
    assert!(!w.answered);
    assert!(!server_walk_over(
        w.walk,
        Some((drudge, 1.0)),
        there,
        w.answered
    ));
    assert!(attack_ended_walk(w.walk, Some(DRUDGE)));

    // What a walk for a use does take as its answer: the corpse's own
    // UseDone, with a fight going on or not.
    let to_corpse = Some(MoveTarget::Object(CORPSE));
    assert!(answers_walk(
        to_corpse,
        &[CORPSE],
        Some(CORPSE),
        Some(DRUDGE)
    ));
    // A take from it refused, which names the item the corpse holds.
    assert!(answers_walk(to_corpse, &[PYREAL, CORPSE], None, None));
    // A loose item walked to for a pickup, refused.
    let to_pyreal = Some(MoveTarget::Object(PYREAL));
    assert!(answers_walk(to_pyreal, &[PYREAL], Some(PYREAL), None));
    // Not the wand refused on the way there.
    assert!(!answers_walk(to_corpse, &[WAND, ME], Some(CORPSE), None));
    // ACE walks to a portal's place: the use sent is what it is for.
    let to_portal = Some(MoveTarget::Position {
        cell: 0x01F6_027B,
        local: glam::Vec3::new(80.0, -36.0, 0.0),
    });
    assert!(answers_walk(to_portal, &[PORTAL], Some(PORTAL), None));
    assert!(!answers_walk(to_portal, &[WAND, ME], Some(PORTAL), None));
    assert!(!answers_walk(to_portal, &[], None, None));
    // No walk, nothing to answer.
    assert!(!answers_walk(None, &[CORPSE], Some(CORPSE), None));
}

#[test]
fn a_pour_refused_on_the_way_is_no_answer_to_the_walk() {
    // The tidying pours in the gaps between takes, so a refusal can
    // land in the middle of a walk the server is doing. It is about
    // two stacks in the pack and nothing else -- but a looted stack
    // keeps the guid it was fetched under, so the refusal of a pour
    // of that very stack reads exactly like the refusal of the
    // pickup still being walked for.
    use ac_world::object::MoveTarget;
    const PYREAL: u32 = 0x8000_3421;
    const MORE_PYREALS: u32 = 0x8000_3422;
    let pour = pack::PourSent {
        merge: pack::Merge {
            from: PYREAL,
            to: MORE_PYREALS,
            amount: 5,
            name: "Pyreal".into(),
            frees_a_slot: true,
        },
        to_before: 40,
    };
    let sent = Instant::now();
    let air = (pour, sent);
    assert!(answers_a_pour(Some(&air), PYREAL, sent), "the source");
    assert!(
        answers_a_pour(Some(&air), MORE_PYREALS, sent),
        "or the target"
    );
    // Anything else is the walk's business, as it always was.
    assert!(!answers_a_pour(Some(&air), CORPSE, sent));
    assert!(!answers_a_pour(None, PYREAL, sent));
    // A pour nobody ever settled -- autoplay switched off a moment
    // after it went out, so the housekeeping that settles it never
    // ran again -- stops claiming refusals at the same wall the
    // settler gives up at. Without this it went on swallowing the
    // answers to server walks for those two stacks for as long as
    // the character stood there.
    let later = sent + pack::POUR_LOST;
    assert!(!answers_a_pour(Some(&air), PYREAL, later));
    assert!(!answers_a_pour(Some(&air), MORE_PYREALS, later));
    assert!(answers_a_pour(
        Some(&air),
        PYREAL,
        sent + pack::POUR_LOST - std::time::Duration::from_millis(1)
    ));
    // Without the guard, the walk to fetch the stack would take its
    // own loot's pour refusal for the pickup's answer.
    let to_pyreal = Some(MoveTarget::Object(PYREAL));
    assert!(answers_walk(to_pyreal, &[PYREAL, ME], None, None));
}
