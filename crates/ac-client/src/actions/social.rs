use crate::{emotes, Client, Event};

impl Client {
    /// Do a soul emote by word ("wave", "bow", "*cheer*"): play the
    /// motion, send it in the next movement state so others see it, and
    /// say the line (SoulEmote 0x01E1: "waves"). False for unknown words.
    pub fn emote(&mut self, words: &str) -> bool {
        let Some((cmd, text)) = emotes::lookup(words) else {
            return false;
        };
        if let Some(pl) = self.player.as_mut() {
            pl.play_command(&self.assets, cmd, 1.0);
            pl.queue_command(cmd);
        }
        let mut w = ac_net::wire::Writer::new();
        w.string16(text);
        self.session
            .send_action(ac_net::messages::action::SOUL_EMOTE, &w.finish());
        true
    }

    /// Found a fellowship (FellowshipCreate 0x00A2: name, share XP).
    pub fn fellowship_create(&mut self, name: &str, share_xp: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.string16(name.trim()).u32(u32::from(share_xp));
        self.session
            .send_action(ac_net::messages::action::FELLOWSHIP_CREATE, &w.finish());
    }

    /// Invite a player (FellowshipRecruit 0x00A5); they get a
    /// confirmation to answer.
    pub fn fellowship_recruit(&mut self, player: u32) {
        self.session.send_action(
            ac_net::messages::action::FELLOWSHIP_RECRUIT,
            &player.to_le_bytes(),
        );
    }

    /// Leave the fellowship; the leader may disband it instead.
    pub fn fellowship_quit(&mut self, disband: bool) {
        self.session.send_action(
            ac_net::messages::action::FELLOWSHIP_QUIT,
            &u32::from(disband).to_le_bytes(),
        );
    }

    /// Remove a member (leader only).
    pub fn fellowship_dismiss(&mut self, player: u32) {
        self.session.send_action(
            ac_net::messages::action::FELLOWSHIP_DISMISS,
            &player.to_le_bytes(),
        );
    }

    /// Answer a server confirmation (ConfirmationResponse 0x0275).
    pub fn confirm(&mut self, kind: u32, context: u32, yes: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.i32(kind as i32).u32(context).i32(i32::from(yes));
        self.session
            .send_action(ac_net::messages::action::CONFIRMATION_RESPONSE, &w.finish());
        self.world
            .confirmations
            .retain(|c| !(c.kind == kind && c.context == context));
    }

    /// Add a friend by name (AddFriend 0x0018); the server answers with a
    /// FriendsListUpdate or "X not found" in chat.
    pub fn add_friend(&mut self, name: &str) {
        let mut w = ac_net::wire::Writer::new();
        w.string16(name.trim());
        self.session
            .send_action(ac_net::messages::action::ADD_FRIEND, &w.finish());
    }

    /// Drop a friend (RemoveFriend 0x0017: guid), or everyone with
    /// `None` (RemoveAllFriends 0x0025).
    pub fn remove_friend(&mut self, guid: Option<u32>) {
        match guid {
            Some(g) => self
                .session
                .send_action(ac_net::messages::action::REMOVE_FRIEND, &g.to_le_bytes()),
            None => self
                .session
                .send_action(ac_net::messages::action::REMOVE_ALL_FRIENDS, &[]),
        }
    }

    /// Show one of our titles (TitleSet 0x002C); the server confirms
    /// with UpdateTitle.
    pub fn set_title(&mut self, title: u32) {
        self.session
            .send_action(ac_net::messages::action::TITLE_SET, &title.to_le_bytes());
    }

    /// Squelch or unsquelch a character (ModifyCharacterSquelch 0x0058:
    /// flag, guid, name, ChatMessageType; guid 0 with a name looks the
    /// player up, `squelch::ALL` for every channel), their whole account
    /// (ModifyAccountSquelch 0x0059: flag, name), or a chat type from
    /// everyone (ModifyGlobalSquelch 0x005B: flag, ChatMessageType). The
    /// server confirms in chat and re-sends the squelch list.
    pub fn squelch(&mut self, guid: u32, name: &str, kind: u32, on: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.u32(u32::from(on))
            .u32(guid)
            .string16(name.trim())
            .u32(kind);
        self.session.send_action(
            ac_net::messages::action::MODIFY_CHARACTER_SQUELCH,
            &w.finish(),
        );
    }

    pub fn squelch_account(&mut self, name: &str, on: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.u32(u32::from(on)).string16(name.trim());
        self.session.send_action(
            ac_net::messages::action::MODIFY_ACCOUNT_SQUELCH,
            &w.finish(),
        );
    }

    pub fn squelch_global(&mut self, chat_type: u32, on: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.u32(u32::from(on)).u32(chat_type);
        self.session
            .send_action(ac_net::messages::action::MODIFY_GLOBAL_SQUELCH, &w.finish());
    }

    /// Ask the server about our house (HouseQuery 0x021E): HouseData
    /// when we own one, HouseStatus when not.
    pub fn house_query(&mut self) {
        self.session
            .send_action(ac_net::messages::action::HOUSE_QUERY, &[]);
    }

    /// The carried items that cover a payment list, largest stacks
    /// first: guids of stacks of each wanted weenie until the
    /// outstanding amount is covered. `None` when something is short.
    pub fn payment_items(&self, payments: &[ac_world::housing::Payment]) -> Option<Vec<u32>> {
        let me = self.world.player_guid;
        let mut out = Vec::new();
        for p in payments {
            let mut need = p.outstanding();
            if need == 0 {
                continue;
            }
            let mut stacks: Vec<&ac_world::WorldObject> = self
                .world
                .inventory()
                .filter(|o| o.weenie_class_id == p.wcid && o.wielder != me)
                .collect();
            stacks.sort_by_key(|o| std::cmp::Reverse(o.stack_size));
            for o in stacks {
                if need == 0 {
                    break;
                }
                out.push(o.guid);
                need = need.saturating_sub(o.stack_size.max(1));
            }
            if need > 0 {
                return None;
            }
        }
        Some(out)
    }

    /// Buy the house whose sign we last used (BuyHouse 0x021C: slumlord,
    /// item guid list), paying with what the pack holds. False when the
    /// profile is missing, the house is owned, or an item is short; the
    /// server answers "Congratulations!  You now own this dwelling." and
    /// the house data, or a refusal in chat.
    pub fn buy_house(&mut self) -> bool {
        let Some(p) = self.world.house_profile.clone() else {
            return false;
        };
        if p.owner != 0 {
            return false;
        }
        let Some(items) = self.payment_items(&p.buy) else {
            tracing::info!("buy house: missing purchase items");
            return false;
        };
        tracing::info!(
            "buy house at {:#010x} with {} items",
            p.slumlord,
            items.len()
        );
        let mut w = ac_net::wire::Writer::new();
        w.u32(p.slumlord).u32(items.len() as u32);
        for g in &items {
            w.u32(*g);
        }
        self.session
            .send_action(ac_net::messages::action::BUY_HOUSE, &w.finish());
        true
    }

    /// Pay the maintenance of the house whose sign we last used
    /// (RentHouse 0x0221), with what the pack holds toward what is still
    /// outstanding. Anyone may pay; the sign must be in the landblock.
    pub fn rent_house(&mut self) -> bool {
        let Some(p) = self.world.house_profile.clone() else {
            return false;
        };
        let Some(items) = self.payment_items(&p.rent) else {
            tracing::info!("rent house: missing items");
            return false;
        };
        if items.is_empty() {
            return false;
        }
        tracing::info!(
            "pay rent at {:#010x} with {} items",
            p.slumlord,
            items.len()
        );
        let mut w = ac_net::wire::Writer::new();
        w.u32(p.slumlord).u32(items.len() as u32);
        for g in &items {
            w.u32(*g);
        }
        self.session
            .send_action(ac_net::messages::action::RENT_HOUSE, &w.finish());
        true
    }

    /// Give the house up (AbandonHouse 0x021F); the server boots
    /// everyone, clears the guest list and answers HouseStatus.
    pub fn abandon_house(&mut self) {
        self.session
            .send_action(ac_net::messages::action::ABANDON_HOUSE, &[]);
    }

    /// Ask for our house's guest list (RequestFullGuestList 0x024D);
    /// it lands in `world.house_access`.
    pub fn house_guest_list(&mut self) {
        self.session
            .send_action(ac_net::messages::action::REQUEST_FULL_GUEST_LIST, &[]);
    }

    /// Add or remove a guest by name (AddPermanentGuest 0x0245 /
    /// RemovePermanentGuest 0x0246); the server confirms in chat, and
    /// the guest list is asked for again.
    pub fn house_guest(&mut self, name: &str, add: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.string16(name.trim());
        let op = if add {
            ac_net::messages::action::ADD_PERMANENT_GUEST
        } else {
            ac_net::messages::action::REMOVE_PERMANENT_GUEST
        };
        self.session.send_action(op, &w.finish());
        self.house_guest_list();
    }

    /// Let a guest use the storage chests, or not (ChangeStoragePermission
    /// 0x0249: name, flag).
    pub fn house_storage(&mut self, name: &str, allow: bool) {
        let mut w = ac_net::wire::Writer::new();
        w.string16(name.trim()).u32(u32::from(allow));
        self.session.send_action(
            ac_net::messages::action::CHANGE_STORAGE_PERMISSION,
            &w.finish(),
        );
        self.house_guest_list();
    }

    /// Open the house to everyone, or make it private (SetOpenHouseStatus
    /// 0x0247).
    pub fn house_open(&mut self, open: bool) {
        self.session.send_action(
            ac_net::messages::action::SET_OPEN_HOUSE_STATUS,
            &u32::from(open).to_le_bytes(),
        );
        self.house_guest_list();
    }

    /// Give or take the allegiance's access to the house, or to its
    /// storage (ModifyAllegianceGuestPermission 0x0267 /
    /// ModifyAllegianceStoragePermission 0x0268).
    pub fn house_allegiance(&mut self, storage: bool, add: bool) {
        let op = if storage {
            ac_net::messages::action::MODIFY_ALLEGIANCE_STORAGE_PERMISSION
        } else {
            ac_net::messages::action::MODIFY_ALLEGIANCE_GUEST_PERMISSION
        };
        self.session.send_action(op, &u32::from(add).to_le_bytes());
        self.house_guest_list();
    }

    /// Throw a visitor out by name (BootSpecificHouseGuest 0x024A), or
    /// everyone with an empty name (BootEveryone 0x025F).
    pub fn house_boot(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.session
                .send_action(ac_net::messages::action::BOOT_EVERYONE, &[]);
            return;
        }
        let mut w = ac_net::wire::Writer::new();
        w.string16(name);
        self.session.send_action(
            ac_net::messages::action::BOOT_SPECIFIC_HOUSE_GUEST,
            &w.finish(),
        );
    }

    /// Clear the guest list (RemoveAllPermanentGuests 0x025E) or every
    /// storage permission (RemoveAllStoragePermission 0x024C).
    pub fn house_clear(&mut self, storage_only: bool) {
        let op = if storage_only {
            ac_net::messages::action::REMOVE_ALL_STORAGE_PERMISSION
        } else {
            ac_net::messages::action::REMOVE_ALL_PERMANENT_GUESTS
        };
        self.session.send_action(op, &[]);
        self.house_guest_list();
    }

    /// Say something in a Turbine chat room (message 0xF7DE): General,
    /// Trade, LFG, Roleplay, or `turbine::ALLEGIANCE` for our
    /// allegiance's own room. Everyone in the room, us included, gets
    /// the line back. False without an allegiance room to speak in.
    pub fn turbine_say(&mut self, room: u32, text: &str) -> bool {
        let text = text.trim();
        if text.is_empty() {
            return false;
        }
        // Two rooms stand for one of ours, whose id the server named in
        // SetTurbineChatChannels; the rest are the same for everyone.
        let room = match room {
            ac_net::messages::turbine::ALLEGIANCE if self.allegiance_room == 0 => {
                self.events.push(Event::Chat {
                    text: "You are not in an allegiance.".into(),
                    kind: 0,
                });
                return false;
            }
            ac_net::messages::turbine::ALLEGIANCE => self.allegiance_room,
            ac_net::messages::turbine::SOCIETY if self.society_room == 0 => {
                self.events.push(Event::Chat {
                    text: "You do not belong to a society.".into(),
                    kind: 0,
                });
                return false;
            }
            ac_net::messages::turbine::SOCIETY => self.society_room,
            other => other,
        };
        let me = self.world.player_guid.unwrap_or(0);
        let context = self.turbine_context;
        self.turbine_context = (self.turbine_context % 0x70) + 1;
        let msg = ac_net::messages::turbine::encode(room, me, text, context);
        self.session
            .send_message(ac_net::messages::queue::WEENIE, msg);
        true
    }

    /// Say something on a group channel (ChatChannel 0x0147): fellowship,
    /// vassals, patron, monarch or co-vassals (`ac_net::messages::channel`).
    /// Everyone on it, us included, hears it as a ChannelBroadcast.
    pub fn chat_channel(&mut self, channel: u32, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let mut w = ac_net::wire::Writer::new();
        w.u32(channel).string16(text);
        self.session
            .send_action(ac_net::messages::action::CHAT_CHANNEL, &w.finish());
    }

    /// Ask the server for our allegiance profile (AllegianceUpdateRequest
    /// 0x001F); it answers with AllegianceUpdate. `panel` is what the
    /// original client sent when its panel opened; ACE ignores it.
    pub fn allegiance_update_request(&mut self, panel: bool) {
        self.session.send_action(
            ac_net::messages::action::ALLEGIANCE_UPDATE_REQUEST,
            &u32::from(panel).to_le_bytes(),
        );
    }

    /// Swear allegiance to a player in reach (SwearAllegiance 0x001D).
    /// The server walks us over, asks the patron (a kind 1
    /// confirmation) and, on yes, sends both an AllegianceUpdate. Refused
    /// when we already have a patron, they ignore allegiance requests,
    /// have 11 vassals, or are our own vassal.
    pub fn swear_allegiance(&mut self, patron: u32) -> bool {
        let Some(o) = self.world.objects.get(&patron) else {
            return false;
        };
        if o.object_desc_flags & ac_world::object_desc_flags::PLAYER == 0
            || Some(patron) == self.world.player_guid
        {
            return false;
        }
        tracing::info!("swear allegiance to {} ({patron:#010x})", o.name);
        self.session.send_action(
            ac_net::messages::action::SWEAR_ALLEGIANCE,
            &patron.to_le_bytes(),
        );
        true
    }

    /// Break with our patron or one of our vassals (BreakAllegiance
    /// 0x001E); they need not be online or in view.
    pub fn break_allegiance(&mut self, member: u32) -> bool {
        let known = self
            .world
            .allegiance
            .as_ref()
            .map(|a| {
                a.patron.as_ref().map(|p| p.guid) == Some(member)
                    || a.vassals.iter().any(|v| v.guid == member)
            })
            .unwrap_or(false);
        if !known {
            return false;
        }
        tracing::info!("break allegiance with {member:#010x}");
        self.session.send_action(
            ac_net::messages::action::BREAK_ALLEGIANCE,
            &member.to_le_bytes(),
        );
        true
    }

    /// Ask for another member's profile by name (AllegianceInfoRequest
    /// 0x027B; officers only). The answer lands in
    /// `world.allegiance_info`.
    pub fn allegiance_info_request(&mut self, name: &str) {
        let mut w = ac_net::wire::Writer::new();
        w.string16(name.trim());
        self.session.send_action(
            ac_net::messages::action::ALLEGIANCE_INFO_REQUEST,
            &w.finish(),
        );
    }

    /// Name the allegiance (SetAllegianceName 0x0033, monarch only) or
    /// clear the name with an empty string (ClearAllegianceName 0x0031).
    /// The server answers in chat and re-sends the profile.
    pub fn set_allegiance_name(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            self.session
                .send_action(ac_net::messages::action::CLEAR_ALLEGIANCE_NAME, &[]);
            return;
        }
        let mut w = ac_net::wire::Writer::new();
        w.string16(name);
        self.session
            .send_action(ac_net::messages::action::SET_ALLEGIANCE_NAME, &w.finish());
        // The server only answers in chat; ask for the renamed profile.
        self.allegiance_update_request(true);
    }

    /// Set (SetMotd 0x0254) or, with an empty string, clear (ClearMotd
    /// 0x0256) the allegiance message of the day; officers only.
    pub fn set_allegiance_motd(&mut self, motd: &str) {
        let motd = motd.trim();
        if motd.is_empty() {
            self.session
                .send_action(ac_net::messages::action::CLEAR_MOTD, &[]);
            return;
        }
        let mut w = ac_net::wire::Writer::new();
        w.string16(motd);
        self.session
            .send_action(ac_net::messages::action::SET_MOTD, &w.finish());
        self.allegiance_update_request(true);
    }

    pub fn say(&mut self, text: &str) {
        if emotes::from_chat_line(text).is_some() && self.emote(text) {
            return;
        }
        let mut w = ac_net::wire::Writer::new();
        w.string16(text);
        self.session
            .send_action(ac_net::messages::action::TALK, &w.finish());
    }
}
