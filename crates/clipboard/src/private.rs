//! The private clipboard format for lossless paste between Open365 windows: a short binary
//! header and a JSON body. Any program can write to the clipboard, so decoding checks the
//! header, the size and every value before the result is used.

use serde::{Deserialize, Serialize};

use crate::limits::MAX_PRIVATE_BYTES;
use crate::model::{ParseError, RichText};
use crate::sheet::SheetClip;

/// The registered clipboard format name.
pub const FORMAT_NAME: &str = "Open365 Clip v1";

const MAGIC: &[u8; 8] = b"O365CLIP";
const VERSION: u8 = 1;
const HEADER: usize = 14;

/// What an Open365 window copied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PrivateClip {
    Text(RichText),
    Sheet(SheetClip),
}

impl PrivateClip {
    fn kind(&self) -> u8 {
        match self {
            Self::Text(_) => 1,
            Self::Sheet(_) => 2,
        }
    }

    /// The bytes for the clipboard, or `None` when they would pass the size limit.
    pub fn encode(&self) -> Option<Vec<u8>> {
        let body = serde_json::to_vec(self).ok()?;
        if body.len() + HEADER > MAX_PRIVATE_BYTES {
            return None;
        }
        let mut out = Vec::with_capacity(HEADER + body.len());
        out.extend_from_slice(MAGIC);
        out.push(VERSION);
        out.push(self.kind());
        out.extend_from_slice(&u32::try_from(body.len()).ok()?.to_le_bytes());
        out.extend_from_slice(&body);
        Some(out)
    }

    pub fn decode(data: &[u8]) -> Result<Self, ParseError> {
        if data.len() > MAX_PRIVATE_BYTES {
            return Err(ParseError::TooLarge);
        }
        if data.len() < HEADER || &data[..8] != MAGIC {
            return Err(ParseError::Malformed("not an Open365 clip"));
        }
        if data[8] != VERSION {
            return Err(ParseError::Malformed("unknown version"));
        }
        let len = u32::from_le_bytes([data[10], data[11], data[12], data[13]]) as usize;
        let body = data
            .get(HEADER..HEADER + len)
            .ok_or(ParseError::Malformed("truncated"))?;
        let mut clip: Self =
            serde_json::from_slice(body).map_err(|_| ParseError::Malformed("bad body"))?;
        if clip.kind() != data[9] {
            return Err(ParseError::Malformed("kind mismatch"));
        }
        let ok = match &mut clip {
            Self::Text(t) => t.sanitize(),
            Self::Sheet(s) => s.sanitize(),
        };
        if ok {
            Ok(clip)
        } else {
            Err(ParseError::TooManyItems)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Para, Run};

    #[test]
    fn round_trip_and_rejection() {
        let rt = RichText {
            paras: vec![Para {
                runs: vec![Run {
                    text: "x".into(),
                    ..Run::default()
                }],
                ..Para::default()
            }],
        };
        let bytes = PrivateClip::Text(rt.clone()).encode().unwrap();
        assert_eq!(PrivateClip::decode(&bytes), Ok(PrivateClip::Text(rt)));
        assert!(PrivateClip::decode(b"junk").is_err());
        let mut bad = bytes.clone();
        bad[9] = 2;
        assert!(PrivateClip::decode(&bad).is_err());
        assert!(PrivateClip::decode(&bytes[..bytes.len() - 1]).is_err());
        let mut body = bytes.clone();
        body.truncate(HEADER);
        body.extend_from_slice(b"{}");
        assert!(PrivateClip::decode(&body).is_err());
    }

    #[test]
    fn hostile_values_are_repaired() {
        let json = br#"{"Text":{"paras":[{"runs":[{"text":"a\u0007b","fmt":{"font":"<x>","size_pt":1e9,"bold":false,"italic":false,"underline":false,"strike":false,"superscript":true,"subscript":true,"color":null,"highlight":null},"link":"javascript:x"}],"align":"Left","heading":9,"list":null}]}}"#;
        let mut data = Vec::new();
        data.extend_from_slice(MAGIC);
        data.push(VERSION);
        data.push(1);
        data.extend_from_slice(&(json.len() as u32).to_le_bytes());
        data.extend_from_slice(json);
        let PrivateClip::Text(t) = PrivateClip::decode(&data).unwrap() else {
            panic!("wrong kind");
        };
        let run = &t.paras[0].runs[0];
        assert_eq!(run.text, "ab");
        assert_eq!(run.fmt.font.as_deref(), Some("x"));
        assert_eq!(run.fmt.size_pt, Some(1638.0));
        assert!(!(run.fmt.superscript && run.fmt.subscript));
        assert_eq!(run.link, None);
        assert_eq!(t.paras[0].heading, None);
    }
}
