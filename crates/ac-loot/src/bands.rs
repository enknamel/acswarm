//! Which workmanships go into one salvage bag together: a salvage rule's `combine`, "1-7, 8, 9, 10".
//! ACE averages all of one material salvaged in one call into one bag (Player_Crafting.cs:251, 279),
//! so a 10 salvaged beside a 6 is wasted; bands keep them in calls of their own.

/// Workmanships from the first to the second, both included, salvaged together.
pub type Band = (u8, u8);

/// Every workmanship together: what a rule naming no bands combines.
pub const ALL: Band = (1, 10);

/// The bands `text` names, "1-7, 8, 9, 10" or "9 10"; with none readable, every workmanship together.
pub fn parse(text: &str) -> Vec<Band> {
    // "1 - 7" is one band, not 1 and 7.
    let mut text = text.to_string();
    while text.contains(" -") || text.contains("- ") {
        text = text.replace(" -", "-").replace("- ", "-");
    }
    let named: Vec<Band> = text
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let (lo, hi) = part.split_once('-').unwrap_or((part, part));
            let read = |s: &str| s.trim().parse::<u8>().ok().map(|w| w.clamp(1, 10));
            let (lo, hi) = (read(lo)?, read(hi)?);
            Some((lo.min(hi), lo.max(hi)))
        })
        .collect();
    if named.is_empty() {
        vec![ALL]
    } else {
        named
    }
}

/// The band `workmanship` falls in, rounded down since a bag's is an average and may not top up a
/// band above it: the first named that holds it, else that workmanship on its own.
pub fn band_of(bands: &[Band], workmanship: f32) -> Band {
    let w = workmanship.floor().clamp(1.0, 10.0) as u8;
    bands
        .iter()
        .copied()
        .find(|(lo, hi)| (*lo..=*hi).contains(&w))
        .unwrap_or((w, w))
}

/// A band in words: "1-7", or "9" on its own.
pub fn tell(band: Band) -> String {
    if band.0 == band.1 {
        band.0.to_string()
    } else {
        format!("{}-{}", band.0, band.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_read_as_the_player_writes_them() {
        assert_eq!(
            parse("1-7, 8, 9, 10"),
            vec![(1, 7), (8, 8), (9, 9), (10, 10)]
        );
        assert_eq!(parse("9 10"), vec![(9, 9), (10, 10)]);
        assert_eq!(parse("1 - 7, 8"), vec![(1, 7), (8, 8)], "spaced");
        assert_eq!(
            parse("7-1; 12"),
            vec![(1, 7), (10, 10)],
            "turned round and clamped"
        );
    }

    #[test]
    fn no_bands_is_every_workmanship_together() {
        assert_eq!(parse(""), vec![ALL]);
        assert_eq!(parse("  , nonsense"), vec![ALL]);
    }

    #[test]
    fn a_workmanship_goes_in_its_band_or_on_its_own() {
        let bands = parse("1-7, 9");
        assert_eq!(band_of(&bands, 6.0), (1, 7));
        assert_eq!(band_of(&bands, 7.6), (1, 7), "a bag's average rounds down");
        assert_eq!(band_of(&bands, 8.0), (8, 8), "named nowhere, alone");
        assert_eq!(band_of(&bands, 10.0), (10, 10));
        assert_eq!(band_of(&parse(""), 10.0), ALL);
    }
}
