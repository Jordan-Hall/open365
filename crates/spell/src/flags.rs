//! Affix flag parsing (`FLAG` modes) and flag sets.

/// One flag: a character, a two-character pair (`FLAG long`) or a number (`FLAG num`).
pub(crate) type Flag = u32;

/// How flags are written in the `.aff` and `.dic` files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FlagMode {
    /// One character per flag (ASCII or UTF-8).
    #[default]
    Char,
    /// Two characters per flag (`FLAG long`).
    Long,
    /// Comma separated decimal numbers (`FLAG num`).
    Num,
}

/// Parses a flag string such as `ABC`, `aZbY` (long) or `1,22,3` (num).
pub(crate) fn parse_flags(text: &str, mode: FlagMode) -> Result<Vec<Flag>, String> {
    match mode {
        FlagMode::Char => Ok(text.chars().map(|c| c as u32).collect()),
        FlagMode::Long => {
            let chars: Vec<char> = text.chars().collect();
            if !chars.len().is_multiple_of(2) {
                return Err(format!("long flag string `{text}` has an odd length"));
            }
            Ok(chars
                .chunks_exact(2)
                .map(|pair| ((pair[0] as u32 & 0xFFFF) << 16) | (pair[1] as u32 & 0xFFFF))
                .collect())
        }
        FlagMode::Num => text
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .map(|part| {
                part.parse::<u32>()
                    .map_err(|_| format!("`{part}` is not a numeric flag"))
            })
            .collect(),
    }
}

/// Parses exactly one flag (used by directives such as `COMPOUNDFLAG`).
pub(crate) fn parse_single_flag(text: &str, mode: FlagMode) -> Result<Flag, String> {
    parse_flags(text, mode)?
        .first()
        .copied()
        .ok_or_else(|| "missing flag".to_owned())
}

/// A sorted, de-duplicated set of flags.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct FlagSet(Box<[Flag]>);

impl FlagSet {
    pub(crate) fn new(mut flags: Vec<Flag>) -> Self {
        flags.sort_unstable();
        flags.dedup();
        Self(flags.into_boxed_slice())
    }

    pub(crate) fn contains(&self, flag: Flag) -> bool {
        self.0.binary_search(&flag).is_ok()
    }

    /// True when `flag` is configured and present.
    pub(crate) fn has(&self, flag: Option<Flag>) -> bool {
        flag.is_some_and(|flag| self.contains(flag))
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = Flag> + '_ {
        self.0.iter().copied()
    }
}
