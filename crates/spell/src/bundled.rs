//! English dictionaries compiled into the binary (feature `bundled-en`).
//!
//! Only SCOWL-derived en_US, en_GB, en_CA and en_AU are bundled: their licence (see
//! `dictionaries/NOTICE.md` and each `*.LICENSE.txt`) is permissive and compatible with MIT
//! distribution. Every other language is loaded at run time from files the user installed.

/// A bundled dictionary.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Bundled {
    /// BCP-47 tag, such as `en-GB`.
    pub tag: &'static str,
    /// `.aff` bytes.
    pub aff: &'static [u8],
    /// `.dic` bytes.
    pub dic: &'static [u8],
}

#[cfg(feature = "bundled-en")]
macro_rules! entry {
    ($tag:literal, $file:literal) => {
        Bundled {
            tag: $tag,
            aff: include_bytes!(concat!("../dictionaries/", $file, ".aff")),
            dic: include_bytes!(concat!("../dictionaries/", $file, ".dic")),
        }
    };
}

#[cfg(feature = "bundled-en")]
static BUNDLED: [Bundled; 4] = [
    entry!("en-US", "en_US"),
    entry!("en-GB", "en_GB"),
    entry!("en-CA", "en_CA"),
    entry!("en-AU", "en_AU"),
];

#[cfg(not(feature = "bundled-en"))]
static BUNDLED: [Bundled; 0] = [];

/// Tags of the bundled dictionaries.
#[must_use]
pub fn bundled_tags() -> Vec<&'static str> {
    BUNDLED.iter().map(|b| b.tag).collect()
}

/// The bundled dictionary closest to `tag` (lower-case, hyphen separated), and whether it is an
/// exact match. Other English regions map to the nearest spelling convention.
pub(crate) fn find(tag: &str) -> Option<(Bundled, bool)> {
    let mut parts = tag.split('-');
    if parts.next() != Some("en") {
        return None;
    }
    let region = parts.find(|p| p.len() == 2).unwrap_or("");
    let (target, exact) = match region {
        "us" => ("en-US", true),
        "gb" => ("en-GB", true),
        "ca" => ("en-CA", true),
        "au" => ("en-AU", true),
        "nz" => ("en-AU", false),
        "ie" | "za" | "in" | "sg" | "my" | "hk" | "mt" | "jm" | "zw" | "bz" | "tt" | "ke"
        | "ng" | "pk" => ("en-GB", false),
        _ => ("en-US", false),
    };
    BUNDLED
        .iter()
        .find(|b| b.tag == target)
        .map(|b| (*b, exact))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "bundled-en")]
    #[test]
    fn maps_regions_to_the_nearest_bundled_dictionary() {
        let tag = |t: &str| find(t).map(|(b, exact)| (b.tag, exact));
        assert_eq!(tag("en-gb"), Some(("en-GB", true)));
        assert_eq!(tag("en"), Some(("en-US", false)));
        assert_eq!(tag("en-nz"), Some(("en-AU", false)));
        assert_eq!(tag("en-ie"), Some(("en-GB", false)));
        assert_eq!(tag("en-latn-us"), Some(("en-US", true)));
        assert_eq!(tag("de-de"), None);
    }
}
