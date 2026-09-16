//! The recalls the server makes for us, and the two other one-word game
//! actions the chat box already had. The rest of retail's travel commands are
//! [`PENDING`](super::PENDING).

use ac_net::messages::action;

use super::Outcome;
use crate::Client;

pub(super) fn lifestone(c: &mut Client) -> Outcome {
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
