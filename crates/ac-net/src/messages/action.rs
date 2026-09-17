pub const TALK: u32 = 0x0015;
/// Tell one player in view by guid: text, target guid. The server only
/// looks the guid up in our own landblock (ACE
/// GameActionTalkDirect.cs:21), so `TELL` by name reaches further.
pub const TALK_DIRECT: u32 = 0x0032;
pub const USE_WITH_TARGET: u32 = 0x0035;
pub const SET_SINGLE_CHARACTER_OPTION: u32 = 0x0005;
pub const FELLOWSHIP_CREATE: u32 = 0x00A2;
pub const FELLOWSHIP_QUIT: u32 = 0x00A3;
pub const FELLOWSHIP_DISMISS: u32 = 0x00A4;
pub const FELLOWSHIP_RECRUIT: u32 = 0x00A5;
pub const FELLOWSHIP_UPDATE_REQUEST: u32 = 0x00A6;
pub const CONFIRMATION_RESPONSE: u32 = 0x0275;
pub const REMOVE_FRIEND: u32 = 0x0017;
pub const ADD_FRIEND: u32 = 0x0018;
pub const REMOVE_ALL_FRIENDS: u32 = 0x0025;
pub const TITLE_SET: u32 = 0x002C;
/// (flag, guid, name, ChatMessageType), (flag, name), (flag, ChatMessageType).
pub const MODIFY_CHARACTER_SQUELCH: u32 = 0x0058;
pub const MODIFY_ACCOUNT_SQUELCH: u32 = 0x0059;
pub const MODIFY_GLOBAL_SQUELCH: u32 = 0x005B;
pub const SWEAR_ALLEGIANCE: u32 = 0x001D;
pub const BREAK_ALLEGIANCE: u32 = 0x001E;
pub const ALLEGIANCE_UPDATE_REQUEST: u32 = 0x001F;
pub const QUERY_ALLEGIANCE_NAME: u32 = 0x0030;
pub const CLEAR_ALLEGIANCE_NAME: u32 = 0x0031;
pub const SET_ALLEGIANCE_NAME: u32 = 0x0033;
pub const ALLEGIANCE_INFO_REQUEST: u32 = 0x027B;
/// Group chat: channel id (see `channel`), text.
pub const CHAT_CHANNEL: u32 = 0x0147;
pub const SET_MOTD: u32 = 0x0254;
pub const QUERY_MOTD: u32 = 0x0255;
pub const CLEAR_MOTD: u32 = 0x0256;
/// Salvage with an Ust: tool guid, count, item guids (ACE names it
/// after the retail client's "create tinkering tool" verb).
pub const CREATE_TINKERING_TOOL: u32 = 0x027D;
/// Books: ask for a carried book's data (guid) and for one page
/// (guid, page index).
pub const BOOK_DATA: u32 = 0x00AA;
pub const BOOK_PAGE_DATA: u32 = 0x00AE;
/// Stacks: merge (from, to, amount), split into a container (stack,
/// container, placement, amount), onto the ground (stack, amount)
/// or into a wield slot (stack, EquipMask, amount).
pub const STACKABLE_MERGE: u32 = 0x0054;
pub const STACKABLE_SPLIT_TO_CONTAINER: u32 = 0x0055;
pub const STACKABLE_SPLIT_TO_3D: u32 = 0x0056;
pub const STACKABLE_SPLIT_TO_WIELD: u32 = 0x019B;
/// Housing (see docs/game/mechanics.md, section 9).
pub const BUY_HOUSE: u32 = 0x021C;
pub const HOUSE_QUERY: u32 = 0x021E;
pub const ABANDON_HOUSE: u32 = 0x021F;
pub const RENT_HOUSE: u32 = 0x0221;
pub const ADD_PERMANENT_GUEST: u32 = 0x0245;
pub const REMOVE_PERMANENT_GUEST: u32 = 0x0246;
pub const SET_OPEN_HOUSE_STATUS: u32 = 0x0247;
pub const CHANGE_STORAGE_PERMISSION: u32 = 0x0249;
pub const BOOT_SPECIFIC_HOUSE_GUEST: u32 = 0x024A;
pub const REMOVE_ALL_STORAGE_PERMISSION: u32 = 0x024C;
pub const REQUEST_FULL_GUEST_LIST: u32 = 0x024D;
pub const QUERY_LORD: u32 = 0x0258;
pub const ADD_ALL_STORAGE_PERMISSION: u32 = 0x025C;
pub const REMOVE_ALL_PERMANENT_GUESTS: u32 = 0x025E;
pub const BOOT_EVERYONE: u32 = 0x025F;
pub const SET_HOOKS_VISIBILITY: u32 = 0x0266;
pub const MODIFY_ALLEGIANCE_GUEST_PERMISSION: u32 = 0x0267;
pub const MODIFY_ALLEGIANCE_STORAGE_PERMISSION: u32 = 0x0268;
pub const LIST_AVAILABLE_HOUSES: u32 = 0x0270;
pub const OPEN_TRADE_NEGOTIATIONS: u32 = 0x01F6;
pub const CLOSE_TRADE_NEGOTIATIONS: u32 = 0x01F7;
pub const ADD_TO_TRADE: u32 = 0x01F8;
pub const ACCEPT_TRADE: u32 = 0x01FA;
pub const DECLINE_TRADE: u32 = 0x01FB;
pub const RESET_TRADE: u32 = 0x0204;
pub const RAISE_VITAL: u32 = 0x0044;
pub const RAISE_ATTRIBUTE: u32 = 0x0045;
pub const RAISE_SKILL: u32 = 0x0046;
pub const TRAIN_SKILL: u32 = 0x0047;
pub const SET_AFK_MODE: u32 = 0x000F;
pub const SET_AFK_MESSAGE: u32 = 0x0010;
pub const TELL: u32 = 0x005D;
pub const TELE_TO_LIFESTONE: u32 = 0x0063;
pub const EMOTE: u32 = 0x01DF;
pub const SOUL_EMOTE: u32 = 0x01E1;
pub const TELE_TO_HOUSE: u32 = 0x0262;
pub const TELE_TO_MANSION: u32 = 0x0278;
pub const TELE_TO_MARKETPLACE: u32 = 0x028D;
pub const ENTER_PK_LITE: u32 = 0x028F;
pub const RECALL_ALLEGIANCE_HOMETOWN: u32 = 0x02AB;
pub const DIE: u32 = 0x0279;
pub const TARGETED_MELEE_ATTACK: u32 = 0x0008;
pub const TARGETED_MISSILE_ATTACK: u32 = 0x000A;
pub const PUT_ITEM_IN_CONTAINER: u32 = 0x0019;
pub const GET_AND_WIELD_ITEM: u32 = 0x001A;
pub const DROP_ITEM: u32 = 0x001B;
pub const USE: u32 = 0x0036;
pub const CAST_UNTARGETED_SPELL: u32 = 0x0048;
pub const CAST_TARGETED_SPELL: u32 = 0x004A;
pub const CHANGE_COMBAT_MODE: u32 = 0x0053;
pub const BUY: u32 = 0x005F;
pub const SELL: u32 = 0x0060;
pub const NO_LONGER_VIEWING_CONTENTS: u32 = 0x0195;
pub const IDENTIFY_OBJECT: u32 = 0x00C8;
pub const GIVE_OBJECT_REQUEST: u32 = 0x00CD;
pub const REMOVE_SPELL: u32 = 0x01A8;
pub const ADD_SPELL_FAVORITE: u32 = 0x01E3;
pub const REMOVE_SPELL_FAVORITE: u32 = 0x01E4;
pub const SET_DESIRED_COMPONENT_LEVEL: u32 = 0x0224;
pub const SPELLBOOK_FILTER: u32 = 0x0286;
/// Sent after entering the world and after each teleport; the server
/// ignores position reports until it arrives.
pub const LOGIN_COMPLETE: u32 = 0x00A1;
/// `u32 guid`: select a creature; the server answers UpdateHealth
/// and keeps sending it every heartbeat (5 s) while the creature is
/// selected. 0 clears the selection.
pub const QUERY_HEALTH: u32 = 0x01BF;
/// No body: stop the attack in progress (AttackDone follows).
pub const CANCEL_ATTACK: u32 = 0x01B7;
/// No body: the server answers PingResponse (0x01EA).
pub const PING_REQUEST: u32 = 0x01E9;
/// `u32 item`: the server answers QueryItemManaResponse (0x0264).
pub const QUERY_ITEM_MANA: u32 = 0x0263;
pub const QUERY_AGE: u32 = 0x01C2;
pub const QUERY_BIRTH: u32 = 0x01C4;
pub const SET_INSCRIPTION: u32 = 0x00BF;
pub const SET_CHARACTER_OPTIONS: u32 = 0x01A1;
pub const ADD_SHORT_CUT: u32 = 0x019C;
pub const REMOVE_SHORT_CUT: u32 = 0x019D;
pub const LIST_CHANNELS: u32 = 0x0148;
pub const INDEX_CHANNELS: u32 = 0x0149;
pub const ADD_CHANNEL: u32 = 0x0145;
pub const REMOVE_CHANNEL: u32 = 0x0146;
/// `u32 target guid` or a heading: turn without walking; ACE turns
/// us itself for casts and attacks, so the client never needs it.
pub const TURN_TO: u32 = 0xF649;
pub const DO_MOVEMENT_COMMAND: u32 = 0xF61E;
pub const STOP_MOVEMENT_COMMAND: u32 = 0xF661;
pub const JUMP_NON_AUTONOMOUS: u32 = 0xF7C9;
pub const AUTONOMY_LEVEL: u32 = 0xF752;
pub const JUMP: u32 = 0xF61B;
pub const MOVE_TO_STATE: u32 = 0xF61C;
pub const AUTONOMOUS_POSITION: u32 = 0xF753;
