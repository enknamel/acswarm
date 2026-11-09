//! The recalls the server makes for us, and the two other one-word game
//! actions the chat box already had. The rest of retail's travel commands are
//! [`PENDING`](super::PENDING).

use ac_net::messages::action;
use ac_world::object::pk_status;

use super::{Outcome, Refused};
use crate::Client;

pub(super) fn lifestone(c: &mut Client) -> Outcome {
    c.note_recall_sent(ac_world::recalls::position_type::SANCTUARY);
    c.session.send_action(action::TELE_TO_LIFESTONE, &[]);
    Ok(())
}

pub(super) fn house(c: &mut Client) -> Outcome {
    c.session.send_action(action::TELE_TO_HOUSE, &[]);
    Ok(())
}

pub(super) fn mansion(c: &mut Client) -> Outcome {
    c.session.send_action(action::TELE_TO_MANSION, &[]);
    Ok(())
}

pub(super) fn marketplace(c: &mut Client) -> Outcome {
    c.session.send_action(action::TELE_TO_MARKETPLACE, &[]);
    Ok(())
}

pub(super) fn hometown(c: &mut Client) -> Outcome {
    c.session
        .send_action(action::RECALL_ALLEGIANCE_HOMETOWN, &[]);
    Ok(())
}

pub(super) fn die(c: &mut Client) -> Outcome {
    c.session.send_action(action::DIE, &[]);
    Ok(())
}

pub(super) fn pk_lite(c: &mut Client) -> Outcome {
    c.session.send_action(action::ENTER_PK_LITE, &[]);
    Ok(())
}

/// To the arena a player killer fights in. Retail refused before sending
/// when it knew the character was not one, and so does this; the server
/// refuses again (`OnlyPKsMayUseCommand`, ACE `Player_Location.cs:480`).
pub(super) fn pk_arena(c: &mut Client) -> Outcome {
    let status = pk_of(c);
    if status != 0 && status != pk_status::PK {
        return Err(Refused::ours("only a player killer may use the PK Arena"));
    }
    c.session.send_action(action::TELE_TO_PK_ARENA, &[]);
    Ok(())
}

/// The same for PK Lite (`OnlyPKLiteMayUseCommand`, `Player_Location.cs:558`).
pub(super) fn pkl_arena(c: &mut Client) -> Outcome {
    let status = pk_of(c);
    if status != 0 && status != pk_status::PK_LITE {
        return Err(Refused::ours(
            "only a PK Lite character may use the PKL Arena",
        ));
    }
    c.session.send_action(action::TELE_TO_PKL_ARENA, &[]);
    Ok(())
}

/// Our own `PlayerKillerStatus`, 0 until the server has said which it is.
fn pk_of(c: &Client) -> u32 {
    c.pk_status()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    #[test]
    fn an_arena_is_for_its_own_kind_of_player_killer() {
        let mut c = testkit::offline_client();
        let sent = c.session.actions_sent();
        // Nothing said yet: retail sent anyway and let the server answer.
        assert_eq!(pk_arena(&mut c), Ok(()));
        assert_eq!(c.session.actions_sent(), sent + 1);
        c.world.player_guid = Some(testkit::ME);
        c.world.objects.entry(testkit::ME).or_default().pk_status = pk_status::NPK;
        assert!(pk_arena(&mut c).is_err(), "a non-PK is refused here");
        assert!(pkl_arena(&mut c).is_err());
        assert_eq!(c.session.actions_sent(), sent + 1, "neither went out");
        c.world.objects.entry(testkit::ME).or_default().pk_status = pk_status::PK_LITE;
        assert_eq!(pkl_arena(&mut c), Ok(()));
        assert!(pk_arena(&mut c).is_err());
    }

    #[test]
    fn a_status_never_updated_is_read_from_the_flags_we_came_in_with() {
        // An NPK all its life gets no PlayerKillerStatus update: its description says it.
        use ac_world::object_desc_flags as flags;
        let mut c = testkit::offline_client();
        assert_eq!(c.pk_status(), 0, "nothing known yet");
        c.world.player_guid = Some(testkit::ME);
        let me = c.world.objects.entry(testkit::ME).or_default();
        me.object_desc_flags = flags::PLAYER;
        assert_eq!(c.pk_status(), pk_status::NPK);
        let me = c.world.objects.entry(testkit::ME).or_default();
        me.object_desc_flags = flags::PLAYER | flags::PLAYER_KILLER;
        assert_eq!(c.pk_status(), pk_status::PK);
        // Made a player killer by a quest: the update is what stands, and the next plan reads it.
        let me = c.world.objects.entry(testkit::ME).or_default();
        me.object_desc_flags = flags::PLAYER;
        me.pk_status = pk_status::PK;
        assert_eq!(c.traveller().pk_status, pk_status::PK);
        assert_eq!(c.travel_prefs().who.pk_status, pk_status::PK);
    }
}
