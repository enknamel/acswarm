use crate::wire::{Reader, Truncated};

/// One material's yield in a SalvageOperationsResult.
#[derive(Debug, Clone, PartialEq)]
pub struct SalvageYield {
    pub material: u32,
    pub workmanship: f64,
    pub units: u32,
}

/// SalvageOperationsResult (game event 0x02B4): the skill used (ACE
/// `Skill`: 40 salvaging, 18/28/29/30 the tinkerings), the guids that
/// could not be salvaged, the yields, and the augmentation bonus percent.
#[derive(Debug, Clone, PartialEq)]
pub struct SalvageResult {
    pub skill: u32,
    pub skipped: Vec<u32>,
    pub yields: Vec<SalvageYield>,
    pub bonus_percent: u32,
}

impl SalvageResult {
    pub fn parse(body: &[u8]) -> Result<Self, Truncated> {
        let mut r = Reader::new(body);
        let skill = r.u32()?;
        let n = r.u32()? as usize;
        let mut skipped = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            skipped.push(r.u32()?);
        }
        let n = r.u32()? as usize;
        let mut yields = Vec::with_capacity(n.min(64));
        for _ in 0..n {
            yields.push(SalvageYield {
                material: r.u32()?,
                workmanship: r.f64()?,
                units: r.u32()?,
            });
        }
        let bonus_percent = r.u32().unwrap_or(0);
        Ok(SalvageResult {
            skill,
            skipped,
            yields,
            bonus_percent,
        })
    }
}
