//! Resolves a picture's relationship id through the part the layout says it belongs to: the
//! main part, a header or footer, or the footnotes or endnotes part. This is the same lookup
//! the PDF export uses (`open365-pdf`), so the screen and the print path find the same bytes.

use open365_docx::Document;
use open365_layout::ImageSource;
use open365_opc::{PartName, Relationships, TargetMode, rels_part_name, resolve_target};

/// The bytes of the image part that relationship `embed` names in `source`.
pub fn image_bytes<'a>(doc: &'a Document, source: &ImageSource, embed: &str) -> Option<&'a [u8]> {
    let owner = match source {
        ImageSource::Main => return doc.image_part(embed).map(|p| p.bytes),
        ImageSource::HeaderFooter(rel) => related(doc, doc.main_part(), |r| r.id == *rel)?,
        ImageSource::Footnotes => {
            related(doc, doc.main_part(), |r| r.rel_type.ends_with("/footnotes"))?
        }
        ImageSource::Endnotes => {
            related(doc, doc.main_part(), |r| r.rel_type.ends_with("/endnotes"))?
        }
    };
    let rel = relationships(doc, &owner)?.get(embed).cloned()?;
    if rel.target_mode != TargetMode::Internal {
        return None;
    }
    let name = resolve_target(Some(&owner), &rel.target).ok()?;
    Some(doc.package().part(name.as_str())?.data())
}

/// A cache key that tells apart equal relationship ids in different parts.
pub fn cache_key(source: &ImageSource, embed: &str) -> String {
    match source {
        ImageSource::Main => format!("main/{embed}"),
        ImageSource::HeaderFooter(rel) => format!("hf:{rel}/{embed}"),
        ImageSource::Footnotes => format!("fn/{embed}"),
        ImageSource::Endnotes => format!("en/{embed}"),
    }
}

fn relationships(doc: &Document, part: &PartName) -> Option<Relationships> {
    let rels = doc.package().part(rels_part_name(Some(part)).as_str())?;
    Relationships::parse(rels.data()).ok()
}

fn related(
    doc: &Document,
    from: &PartName,
    pick: impl Fn(&open365_opc::Relationship) -> bool,
) -> Option<PartName> {
    let rels = relationships(doc, from)?;
    let rel = rels
        .iter()
        .find(|r| r.target_mode == TargetMode::Internal && pick(r))?;
    resolve_target(Some(from), &rel.target).ok()
}
