use ac_net::messages::{self, event};

use crate::{Applied, World};

impl World {
    pub(super) fn apply_game_event(&mut self, op: u32, body: &[u8]) -> Applied {
        match messages::split_game_event(body) {
            Some((_, _, event::INVENTORY_PUT_OBJ_IN_CONTAINER, rest)) => {
                self.apply_put_in_container(rest)
            }
            Some((_, _, event::WIELD_OBJECT, rest)) => self.apply_wield_object(rest),
            Some((_, _, event::UPDATE_HEALTH, rest)) => self.apply_update_health(rest),
            Some((_, _, event::FELLOWSHIP_FULL_UPDATE, rest)) => {
                self.apply_fellowship_full_update(rest)
            }
            Some((_, _, event::FELLOWSHIP_UPDATE_FELLOW, rest)) => self.apply_fellow_update(rest),
            Some((_, _, event::FELLOWSHIP_DISBAND, _)) => self.apply_fellowship_disband(),
            Some((_, _, event::FELLOWSHIP_QUIT | event::FELLOWSHIP_DISMISS, rest)) => {
                self.apply_fellow_gone(rest)
            }
            Some((_, _, event::CONFIRMATION_REQUEST, rest)) => {
                self.apply_confirmation_request(rest)
            }
            Some((_, _, event::ALLEGIANCE_UPDATE, rest)) => self.apply_allegiance_update(rest),
            Some((_, _, event::ALLEGIANCE_INFO_RESPONSE, rest)) => self.apply_allegiance_info(rest),
            Some((_, _, event::ALLEGIANCE_LOGIN_NOTIFICATION, rest)) => {
                self.apply_allegiance_login(rest)
            }
            Some((_, _, event::ALLEGIANCE_UPDATE_DONE | event::ALLEGIANCE_UPDATE_ABORTED, _)) => {
                Applied::Ignored
            }
            Some((_, _, event::HOUSE_PROFILE, rest)) => self.apply_house_profile(rest),
            Some((_, _, event::HOUSE_DATA, rest)) => self.apply_house_data(rest),
            Some((_, _, event::HOUSE_STATUS, _)) => self.apply_house_status(),
            Some((_, _, event::UPDATE_HAR, rest)) => self.apply_update_har(rest),
            Some((_, _, event::UPDATE_RENT_TIME, rest)) => self.apply_rent_time(rest),
            Some((_, _, event::UPDATE_RENT_PAYMENT, rest)) => self.apply_rent_payment(rest),
            Some((_, _, event::FRIENDS_LIST_UPDATE, rest)) => self.apply_friends_update(rest),
            Some((_, _, event::CHARACTER_TITLE, rest)) => self.apply_character_title(rest),
            Some((_, _, event::UPDATE_TITLE, rest)) => self.apply_update_title(rest),
            Some((_, _, event::SET_SQUELCH_DB, rest)) => self.apply_squelch_db(rest),
            Some((_, _, event::CONFIRMATION_DONE, rest)) => self.apply_confirmation_done(rest),
            Some((_, _, event::REGISTER_TRADE, rest)) => self.apply_register_trade(rest),
            Some((_, _, event::CLOSE_TRADE, _)) => self.apply_close_trade(),
            Some((_, _, event::ADD_TO_TRADE, rest)) => self.apply_add_to_trade(rest),
            Some((_, _, event::REMOVE_FROM_TRADE, rest)) => self.apply_remove_from_trade(rest),
            Some((_, _, event::ACCEPT_TRADE, rest)) => self.apply_accept_trade(rest),
            Some((_, _, event::DECLINE_TRADE, rest)) => self.apply_decline_trade(rest),
            Some((_, _, event::RESET_TRADE, _)) => self.apply_reset_trade(),
            Some((_, _, event::CLEAR_TRADE_ACCEPTANCE, _)) => self.apply_clear_trade_acceptance(),
            Some((_, _, event::TRADE_FAILURE, rest)) => self.apply_trade_failure(rest),
            Some((_, _, event::VIEW_CONTENTS, rest)) => self.apply_view_contents(rest),
            Some((_, _, event::APPROACH_VENDOR, rest)) => self.apply_approach_vendor(rest),
            Some((_, _, event::CLOSE_GROUND_CONTAINER, _)) => self.apply_close_ground_container(),
            Some((_, _, event::QUERY_ITEM_MANA_RESPONSE, rest)) => self.apply_item_mana(rest),
            Some((_, _, event::INVENTORY_PUT_OBJECT_IN_3D, rest)) => self.apply_put_in_3d(rest),
            _ => self.apply_stats(op, body),
        }
    }
}
