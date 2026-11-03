//! Sample quests for `--demo-ui`: the eater jaws and the augmentation chain, a rat tail and a
//! kill task, each with its browse state, two of them pursued.

use ac_client::autoplay::{PursuitLine, PursuitStep};
use ac_client::quest_log::{QuestLine, QuestState};
use ac_world::quest_chain::QuestCard;
use ac_world::quests::{Kind, Repeat};

use super::quest_filter::WEEK;

/// Sample quests for `--demo-ui`: the eater jaws and the augmentation chain, a rat tail, a kill
/// task, each with its browse state and two pursued.
pub(crate) struct Demo {
    pub cards: Vec<QuestCard>,
    pub browse: Vec<QuestLine>,
    pub pursuit: Vec<PursuitLine>,
}

#[allow(clippy::too_many_arguments)] // a card is its fields
pub(crate) fn demo_card(
    id: &str,
    title: &str,
    kind: Kind,
    item: Option<&str>,
    giver: (&str, &str),
    creature: Option<(&str, u32, &str)>,
    xp: u64,
    repeat: Repeat,
) -> QuestCard {
    QuestCard {
        id: id.into(),
        title: title.into(),
        kind,
        item: item.map(str::to_string),
        givers: vec![(giver.0.into(), giver.1.into())],
        creatures: creature
            .iter()
            .map(|c| (c.0.to_string(), c.1, 1.0))
            .collect(),
        places: creature.iter().map(|c| c.2.to_string()).collect(),
        levels: (0, 0),
        easiest: creature.map(|c| c.1),
        entry: 0,
        xp,
        gives: Vec::new(),
        repeat,
        flags: Vec::new(),
        does: if xp > 0 {
            vec!["xp".into()]
        } else {
            Vec::new()
        },
        unread: Vec::new(),
    }
}

/// The demo's quests, browse states and pursuit lines at `now` (Unix seconds).
pub(crate) fn demo(now: u64) -> Demo {
    let mut cards = vec![
        demo_card(
            "augmentationprovinggroundaccess",
            "Augmentation Proving Ground Access",
            Kind::Item,
            Some("Insatiable Eater Jaw"),
            ("Fiun Rehlyun", "Fiun Outpost"),
            Some(("Insatiable Eater", 160, "Insatiable Vault")),
            0,
            Repeat::Any,
        ),
        demo_card(
            "engorgedeaterjaw",
            "Engorged Eater Jaw",
            Kind::Item,
            Some("Engorged Eater Jaw"),
            ("Clemenza", "Westwatch"),
            Some(("Engorged Eater", 80, "Remote Empyrean Vault")),
            2_500_000,
            Repeat::After(72_000),
        ),
        demo_card(
            "ginsword",
            "Sir Ginazio's Sword",
            Kind::Talk,
            None,
            ("Sir Ginazio", "near Silyun"),
            None,
            0,
            Repeat::Once,
        ),
        demo_card(
            "give:3684",
            "Red Rat Tail",
            Kind::Item,
            Some("Red Rat Tail"),
            ("Yukara Miko the Collector", "Shoushi"),
            Some(("Red Rat", 3, "Shoushi outskirts")),
            1_000,
            Repeat::Any,
        ),
        demo_card(
            "repugeaterkillcount",
            "Repugnant Eater Kill Task",
            Kind::Kill,
            None,
            ("Xun Yu", "Yanshi"),
            Some(("Repugnant Eater", 95, "Repugnant Eater Lair")),
            70_000_000,
            Repeat::After(WEEK),
        ),
    ];
    cards[0].does = vec!["flag".into(), "takes".into()];
    cards[0].entry = 125;
    cards[1].gives = vec!["Trade Note (75,000) x1".into()];
    cards[2].does = vec!["flag".into()];
    cards[2].unread = vec!["a check of type 75".into()];
    cards[4].levels = (80, 0);
    let line = |id: &str, title: &str, kind, giver: &str, xp, state| QuestLine {
        id: id.into(),
        title: title.into(),
        kind,
        state,
        xp,
        givers: 1,
        giver: giver.into(),
        on: true,
        every_way: true,
    };
    let browse = vec![
        line(
            "augmentationprovinggroundaccess",
            "Augmentation Proving Ground Access",
            Kind::Item,
            "Fiun Rehlyun",
            0,
            QuestState::Needs {
                more: 1,
                of: "Insatiable Eater Jaw".into(),
            },
        ),
        line(
            "engorgedeaterjaw",
            "Engorged Eater Jaw",
            Kind::Item,
            "Clemenza",
            2_500_000,
            QuestState::Waiting {
                until: now + 5 * 3_600 + 12 * 60,
            },
        ),
        line(
            "ginsword",
            "Sir Ginazio's Sword",
            Kind::Talk,
            "Sir Ginazio",
            0,
            QuestState::Done,
        ),
        line(
            "give:3684",
            "Red Rat Tail",
            Kind::Item,
            "Yukara Miko the Collector",
            1_000,
            QuestState::Ready,
        ),
        line(
            "repugeaterkillcount",
            "Repugnant Eater Kill Task",
            Kind::Kill,
            "Xun Yu",
            70_000_000,
            QuestState::NotTaken {
                from: "Xun Yu".into(),
            },
        ),
    ];
    let pursuit = vec![
        PursuitLine {
            id: "augmentationprovinggroundaccess".into(),
            title: "Augmentation Proving Ground Access".into(),
            step: PursuitStep::Hunt {
                landblock: 0x0096,
                creature: "Insatiable Eater".into(),
                place: "Insatiable Vault".into(),
                more: 1,
            },
            working: true,
            via: None,
        },
        PursuitLine {
            id: "engorgedeaterjaw".into(),
            title: "Engorged Eater Jaw".into(),
            step: PursuitStep::Waiting {
                until: now + 5 * 3_600 + 12 * 60,
            },
            working: false,
            via: None,
        },
    ];
    Demo {
        cards,
        browse,
        pursuit,
    }
}
