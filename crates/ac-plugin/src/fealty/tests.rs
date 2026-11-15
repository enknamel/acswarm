use super::*;

fn tree(rank: u32) -> Tree {
    let mut t = Tree {
        host: "play.example:9000".into(),
        target: "Blargerton".into(),
        rank,
        ..Default::default()
    };
    t.fill();
    t
}

fn all(t: &mut Tree, made: bool, ready: bool, sworn: bool) {
    for m in &mut t.members {
        m.made = made;
        m.ready = ready;
        m.sworn = sworn;
    }
}

#[test]
fn rank_six_is_sixty_three_characters_on_six_accounts_and_seven_reuses_them_all() {
    let mut t = tree(6);
    assert_eq!(t.members.len(), 63);
    assert_eq!(
        t.accounts.len(),
        6,
        "21 on even depths, 42 on odd, 11 an account"
    );
    all(&mut t, true, true, true);
    assert_eq!(rank_at(&t, 1), 6);
    let six = t.members.clone();
    t.rank = 7;
    let made = t.fill();
    assert_eq!(t.members.len(), 127);
    assert_eq!(t.accounts.len(), 12);
    assert_eq!(made.len(), 6, "the new bottom row's accounts only");
    assert_eq!(
        &t.members[..63],
        &six[..],
        "the rank six tree stands as it was"
    );
    assert!(t.members[63..].iter().all(|m| m.slot >= 64 && !m.made));
    assert_eq!(rank_at(&t, 1), 6, "a row not yet sworn adds nothing");
    all(&mut t, true, true, true);
    assert_eq!(rank_at(&t, 1), 7);
}

#[test]
fn a_patron_and_its_vassal_are_never_on_one_account_nor_more_than_eleven_on_one() {
    let t = tree(7);
    let account = |slot: u32| &t.members.iter().find(|m| m.slot == slot).unwrap().account;
    for m in t.members.iter().filter(|m| m.slot > 1) {
        assert_ne!(&m.account, account(m.slot / 2), "slot {}", m.slot);
    }
    for a in &t.accounts {
        let n = t.members.iter().filter(|m| m.account == a.name).count();
        assert!(n <= PER_ACCOUNT, "{} holds {n}", a.name);
    }
    let names: std::collections::BTreeSet<_> = t.members.iter().map(|m| &m.character).collect();
    assert_eq!(names.len(), t.members.len());
    assert_eq!(t.members[0].character, "Blargerton Vassal A");
    assert_eq!(t.accounts[0].name, "blargertonvassal1");
}

#[test]
fn rank_follows_ace_one_vassal_keeps_the_rank_two_raise_it() {
    let mut t = tree(2);
    all(&mut t, true, true, false);
    assert_eq!(rank_at(&t, 1), 1, "no vassals: rank 1");
    t.members[1].sworn = true;
    assert_eq!(rank_at(&t, 1), 1, "one rank 1 vassal keeps rank 1");
    t.members[2].sworn = true;
    assert_eq!(rank_at(&t, 1), 2);
}

#[test]
fn the_academy_comes_first_then_oaths_then_the_root_swears_last() {
    let mut t = tree(2);
    let w = wanted(&t, &[]).online;
    let accounts: Vec<&str> = w
        .iter()
        .map(|s| {
            t.members
                .iter()
                .find(|m| m.slot == *s)
                .unwrap()
                .account
                .as_str()
        })
        .collect();
    assert_eq!(w, vec![1, 2], "one an account: slots 2 and 3 share one");
    assert_ne!(accounts[0], accounts[1]);
    all(&mut t, true, true, false);
    let p = wanted(&t, &[]);
    assert_eq!(
        (p.online, p.oaths),
        (vec![2, 1], vec![2]),
        "the first oath, to the root"
    );
    t.members[1].sworn = true;
    assert_eq!(wanted(&t, &[]).online, vec![3, 1]);
    t.members[2].sworn = true;
    let p = wanted(&t, &[]);
    assert_eq!(
        (p.online, p.oaths),
        (vec![1], vec![1]),
        "the root, to the target"
    );
    t.members[0].sworn = true;
    assert_eq!(wanted(&t, &[]), Plan::default(), "done: everyone logs off");
}

#[test]
fn one_still_in_the_academy_keeps_its_account_ahead_of_an_oath() {
    let mut t = tree(2);
    all(&mut t, true, true, false);
    t.members[2].ready = false;
    assert_eq!(
        wanted(&t, &[3]).online,
        vec![3],
        "3 shares 2's account and is online"
    );
    assert_eq!(wanted(&t, &[]).online, vec![2, 1]);
    assert_eq!(
        wanted(&t, &[1, 3]).online,
        vec![3, 1],
        "the root waits online for its vassal coming out of the Academy"
    );
    t.members[0].ready = true;
    t.members[1].sworn = true;
    assert_eq!(
        wanted(&t, &[2]).online,
        vec![3],
        "one with nothing owed logs off"
    );
}

#[test]
fn the_patron_accepts_an_oath_from_the_member_it_waits_for_only() {
    let mut c = Client::offline(std::rc::Rc::new(ac_scene::Assets::empty()));
    for (context, text) in [(7, "A Stranger"), (8, "Blargerton Vassal B")] {
        c.world.confirmations.push(ac_world::Confirmation {
            kind: 1,
            context,
            text: text.into(),
        });
    }
    assert!(!c.accept_oath_from(&["Blargerton Vassal C".into()]));
    assert!(c.accept_oath_from(&["blargerton vassal b".into()]));
    assert_eq!(c.world.confirmations.len(), 1);
    assert_eq!(c.world.confirmations[0].text, "A Stranger");
}

#[test]
fn a_request_is_taken_once_off_the_board() {
    let mut board = Blackboard::default();
    ask(&mut board, " Blargerton ", 6);
    ask(&mut board, "Other", 2);
    let asked: Vec<Request> =
        serde_json::from_value(board.get(BUILD_KEY).unwrap().clone()).unwrap();
    assert_eq!(
        asked,
        vec![
            Request {
                target: "Blargerton".into(),
                rank: 6
            },
            Request {
                target: "Other".into(),
                rank: 2
            }
        ]
    );
}
