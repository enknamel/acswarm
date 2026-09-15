use crate::wire::{Reader, Truncated, Writer};

/// CombatMode values for ChangeCombatMode.
pub mod combat_mode {
    pub const NON_COMBAT: u32 = 1;
    pub const MELEE: u32 = 2;
    pub const MISSILE: u32 = 4;
    pub const MAGIC: u32 = 8;
}

/// AttackerNotification (0x01B1) / DefenderNotification (0x01B2): one
/// landed melee blow, from either side.
#[derive(Debug, Clone, PartialEq)]
pub struct AttackNotice {
    /// The other party's name.
    pub name: String,
    pub damage_type: u32,
    /// Fraction of the victim's health removed.
    pub percent: f64,
    pub damage: u32,
    pub critical: bool,
}

impl AttackNotice {
    pub fn parse_attacker(body: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(body);
        Ok(AttackNotice {
            name: r.string16()?,
            damage_type: r.u32()?,
            percent: r.f64()?,
            damage: r.u32()?,
            critical: r.u32()? != 0,
        })
    }
    pub fn parse_defender(body: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(body);
        let name = r.string16()?;
        let damage_type = r.u32()?;
        let percent = r.f64()?;
        let damage = r.u32()?;
        let _location = r.u32()?;
        let critical = r.u32()? != 0;
        Ok(AttackNotice {
            name,
            damage_type,
            percent,
            damage,
            critical,
        })
    }
}

/// CastTargetedSpell body: target guid then spell id.
pub fn cast_targeted(target: u32, spell: u32) -> Vec<u8> {
    let mut w = Writer::new();
    w.u32(target).u32(spell);
    w.finish()
}

/// PlayerKilled (0x019E): a player in view died: `(message, victim,
/// killer)`; the killer is 0 when nothing dealt the blow.
pub fn parse_player_killed(body: &[u8]) -> Result<(String, u32, u32), Truncated> {
    let mut r = Reader::new(body);
    Ok((r.string16()?, r.u32()?, r.u32()?))
}

/// UpdateHealth (0x01C0): a creature's health as a fraction.
pub fn parse_update_health(body: &[u8]) -> Result<(u32, f32), Truncated> {
    let mut r = Reader::new(body);
    Ok((r.u32()?, r.f32()?))
}

#[cfg(test)]
mod tests;
