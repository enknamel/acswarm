pub const POPUP_STRING: u32 = 0x0004;
pub const PLAYER_DESCRIPTION: u32 = 0x0013;
pub const INVENTORY_PUT_OBJ_IN_CONTAINER: u32 = 0x0022;
pub const WIELD_OBJECT: u32 = 0x0023;
pub const INVENTORY_PUT_OBJECT_IN_3D: u32 = 0x019A;
pub const VIEW_CONTENTS: u32 = 0x0196;
pub const FELLOWSHIP_QUIT: u32 = 0x00A3;
pub const FELLOWSHIP_DISMISS: u32 = 0x00A4;
pub const FELLOWSHIP_FULL_UPDATE: u32 = 0x02BE;
pub const FELLOWSHIP_DISBAND: u32 = 0x02BF;
pub const FELLOWSHIP_UPDATE_FELLOW: u32 = 0x02C0;
pub const CONFIRMATION_REQUEST: u32 = 0x0274;
pub const CONFIRMATION_DONE: u32 = 0x0276;
pub const ALLEGIANCE_UPDATE_ABORTED: u32 = 0x0003;
/// Friends: count, records (guid, online, appear offline, name, two
/// empty guid lists), then the update kind (0 full list, 1 added, 2
/// removed, 4 status changed).
pub const FRIENDS_LIST_UPDATE: u32 = 0x0021;
/// At login: 1, the shown title id, count, title ids.
pub const CHARACTER_TITLE: u32 = 0x0029;
/// A title granted or chosen: title id, shown flag.
pub const UPDATE_TITLE: u32 = 0x002B;
/// The squelch list: account table, character table, global mask.
pub const SET_SQUELCH_DB: u32 = 0x01F4;
pub const ALLEGIANCE_UPDATE: u32 = 0x0020;
pub const ALLEGIANCE_UPDATE_DONE: u32 = 0x01C8;
pub const ALLEGIANCE_LOGIN_NOTIFICATION: u32 = 0x027A;
pub const ALLEGIANCE_INFO_RESPONSE: u32 = 0x027C;
/// The Turbine chat rooms we are in: allegiance room (0 without
/// one), general, trade, lfg, roleplay, olthoi, society, three
/// society rooms; ten u32.
pub const SET_TURBINE_CHAT_CHANNELS: u32 = 0x0295;
/// A used book, sign or plaque: see `BookData`.
pub const BOOK_DATA_RESPONSE: u32 = 0x00B4;
/// One page's text: see `BookData::parse_page`.
pub const BOOK_PAGE_DATA_RESPONSE: u32 = 0x00B8;
/// Salvage results: skill, skipped guids, (material, workmanship, units) list, bonus.
pub const SALVAGE_OPERATIONS_RESULT: u32 = 0x02B4;
/// A house sign was used: slumlord guid, then the profile.
pub const HOUSE_PROFILE: u32 = 0x021D;
/// Our own house (answer to HouseQuery when we own one).
pub const HOUSE_DATA: u32 = 0x0225;
/// Answer to HouseQuery without a house: a WeenieError (0 = none).
pub const HOUSE_STATUS: u32 = 0x0226;
pub const UPDATE_RENT_TIME: u32 = 0x0227;
pub const UPDATE_RENT_PAYMENT: u32 = 0x0228;
/// Who may enter a house we are near: sequence, house guid, record.
pub const HOUSE_UPDATE_RESTRICTIONS: u32 = 0x0248;
/// Our house's access records (guest list), on request.
pub const UPDATE_HAR: u32 = 0x0257;
pub const HOUSE_TRANSACTION: u32 = 0x0259;
pub const AVAILABLE_HOUSES: u32 = 0x0271;
pub const REGISTER_TRADE: u32 = 0x01FD;
pub const OPEN_TRADE: u32 = 0x01FE;
pub const CLOSE_TRADE: u32 = 0x01FF;
pub const ADD_TO_TRADE: u32 = 0x0200;
pub const REMOVE_FROM_TRADE: u32 = 0x0201;
pub const ACCEPT_TRADE: u32 = 0x0202;
pub const DECLINE_TRADE: u32 = 0x0203;
pub const RESET_TRADE: u32 = 0x0205;
pub const TRADE_FAILURE: u32 = 0x0207;
pub const CLEAR_TRADE_ACCEPTANCE: u32 = 0x0208;
pub const CLOSE_GROUND_CONTAINER: u32 = 0x0052;
pub const APPROACH_VENDOR: u32 = 0x0062;
/// `u16 spell id, u16 layer`: a spell left the spellbook.
pub const MAGIC_REMOVE_SPELL: u32 = 0x01A8;
/// `u16 spell id, u16 layer`: a spell entered the spellbook.
pub const MAGIC_UPDATE_SPELL: u32 = 0x02C1;
/// One enchantment record (see `ac_world::stats::Enchantment`).
pub const MAGIC_UPDATE_ENCHANTMENT: u32 = 0x02C2;
/// `u16 spell id, u16 layer`.
pub const MAGIC_REMOVE_ENCHANTMENT: u32 = 0x02C3;
/// `u32 count`, then enchantment records.
pub const MAGIC_UPDATE_MULTIPLE_ENCHANTMENTS: u32 = 0x02C4;
/// `u32 count`, then `(u16 spell id, u16 layer)` pairs.
pub const MAGIC_REMOVE_MULTIPLE_ENCHANTMENTS: u32 = 0x02C5;
/// No body: every enchantment is gone.
pub const MAGIC_PURGE_ENCHANTMENTS: u32 = 0x02C6;
/// `u16 spell id, u16 layer`: removed by a dispel.
pub const MAGIC_DISPEL_ENCHANTMENT: u32 = 0x02C7;
/// `u32 count`, then `(u16 spell id, u16 layer)` pairs.
pub const MAGIC_DISPEL_MULTIPLE_ENCHANTMENTS: u32 = 0x02C8;
/// No body: the harmful enchantments are gone (sent on death).
pub const MAGIC_PURGE_BAD_ENCHANTMENTS: u32 = 0x0312;
pub const ATTACK_DONE: u32 = 0x01A7;
pub const VICTIM_NOTIFICATION: u32 = 0x01AC;
pub const KILLER_NOTIFICATION: u32 = 0x01AD;
pub const ATTACKER_NOTIFICATION: u32 = 0x01B1;
pub const DEFENDER_NOTIFICATION: u32 = 0x01B2;
pub const EVASION_ATTACKER_NOTIFICATION: u32 = 0x01B3;
pub const EVASION_DEFENDER_NOTIFICATION: u32 = 0x01B4;
pub const UPDATE_HEALTH: u32 = 0x01C0;
pub const CHANNEL_BROADCAST: u32 = 0x0147;
pub const IDENTIFY_OBJECT_RESPONSE: u32 = 0x00C9;
pub const USE_DONE: u32 = 0x01C7;
pub const EMOTE: u32 = 0x01E2;
pub const WEENIE_ERROR: u32 = 0x028A;
pub const WEENIE_ERROR_WITH_STRING: u32 = 0x028B;
pub const TRANSIENT_STRING: u32 = 0x02EB;
pub const TELL: u32 = 0x02BD;
/// The barber (appearance) window's data: 13 data ids and two
/// option words.
pub const START_BARBER: u32 = 0x0075;
/// `u32 item guid, u32 WeenieError`: a pickup, drop, wield or split
/// the server refused.
pub const INVENTORY_SERVER_SAVE_FAILED: u32 = 0x00A0;
/// `u32 book, i32 page, u32 success` each.
pub const BOOK_MODIFY_PAGE_RESPONSE: u32 = 0x00B5;
pub const BOOK_ADD_PAGE_RESPONSE: u32 = 0x00B6;
pub const BOOK_DELETE_PAGE_RESPONSE: u32 = 0x00B7;
/// `u32 item, string16 inscription, u32 scribe guid, string16 scribe
/// name, string16 scribe account`.
pub const GET_INSCRIPTION_RESPONSE: u32 = 0x00C3;
/// `u32 count, string16 names`: who is in a chat channel.
pub const CHANNEL_LIST: u32 = 0x0148;
/// `u32 count, string16 channel names`: the admin channels open to us.
pub const CHANNEL_INDEX: u32 = 0x0149;
/// No body: our attack animation started (after the walk up to the
/// target).
pub const COMBAT_COMMENCE_ATTACK: u32 = 0x01B8;
/// `string16 name, string16 age`.
pub const QUERY_AGE_RESPONSE: u32 = 0x01C3;
/// No body: the fellowship panel's refresh is complete.
pub const FELLOWSHIP_FELLOW_UPDATE_DONE: u32 = 0x01C9;
pub const FELLOWSHIP_FELLOW_STATS_DONE: u32 = 0x01CA;
pub const ITEM_APPRAISE_DONE: u32 = 0x01CB;
/// No body: the answer to PingRequest.
pub const PING_RESPONSE: u32 = 0x01EA;
/// `u32 item, f32 mana fraction, u32 success`.
pub const QUERY_ITEM_MANA_RESPONSE: u32 = 0x0264;
pub const JOIN_GAME_RESPONSE: u32 = 0x0281;
pub const START_GAME: u32 = 0x0282;
pub const MOVE_RESPONSE: u32 = 0x0283;
pub const OPPONENT_TURN: u32 = 0x0284;
pub const OPPONENT_STALEMATE: u32 = 0x0285;
pub const GAME_OVER: u32 = 0x028C;
pub const ADMIN_QUERY_PLUGIN_LIST: u32 = 0x02AE;
pub const ADMIN_QUERY_PLUGIN: u32 = 0x02B1;
pub const ADMIN_QUERY_PLUGIN_RESPONSE: u32 = 0x02B3;
/// `f32 extent`: a portal storm is gathering around us.
pub const PORTAL_STORM_BREWING: u32 = 0x02C9;
pub const PORTAL_STORM_IMMINENT: u32 = 0x02CA;
/// No body: the storm took us (a teleport follows).
pub const PORTAL_STORM: u32 = 0x02CB;
pub const PORTAL_STORM_SUBSIDED: u32 = 0x02CC;
/// The quest contract tracker table at login, and one contract.
pub const CONTRACT_TRACKER_TABLE: u32 = 0x0314;
pub const CONTRACT_TRACKER: u32 = 0x0315;

/// ACE's name for a GameEvent type, for logs and the coverage
/// audit; `None` for one ACE does not define.
pub fn name(ev: u32) -> Option<&'static str> {
    Some(match ev {
        ALLEGIANCE_UPDATE_ABORTED => "AllegianceUpdateAborted",
        POPUP_STRING => "PopupString",
        PLAYER_DESCRIPTION => "PlayerDescription",
        ALLEGIANCE_UPDATE => "AllegianceUpdate",
        FRIENDS_LIST_UPDATE => "FriendsListUpdate",
        INVENTORY_PUT_OBJ_IN_CONTAINER => "InventoryPutObjInContainer",
        WIELD_OBJECT => "WieldObject",
        CHARACTER_TITLE => "CharacterTitle",
        UPDATE_TITLE => "UpdateTitle",
        CLOSE_GROUND_CONTAINER => "CloseGroundContainer",
        APPROACH_VENDOR => "ApproachVendor",
        START_BARBER => "StartBarber",
        INVENTORY_SERVER_SAVE_FAILED => "InventoryServerSaveFailed",
        FELLOWSHIP_QUIT => "FellowshipQuit",
        FELLOWSHIP_DISMISS => "FellowshipDismiss",
        BOOK_DATA_RESPONSE => "BookDataResponse",
        BOOK_MODIFY_PAGE_RESPONSE => "BookModifyPageResponse",
        BOOK_ADD_PAGE_RESPONSE => "BookAddPageResponse",
        BOOK_DELETE_PAGE_RESPONSE => "BookDeletePageResponse",
        BOOK_PAGE_DATA_RESPONSE => "BookPageDataResponse",
        GET_INSCRIPTION_RESPONSE => "GetInscriptionResponse",
        IDENTIFY_OBJECT_RESPONSE => "IdentifyObjectResponse",
        CHANNEL_BROADCAST => "ChannelBroadcast",
        CHANNEL_LIST => "ChannelList",
        CHANNEL_INDEX => "ChannelIndex",
        VIEW_CONTENTS => "ViewContents",
        INVENTORY_PUT_OBJECT_IN_3D => "InventoryPutObjectIn3D",
        ATTACK_DONE => "AttackDone",
        MAGIC_REMOVE_SPELL => "MagicRemoveSpell",
        VICTIM_NOTIFICATION => "VictimNotification",
        KILLER_NOTIFICATION => "KillerNotification",
        ATTACKER_NOTIFICATION => "AttackerNotification",
        DEFENDER_NOTIFICATION => "DefenderNotification",
        EVASION_ATTACKER_NOTIFICATION => "EvasionAttackerNotification",
        EVASION_DEFENDER_NOTIFICATION => "EvasionDefenderNotification",
        COMBAT_COMMENCE_ATTACK => "CombatCommenceAttack",
        UPDATE_HEALTH => "UpdateHealth",
        QUERY_AGE_RESPONSE => "QueryAgeResponse",
        USE_DONE => "UseDone",
        ALLEGIANCE_UPDATE_DONE => "AllegianceAllegianceUpdateDone",
        FELLOWSHIP_FELLOW_UPDATE_DONE => "FellowshipFellowUpdateDone",
        FELLOWSHIP_FELLOW_STATS_DONE => "FellowshipFellowStatsDone",
        ITEM_APPRAISE_DONE => "ItemAppraiseDone",
        EMOTE => "Emote",
        PING_RESPONSE => "PingResponse",
        SET_SQUELCH_DB => "SetSquelchDB",
        REGISTER_TRADE => "RegisterTrade",
        OPEN_TRADE => "OpenTrade",
        CLOSE_TRADE => "CloseTrade",
        ADD_TO_TRADE => "AddToTrade",
        REMOVE_FROM_TRADE => "RemoveFromTrade",
        ACCEPT_TRADE => "AcceptTrade",
        DECLINE_TRADE => "DeclineTrade",
        RESET_TRADE => "ResetTrade",
        TRADE_FAILURE => "TradeFailure",
        CLEAR_TRADE_ACCEPTANCE => "ClearTradeAcceptance",
        HOUSE_PROFILE => "HouseProfile",
        HOUSE_DATA => "HouseData",
        HOUSE_STATUS => "HouseStatus",
        UPDATE_RENT_TIME => "UpdateRentTime",
        UPDATE_RENT_PAYMENT => "UpdateRentPayment",
        HOUSE_UPDATE_RESTRICTIONS => "HouseUpdateRestrictions",
        UPDATE_HAR => "UpdateHAR",
        HOUSE_TRANSACTION => "HouseTransaction",
        QUERY_ITEM_MANA_RESPONSE => "QueryItemManaResponse",
        AVAILABLE_HOUSES => "AvailableHouses",
        CONFIRMATION_REQUEST => "CharacterConfirmationRequest",
        CONFIRMATION_DONE => "CharacterConfirmationDone",
        ALLEGIANCE_LOGIN_NOTIFICATION => "AllegianceLoginNotification",
        ALLEGIANCE_INFO_RESPONSE => "AllegianceInfoResponse",
        JOIN_GAME_RESPONSE => "JoinGameResponse",
        START_GAME => "StartGame",
        MOVE_RESPONSE => "MoveResponse",
        OPPONENT_TURN => "OpponentTurn",
        OPPONENT_STALEMATE => "OpponentStalemate",
        WEENIE_ERROR => "WeenieError",
        WEENIE_ERROR_WITH_STRING => "WeenieErrorWithString",
        GAME_OVER => "GameOver",
        SET_TURBINE_CHAT_CHANNELS => "SetTurbineChatChannels",
        ADMIN_QUERY_PLUGIN_LIST => "AdminQueryPluginList",
        ADMIN_QUERY_PLUGIN => "AdminQueryPlugin",
        ADMIN_QUERY_PLUGIN_RESPONSE => "AdminQueryPluginResponse",
        SALVAGE_OPERATIONS_RESULT => "SalvageOperationsResult",
        TELL => "Tell",
        FELLOWSHIP_FULL_UPDATE => "FellowshipFullUpdate",
        FELLOWSHIP_DISBAND => "FellowshipDisband",
        FELLOWSHIP_UPDATE_FELLOW => "FellowshipUpdateFellow",
        MAGIC_UPDATE_SPELL => "MagicUpdateSpell",
        MAGIC_UPDATE_ENCHANTMENT => "MagicUpdateEnchantment",
        MAGIC_REMOVE_ENCHANTMENT => "MagicRemoveEnchantment",
        MAGIC_UPDATE_MULTIPLE_ENCHANTMENTS => "MagicUpdateMultipleEnchantments",
        MAGIC_REMOVE_MULTIPLE_ENCHANTMENTS => "MagicRemoveMultipleEnchantments",
        MAGIC_PURGE_ENCHANTMENTS => "MagicPurgeEnchantments",
        MAGIC_DISPEL_ENCHANTMENT => "MagicDispelEnchantment",
        MAGIC_DISPEL_MULTIPLE_ENCHANTMENTS => "MagicDispelMultipleEnchantments",
        PORTAL_STORM_BREWING => "MiscPortalStormBrewing",
        PORTAL_STORM_IMMINENT => "MiscPortalStormImminent",
        PORTAL_STORM => "MiscPortalStorm",
        PORTAL_STORM_SUBSIDED => "MiscPortalstormSubsided",
        TRANSIENT_STRING => "CommunicationTransientString",
        MAGIC_PURGE_BAD_ENCHANTMENTS => "MagicPurgeBadEnchantments",
        CONTRACT_TRACKER_TABLE => "SendClientContractTrackerTable",
        CONTRACT_TRACKER => "SendClientContractTracker",
        _ => return None,
    })
}
