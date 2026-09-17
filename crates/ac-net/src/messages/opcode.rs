pub const CHARACTER_CREATE_RESPONSE: u32 = 0xF643;
pub const CHARACTER_CREATE: u32 = 0xF656;
pub const CHARACTER_ENTER_WORLD: u32 = 0xF657;
pub const CHARACTER_LIST: u32 = 0xF658;
pub const CHARACTER_ERROR: u32 = 0xF659;
pub const CHARACTER_DELETE: u32 = 0xF655;
pub const CHARACTER_RESTORE: u32 = 0xF7D9;
pub const OBJECT_CREATE: u32 = 0xF745;
pub const PLAYER_CREATE: u32 = 0xF746;
pub const OBJECT_DELETE: u32 = 0xF747;
/// Turbine chat (the General, Trade, LFG, Roleplay, society and
/// allegiance rooms), both ways; see `turbine`.
pub const TURBINE_CHAT: u32 = 0xF7DE;
/// A carried item left our inventory (spent, given, dropped to the
/// corpse): the guid; the object itself is not deleted.
pub const INVENTORY_REMOVE_OBJECT: u32 = 0x0024;
pub const UPDATE_POSITION: u32 = 0xF748;
pub const SET_STATE: u32 = 0xF74B;
pub const PLAYER_TELEPORT: u32 = 0xF751;
pub const MOVEMENT_EVENT: u32 = 0xF74C;
pub const GAME_EVENT: u32 = 0xF7B0;
pub const GAME_ACTION: u32 = 0xF7B1;
pub const CHARACTER_ENTER_WORLD_REQUEST: u32 = 0xF7C8;
pub const ACCOUNT_BOOT: u32 = 0xF7DC;
pub const UPDATE_OBJECT: u32 = 0xF7DB;
pub const CHARACTER_ENTER_WORLD_SERVER_READY: u32 = 0xF7DF;
pub const EMOTE_TEXT: u32 = 0x01E0;
/// HearSoulEmote: sender guid, name, the emote's line ("waves").
pub const SOUL_EMOTE: u32 = 0x01E2;
pub const SET_STACK_SIZE: u32 = 0x0197;
pub const PUBLIC_UPDATE_INSTANCE_ID: u32 = 0x02DA;
pub const PICKUP_EVENT: u32 = 0xF74A;
pub const SOUND: u32 = 0xF750;
pub const OBJ_DESC_EVENT: u32 = 0xF625;
pub const PUBLIC_UPDATE_PROPERTY_INT: u32 = 0x02CE;
pub const PRIVATE_UPDATE_PROPERTY_INT: u32 = 0x02CD;
pub const PRIVATE_UPDATE_PROPERTY_INT64: u32 = 0x02CF;
pub const PRIVATE_UPDATE_PROPERTY_STRING: u32 = 0x02D5;
/// One of the character's saved positions (by `PositionType`):
/// `u8 sequence, u32 type, position`. The server sends only where
/// the last corpse fell; see `ac_world::recalls`.
pub const PRIVATE_UPDATE_POSITION: u32 = 0x02DB;
pub const PRIVATE_UPDATE_SKILL: u32 = 0x02DD;
pub const PRIVATE_UPDATE_SKILL_LEVEL: u32 = 0x02DF;
pub const PRIVATE_UPDATE_SKILL_AC: u32 = 0x02E1;
pub const PRIVATE_UPDATE_ATTRIBUTE: u32 = 0x02E3;
pub const PRIVATE_UPDATE_VITAL: u32 = 0x02E7;
pub const PRIVATE_UPDATE_ATTRIBUTE_2ND_LEVEL: u32 = 0x02E9;
pub const HEAR_SPEECH: u32 = 0x02BB;
pub const HEAR_RANGED_SPEECH: u32 = 0x02BC;
pub const SERVER_MESSAGE: u32 = 0xF7E0;
pub const SERVER_NAME: u32 = 0xF7E1;
pub const DDD_INTERROGATION: u32 = 0xF7E5;
pub const DDD_INTERROGATION_RESPONSE: u32 = 0xF7E6;
pub const DDD_BEGIN_DDD: u32 = 0xF7E7;
pub const DDD_END_DDD: u32 = 0xF7EA;
/// A player in view died: `string16 message, u32 victim, u32 killer`
/// (the killer is 0 when nobody dealt the blow).
pub const PLAYER_KILLED: u32 = 0x019E;
/// `u8 sequence, u32 guid, u32 property, i64 value`.
pub const PUBLIC_UPDATE_PROPERTY_INT64: u32 = 0x02D0;
/// `u8 sequence, u32 property, u32 value (0/1)`: our own bool.
pub const PRIVATE_UPDATE_PROPERTY_BOOL: u32 = 0x02D1;
/// `u8 sequence, u32 guid, u32 property, u32 value (0/1)`.
pub const PUBLIC_UPDATE_PROPERTY_BOOL: u32 = 0x02D2;
/// `u8 sequence, u32 property, f64 value`: our own float.
pub const PRIVATE_UPDATE_PROPERTY_FLOAT: u32 = 0x02D3;
/// `u8 sequence, u32 guid, u32 property, f64 value`.
pub const PUBLIC_UPDATE_PROPERTY_FLOAT: u32 = 0x02D4;
/// `u8 sequence, u32 property, u32 guid, align, string16 value`
/// (the property comes before the guid, unlike the other public
/// updates).
pub const PUBLIC_UPDATE_PROPERTY_STRING: u32 = 0x02D6;
/// `u8 sequence, u32 property, u32 value`: one of our own data ids
/// (a motion table swap when a mount or an emote changes it).
pub const PRIVATE_UPDATE_PROPERTY_DATA_ID: u32 = 0x02D7;
/// `u8 sequence, u32 guid, u32 property, u32 value`.
pub const PUBLIC_UPDATE_PROPERTY_DATA_ID: u32 = 0x02D8;
/// `u8 sequence, u32 property, u32 value`: one of our own instance
/// ids (CurrentAttacker: who hit us last).
pub const PRIVATE_UPDATE_INSTANCE_ID: u32 = 0x02D9;
/// Never sent by ACE; the layouts are the public counterparts of
/// the private updates with a guid after the sequence byte.
pub const PUBLIC_UPDATE_POSITION: u32 = 0x02DC;
pub const PUBLIC_UPDATE_SKILL: u32 = 0x02DE;
pub const PUBLIC_UPDATE_SKILL_LEVEL: u32 = 0x02E0;
pub const PUBLIC_UPDATE_ATTRIBUTE: u32 = 0x02E4;
pub const PUBLIC_UPDATE_VITAL: u32 = 0x02E8;
/// `u32 EnvironChangeType`: an admin changed the weather or the
/// sound environment for us.
pub const ADMIN_ENVIRONS: u32 = 0xEA60;
pub const POSITION_AND_MOVEMENT: u32 = 0xF619;
/// The server logged our character out (no body). Also the C2S
/// message asking to.
pub const CHARACTER_LOG_OFF: u32 = 0xF653;
pub const FORCE_OBJECT_DESC_SEND: u32 = 0xF6EA;
/// `u32 parent, u32 child, u32 parent location, u32 placement, u16
/// instance seq, u16 position seq`: `child` is now carried by
/// `parent` (a wielded item, an arrow nocked in a bow hand).
pub const PARENT_EVENT: u32 = 0xF749;
/// `u32 guid, f32x3 velocity, f32x3 omega, u16 instance seq, u16
/// vector seq`: an object's velocity changed without a position.
pub const VECTOR_UPDATE: u32 = 0xF74E;
pub const PLAY_SCRIPT_ID: u32 = 0xF754;
/// `u32 guid, u32 PlayScript, f32 speed`: a particle effect on an
/// object (spell fizzle, portal entry, level-up flash).
pub const PLAY_EFFECT: u32 = 0xF755;
/// `u32 seconds until the ban ends, [string16 reason]`.
pub const ACCOUNT_BANNED: u32 = 0xF7C1;
/// No payload: the server's own build, which retail asked for from an
/// admin, arch or PSR character only.
pub const GET_SERVER_VERSION: u32 = 0xF7CC;
pub const FRIENDS_OLD: u32 = 0xF7CD;
pub const DDD_DATA_MESSAGE: u32 = 0xF7E2;
pub const DDD_REQUEST_DATA_MESSAGE: u32 = 0xF7E3;
pub const DDD_ERROR_MESSAGE: u32 = 0xF7E4;
pub const DDD_BEGIN_PULL_DDD: u32 = 0xF7E8;
pub const DDD_ITERATION_DATA: u32 = 0xF7E9;

/// ACE's name for a server-to-client message opcode, for logs and
/// the coverage audit; `None` for one ACE does not define.
pub fn name(op: u32) -> Option<&'static str> {
    Some(match op {
        INVENTORY_REMOVE_OBJECT => "InventoryRemoveObject",
        SET_STACK_SIZE => "SetStackSize",
        PLAYER_KILLED => "PlayerKilled",
        EMOTE_TEXT => "EmoteText",
        SOUL_EMOTE => "SoulEmote",
        HEAR_SPEECH => "HearSpeech",
        HEAR_RANGED_SPEECH => "HearRangedSpeech",
        PRIVATE_UPDATE_PROPERTY_INT => "PrivateUpdatePropertyInt",
        PUBLIC_UPDATE_PROPERTY_INT => "PublicUpdatePropertyInt",
        PRIVATE_UPDATE_PROPERTY_INT64 => "PrivateUpdatePropertyInt64",
        PUBLIC_UPDATE_PROPERTY_INT64 => "PublicUpdatePropertyInt64",
        PRIVATE_UPDATE_PROPERTY_BOOL => "PrivateUpdatePropertyBool",
        PUBLIC_UPDATE_PROPERTY_BOOL => "PublicUpdatePropertyBool",
        PRIVATE_UPDATE_PROPERTY_FLOAT => "PrivateUpdatePropertyFloat",
        PUBLIC_UPDATE_PROPERTY_FLOAT => "PublicUpdatePropertyFloat",
        PRIVATE_UPDATE_PROPERTY_STRING => "PrivateUpdatePropertyString",
        PUBLIC_UPDATE_PROPERTY_STRING => "PublicUpdatePropertyString",
        PRIVATE_UPDATE_PROPERTY_DATA_ID => "PrivateUpdatePropertyDataID",
        PUBLIC_UPDATE_PROPERTY_DATA_ID => "PublicUpdatePropertyDataID",
        PRIVATE_UPDATE_INSTANCE_ID => "PrivateUpdatePropertyInstanceID",
        PUBLIC_UPDATE_INSTANCE_ID => "PublicUpdateInstanceId",
        PRIVATE_UPDATE_POSITION => "PrivateUpdatePosition",
        PUBLIC_UPDATE_POSITION => "PublicUpdatePosition",
        PRIVATE_UPDATE_SKILL => "PrivateUpdateSkill",
        PUBLIC_UPDATE_SKILL => "PublicUpdateSkill",
        PRIVATE_UPDATE_SKILL_LEVEL => "PrivateUpdateSkillLevel",
        PUBLIC_UPDATE_SKILL_LEVEL => "PublicUpdateSkillLevel",
        PRIVATE_UPDATE_SKILL_AC => "PrivateUpdateSkillAC",
        PRIVATE_UPDATE_ATTRIBUTE => "PrivateUpdateAttribute",
        PUBLIC_UPDATE_ATTRIBUTE => "PublicUpdateAttribute",
        PRIVATE_UPDATE_VITAL => "PrivateUpdateVital",
        PUBLIC_UPDATE_VITAL => "PublicUpdateVital",
        PRIVATE_UPDATE_ATTRIBUTE_2ND_LEVEL => "PrivateUpdateAttribute2ndLevel",
        ADMIN_ENVIRONS => "AdminEnvirons",
        POSITION_AND_MOVEMENT => "PositionAndMovement",
        OBJ_DESC_EVENT => "ObjDescEvent",
        CHARACTER_CREATE_RESPONSE => "CharacterCreateResponse",
        CHARACTER_LOG_OFF => "CharacterLogOff",
        CHARACTER_DELETE => "CharacterDelete",
        CHARACTER_CREATE => "CharacterCreate",
        CHARACTER_ENTER_WORLD => "CharacterEnterWorld",
        CHARACTER_LIST => "CharacterList",
        CHARACTER_ERROR => "CharacterError",
        FORCE_OBJECT_DESC_SEND => "ForceObjectDescSend",
        OBJECT_CREATE => "ObjectCreate",
        PLAYER_CREATE => "PlayerCreate",
        OBJECT_DELETE => "ObjectDelete",
        UPDATE_POSITION => "UpdatePosition",
        PARENT_EVENT => "ParentEvent",
        PICKUP_EVENT => "PickupEvent",
        SET_STATE => "SetState",
        MOVEMENT_EVENT => "MovementEvent",
        VECTOR_UPDATE => "VectorUpdate",
        SOUND => "Sound",
        PLAYER_TELEPORT => "PlayerTeleport",
        0xF753 => "AutonomousPosition",
        PLAY_SCRIPT_ID => "PlayScriptId",
        PLAY_EFFECT => "PlayEffect",
        GAME_EVENT => "GameEvent",
        GAME_ACTION => "GameAction",
        ACCOUNT_BANNED => "AccountBanned",
        CHARACTER_ENTER_WORLD_REQUEST => "CharacterEnterWorldRequest",
        GET_SERVER_VERSION => "GetServerVersion",
        FRIENDS_OLD => "FriendsOld",
        CHARACTER_RESTORE => "CharacterRestore",
        ACCOUNT_BOOT => "AccountBoot",
        UPDATE_OBJECT => "UpdateObject",
        TURBINE_CHAT => "TurbineChat",
        CHARACTER_ENTER_WORLD_SERVER_READY => "CharacterEnterWorldServerReady",
        SERVER_MESSAGE => "ServerMessage",
        SERVER_NAME => "ServerName",
        DDD_DATA_MESSAGE => "DDD_DataMessage",
        DDD_REQUEST_DATA_MESSAGE => "DDD_RequestDataMessage",
        DDD_ERROR_MESSAGE => "DDD_ErrorMessage",
        DDD_INTERROGATION => "DDD_Interrogation",
        DDD_INTERROGATION_RESPONSE => "DDD_InterrogationResponse",
        DDD_BEGIN_DDD => "DDD_BeginDDD",
        DDD_BEGIN_PULL_DDD => "DDD_BeginPullDDD",
        DDD_ITERATION_DATA => "DDD_IterationData",
        DDD_END_DDD => "DDD_EndDDD",
        _ => return None,
    })
}
