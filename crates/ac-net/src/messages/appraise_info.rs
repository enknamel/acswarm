use crate::wire::{Reader, Truncated};

/// A weapon's appraisal block (ACE `WeaponProfile`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WeaponProfile {
    /// DamageType bits: 1 slash, 2 pierce, 4 bludgeon, 8 cold, 0x10 fire,
    /// 0x20 acid, 0x40 electric, 0x400 nether.
    pub damage_type: u32,
    /// Attack speed, lower is faster (0..=100).
    pub speed: u32,
    pub skill: u32,
    pub damage: u32,
    /// 0..=1: the low end of the damage roll is `damage * (1 - variance)`.
    pub variance: f64,
    pub damage_mod: f64,
    pub length: f64,
    pub max_velocity: f64,
    /// Attack skill multiplier (1.05 = +5%).
    pub offense: f64,
    pub max_velocity_estimated: u32,
}

/// An armor piece's protections (ACE `ArmorProfile`), as multipliers of
/// the armor level per damage type.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ArmorProfile {
    pub slash: f32,
    pub pierce: f32,
    pub bludgeon: f32,
    pub cold: f32,
    pub fire: f32,
    pub acid: f32,
    pub nether: f32,
    pub electric: f32,
}

/// A creature's appraisal block (ACE `CreatureProfile`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CreatureProfile {
    pub flags: u32,
    pub health: u32,
    pub health_max: u32,
    /// Strength, Endurance, Quickness, Coordination, Focus, Self; only
    /// with flag 8.
    pub attributes: Option<[u32; 6]>,
    pub stamina: u32,
    pub mana: u32,
    pub stamina_max: u32,
    pub mana_max: u32,
    /// (highlight, colour) bitmasks of buffed/debuffed attributes.
    pub attribute_marks: Option<(u16, u16)>,
}

/// IdentifyObjectResponse (game event 0x00C9): the property tables of an
/// appraised object and, by flags, its spell list, armor, creature and
/// weapon profiles, hook profile, enchantment marks and per-location
/// armor levels (ACE `AppraiseInfo`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Appraisal {
    pub guid: u32,
    pub success: bool,
    pub ints: Vec<(u32, i32)>,
    pub int64s: Vec<(u32, i64)>,
    pub bools: Vec<(u32, bool)>,
    pub floats: Vec<(u32, f64)>,
    pub strings: Vec<(u32, String)>,
    pub dids: Vec<(u32, u32)>,
    /// Spell ids the item casts or carries.
    pub spells: Vec<u32>,
    pub armor: Option<ArmorProfile>,
    pub creature: Option<CreatureProfile>,
    pub weapon: Option<WeaponProfile>,
    /// (flags, valid locations, ammo type) of a hook.
    pub hook: Option<(u32, u32, u32)>,
    /// (highlight, colour) marks for armor, weapon and resist values
    /// changed by enchantments.
    pub armor_marks: Option<(u16, u16)>,
    pub weapon_marks: Option<(u16, u16)>,
    pub resist_marks: Option<(u16, u16)>,
    /// A creature's armor by location: head, chest, abdomen, upper arm,
    /// lower arm, hand, upper leg, lower leg, foot.
    pub armor_levels: Option<[u32; 9]>,
}

impl Appraisal {
    pub const FLAG_INT: u32 = 0x0001;
    pub const FLAG_BOOL: u32 = 0x0002;
    pub const FLAG_FLOAT: u32 = 0x0004;
    pub const FLAG_STRING: u32 = 0x0008;
    pub const FLAG_DID: u32 = 0x1000;
    pub const FLAG_INT64: u32 = 0x2000;
    pub const FLAG_SPELL_BOOK: u32 = 0x0010;
    pub const FLAG_WEAPON_PROFILE: u32 = 0x0020;
    pub const FLAG_HOOK_PROFILE: u32 = 0x0040;
    pub const FLAG_ARMOR_PROFILE: u32 = 0x0080;
    pub const FLAG_CREATURE_PROFILE: u32 = 0x0100;
    pub const FLAG_ARMOR_MARKS: u32 = 0x0200;
    pub const FLAG_RESIST_MARKS: u32 = 0x0400;
    pub const FLAG_WEAPON_MARKS: u32 = 0x0800;
    pub const FLAG_ARMOR_LEVELS: u32 = 0x4000;
    pub const STRING_USE: u32 = 14;
    pub const STRING_SHORT_DESC: u32 = 15;
    pub const STRING_LONG_DESC: u32 = 16;

    pub fn parse(body: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(body);
        let mut a = Appraisal {
            guid: r.u32()?,
            ..Default::default()
        };
        let flags = r.u32()?;
        a.success = r.u32()? != 0;
        fn header(r: &mut Reader) -> Result<u16, Truncated> {
            let n = r.u16()?;
            r.u16()?;
            Ok(n)
        }
        if flags & Self::FLAG_INT != 0 {
            for _ in 0..header(&mut r)? {
                a.ints.push((r.u32()?, r.i32()?));
            }
        }
        if flags & Self::FLAG_INT64 != 0 {
            for _ in 0..header(&mut r)? {
                a.int64s.push((r.u32()?, r.u64()? as i64));
            }
        }
        if flags & Self::FLAG_BOOL != 0 {
            for _ in 0..header(&mut r)? {
                a.bools.push((r.u32()?, r.u32()? != 0));
            }
        }
        if flags & Self::FLAG_FLOAT != 0 {
            for _ in 0..header(&mut r)? {
                a.floats.push((r.u32()?, r.f64()?));
            }
        }
        if flags & Self::FLAG_STRING != 0 {
            for _ in 0..header(&mut r)? {
                a.strings.push((r.u32()?, r.string16()?));
            }
        }
        if flags & Self::FLAG_DID != 0 {
            for _ in 0..header(&mut r)? {
                a.dids.push((r.u32()?, r.u32()?));
            }
        }
        if flags & Self::FLAG_SPELL_BOOK != 0 {
            let n = r.u32()? as usize;
            for _ in 0..n.min(256) {
                a.spells.push(r.u32()?);
            }
        }
        if flags & Self::FLAG_ARMOR_PROFILE != 0 {
            a.armor = Some(ArmorProfile {
                slash: r.f32()?,
                pierce: r.f32()?,
                bludgeon: r.f32()?,
                cold: r.f32()?,
                fire: r.f32()?,
                acid: r.f32()?,
                nether: r.f32()?,
                electric: r.f32()?,
            });
        }
        if flags & Self::FLAG_CREATURE_PROFILE != 0 {
            let cflags = r.u32()?;
            let mut c = CreatureProfile {
                flags: cflags,
                health: r.u32()?,
                health_max: r.u32()?,
                ..Default::default()
            };
            if cflags & 0x8 != 0 {
                let mut attrs = [0u32; 6];
                for a in attrs.iter_mut() {
                    *a = r.u32()?;
                }
                c.attributes = Some(attrs);
                c.stamina = r.u32()?;
                c.mana = r.u32()?;
                c.stamina_max = r.u32()?;
                c.mana_max = r.u32()?;
            }
            if cflags & 0x1 != 0 {
                c.attribute_marks = Some((r.u16()?, r.u16()?));
            }
            a.creature = Some(c);
        }
        if flags & Self::FLAG_WEAPON_PROFILE != 0 {
            a.weapon = Some(WeaponProfile {
                damage_type: r.u32()?,
                speed: r.u32()?,
                skill: r.u32()?,
                damage: r.u32()?,
                variance: r.f64()?,
                damage_mod: r.f64()?,
                length: r.f64()?,
                max_velocity: r.f64()?,
                offense: r.f64()?,
                max_velocity_estimated: r.u32()?,
            });
        }
        if flags & Self::FLAG_HOOK_PROFILE != 0 {
            a.hook = Some((r.u32()?, r.u32()?, r.u32()?));
        }
        if flags & Self::FLAG_ARMOR_MARKS != 0 {
            a.armor_marks = Some((r.u16()?, r.u16()?));
        }
        if flags & Self::FLAG_WEAPON_MARKS != 0 {
            a.weapon_marks = Some((r.u16()?, r.u16()?));
        }
        if flags & Self::FLAG_RESIST_MARKS != 0 {
            a.resist_marks = Some((r.u16()?, r.u16()?));
        }
        if flags & Self::FLAG_ARMOR_LEVELS != 0 {
            let mut levels = [0u32; 9];
            for l in levels.iter_mut() {
                *l = r.u32()?;
            }
            a.armor_levels = Some(levels);
        }
        Ok(a)
    }

    pub fn int(&self, key: u32) -> Option<i32> {
        self.ints.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    pub fn float(&self, key: u32) -> Option<f64> {
        self.floats.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    pub fn bool(&self, key: u32) -> Option<bool> {
        self.bools.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
    }

    pub fn string(&self, key: u32) -> Option<&str> {
        self.strings
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
    }
}

#[cfg(test)]
mod tests;
