use crate::{Client, Event};

impl Client {
    /// The character `Config::character` names (case-insensitive), or the
    /// first on the account.
    pub(super) fn pick_character(&self) -> Option<&ac_net::messages::CharacterEntry> {
        match &self.config.character {
            Some(name) => self
                .characters
                .iter()
                .find(|c| c.name.eq_ignore_ascii_case(name)),
            None => self.characters.first(),
        }
    }

    /// Once the DDD exchange is done and the character list is known:
    /// send a held `enter_world`, enter with the configured (or first)
    /// character when auto-entering, or hand the list to the driver and
    /// wait. Runs again on every refreshed list.
    pub(super) fn lobby_ready(&mut self) {
        if !(self.ddd_done && self.characters_known) {
            return;
        }
        if self.enter_requested {
            return;
        }
        if self.entering.is_some() {
            self.send_enter_request();
            return;
        }
        let auto = self.config.auto_enter || self.config.character.is_some();
        let pick = if auto {
            self.pick_character().map(|c| c.id)
        } else {
            None
        };
        match pick {
            Some(id) => self.enter_world(id),
            None => {
                if auto && self.create_missing() {
                    // Being created from its spec; the answer enters. The
                    // list still goes out so a driver can show it.
                    self.events
                        .push(self::Event::Characters(self.characters.clone()));
                    return;
                }
                if auto {
                    match &self.config.character {
                        Some(name) => tracing::error!(
                            "no character named {name:?} on this account (have {:?})",
                            self.characters.iter().map(|c| &c.name).collect::<Vec<_>>()
                        ),
                        None => tracing::error!(
                            "no character on this account; create one (acclient/acbot --create)"
                        ),
                    }
                }
                self.events
                    .push(self::Event::Characters(self.characters.clone()));
            }
        }
    }
}
