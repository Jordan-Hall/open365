use std::collections::HashMap;
use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::main_part::{CONTENT_TYPES_PART, without_override};
use crate::{OpcError, PartName};

/// Resource limits applied while reading an untrusted package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Maximum number of zip entries.
    pub max_parts: usize,
    /// Maximum uncompressed size of one part, in bytes.
    pub max_part_bytes: u64,
    /// Maximum uncompressed size of all parts together, in bytes.
    pub max_total_bytes: u64,
    /// Maximum uncompressed/compressed ratio for parts larger than `ratio_floor_bytes`.
    pub max_ratio: u64,
    /// Parts at or below this size are exempt from the ratio check.
    pub ratio_floor_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_parts: 20_000,
            max_part_bytes: 512 * 1024 * 1024,
            max_total_bytes: 2 * 1024 * 1024 * 1024,
            max_ratio: 200,
            ratio_floor_bytes: 1024 * 1024,
        }
    }
}

/// How a part is compressed inside the zip. Preserved on save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Stored,
    Deflated,
}

/// One zip entry with its uncompressed bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    name: PartName,
    data: Vec<u8>,
    compression: Compression,
    dirty: bool,
}

impl Part {
    pub fn name(&self) -> &PartName {
        &self.name
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn compression(&self) -> Compression {
        self.compression
    }

    /// True when the part was added or replaced since the package was read.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

/// An OPC package: all entries in original zip order. Directory entries are dropped
/// because they carry no content.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Package {
    parts: Vec<Part>,
    /// Lower-cased part name to position in `parts`. Always rebuilt or patched together
    /// with `parts`, so lookups are O(1) and zip order is untouched.
    index: HashMap<String, usize>,
}

impl Package {
    /// Reads a package from zip bytes, enforcing `limits`.
    pub fn read(bytes: &[u8], limits: &Limits) -> Result<Self, OpcError> {
        let mut archive = ZipArchive::new(Cursor::new(bytes))?;
        if archive.len() > limits.max_parts {
            return Err(OpcError::LimitExceeded(format!(
                "{} entries (max {})",
                archive.len(),
                limits.max_parts
            )));
        }
        let mut parts = Vec::with_capacity(archive.len());
        let mut index = HashMap::with_capacity(archive.len());
        let mut total: u64 = 0;
        for position in 0..archive.len() {
            let mut entry = archive.by_index(position)?;
            if entry.is_dir() {
                continue;
            }
            let name = PartName::new(&entry.name()?)?;
            if index.insert(name.key(), parts.len()).is_some() {
                return Err(OpcError::DuplicatePart(name.to_string()));
            }
            let too_big = |size: u64| {
                OpcError::LimitExceeded(format!(
                    "part {name} is {size} bytes (max {})",
                    limits.max_part_bytes
                ))
            };
            if entry.size() > limits.max_part_bytes {
                return Err(too_big(entry.size()));
            }
            let compression = match entry.compression() {
                CompressionMethod::Stored => Compression::Stored,
                _ => Compression::Deflated,
            };
            let compressed_size = entry.compressed_size().max(1);
            // The declared size can lie, so cap the actual read as well.
            let mut data = Vec::new();
            entry
                .by_ref()
                .take(limits.max_part_bytes + 1)
                .read_to_end(&mut data)?;
            let size = data.len() as u64;
            if size > limits.max_part_bytes {
                return Err(too_big(size));
            }
            if size > limits.ratio_floor_bytes && size / compressed_size > limits.max_ratio {
                return Err(OpcError::LimitExceeded(format!(
                    "part {name} expands {}x (max {}x)",
                    size / compressed_size,
                    limits.max_ratio
                )));
            }
            total += size;
            if total > limits.max_total_bytes {
                return Err(OpcError::LimitExceeded(format!(
                    "total uncompressed size over {} bytes",
                    limits.max_total_bytes
                )));
            }
            parts.push(Part {
                name,
                data,
                compression,
                dirty: false,
            });
        }
        Ok(Self { parts, index })
    }

    /// Writes the package as zip bytes, keeping part order and compression method.
    pub fn write(&self) -> Result<Vec<u8>, OpcError> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for part in &self.parts {
            let method = match part.compression {
                Compression::Stored => CompressionMethod::Stored,
                Compression::Deflated => CompressionMethod::Deflated,
            };
            writer.start_file(
                part.name.as_str(),
                SimpleFileOptions::default().compression_method(method),
            )?;
            writer.write_all(&part.data)?;
        }
        Ok(writer.finish()?.into_inner())
    }

    pub fn parts(&self) -> impl Iterator<Item = &Part> {
        self.parts.iter()
    }

    /// Looks up a part by name, case-insensitively, in O(1).
    pub fn part(&self, name: &str) -> Option<&Part> {
        self.position(name).map(|i| &self.parts[i])
    }

    fn position(&self, name: &str) -> Option<usize> {
        // Names that are already lower case (the common lookup) need no allocation.
        if name.bytes().any(|b| b.is_ascii_uppercase()) {
            self.index.get(&name.to_ascii_lowercase()).copied()
        } else {
            self.index.get(name).copied()
        }
    }

    /// Replaces a part's bytes, or appends a new deflated part. Either way it becomes dirty.
    pub fn set_part(&mut self, name: PartName, data: Vec<u8>) {
        match self.position(name.as_str()) {
            Some(i) => {
                let part = &mut self.parts[i];
                part.data = data;
                part.dirty = true;
            }
            None => {
                self.index.insert(name.key(), self.parts.len());
                self.parts.push(Part {
                    name,
                    data,
                    compression: Compression::Deflated,
                    dirty: true,
                });
            }
        }
    }

    /// Removes a part by name, case-insensitively. Later parts keep their relative order.
    pub fn remove_part(&mut self, name: &str) -> Option<Part> {
        let position = self.position(name)?;
        let part = self.parts.remove(position);
        self.index.remove(&part.name.key());
        for slot in self.index.values_mut() {
            if *slot > position {
                *slot -= 1;
            }
        }
        Some(part)
    }

    /// Removes a part and, in the same step, its `Override` in `[Content_Types].xml`, so no
    /// dangling override is left behind. Only the bytes of the matching `<Override>`
    /// elements are cut; the rest of the content-types part stays byte-identical. Returns
    /// the removed part, if there was one. The override is also cut when the part itself is
    /// already gone.
    pub fn remove_part_and_override(&mut self, name: &str) -> Option<Part> {
        let removed = self.remove_part(name);
        if let Some(types) = self.part(CONTENT_TYPES_PART)
            && let Some(cut) = without_override(types.data(), name)
            && let Ok(types_name) = PartName::new(CONTENT_TYPES_PART)
        {
            self.set_part(types_name, cut);
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    use super::{Compression, Limits, Package};
    use crate::{OpcError, PartName};

    fn zip_of(entries: &[(&str, &[u8], CompressionMethod)]) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data, method) in entries {
            writer
                .start_file(
                    *name,
                    SimpleFileOptions::default().compression_method(*method),
                )
                .unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn sample() -> Vec<u8> {
        zip_of(&[
            (
                "[Content_Types].xml",
                b"<Types/>",
                CompressionMethod::Deflated,
            ),
            (
                "_rels/.rels",
                b"<Relationships/>",
                CompressionMethod::Deflated,
            ),
            (
                "word/document.xml",
                b"<w:document/>",
                CompressionMethod::Deflated,
            ),
            (
                "word/media/image1.png",
                b"\x89PNG",
                CompressionMethod::Stored,
            ),
        ])
    }

    fn snapshot(package: &Package) -> Vec<(String, Vec<u8>, Compression)> {
        package
            .parts()
            .map(|p| (p.name().to_string(), p.data().to_vec(), p.compression()))
            .collect()
    }

    #[test]
    fn round_trip_keeps_names_order_bytes_and_compression() {
        let original = Package::read(&sample(), &Limits::default()).unwrap();
        let reread = Package::read(&original.write().unwrap(), &Limits::default()).unwrap();
        assert_eq!(snapshot(&original), snapshot(&reread));
        assert!(reread.parts().all(|p| !p.is_dirty()));
    }

    #[test]
    fn set_part_changes_only_that_part() {
        let mut package = Package::read(&sample(), &Limits::default()).unwrap();
        let before = snapshot(&package);
        package.set_part(
            PartName::new("WORD/Document.xml").unwrap(),
            b"<w:document>edited</w:document>".to_vec(),
        );
        let after = Package::read(&package.write().unwrap(), &Limits::default()).unwrap();
        let after = snapshot(&after);
        assert_eq!(after.len(), before.len());
        for (old, new) in before.iter().zip(&after) {
            if old.0 == "word/document.xml" {
                assert_eq!(new.1, b"<w:document>edited</w:document>");
            } else {
                assert_eq!(old, new);
            }
        }
        assert_eq!(
            package
                .parts()
                .filter(|p| p.is_dirty())
                .map(|p| p.name().as_str())
                .collect::<Vec<_>>(),
            ["word/document.xml"]
        );
    }

    #[test]
    fn rejects_path_traversal_entry() {
        let bytes = zip_of(&[("../evil.xml", b"x", CompressionMethod::Stored)]);
        assert!(matches!(
            Package::read(&bytes, &Limits::default()),
            Err(OpcError::InvalidPartName { .. })
        ));
    }

    #[test]
    fn rejects_case_insensitive_duplicate() {
        let bytes = zip_of(&[
            ("word/document.xml", b"a", CompressionMethod::Stored),
            ("Word/Document.xml", b"b", CompressionMethod::Stored),
        ]);
        assert!(matches!(
            Package::read(&bytes, &Limits::default()),
            Err(OpcError::DuplicatePart(_))
        ));
    }

    #[test]
    fn rejects_too_many_entries() {
        let limits = Limits {
            max_parts: 3,
            ..Limits::default()
        };
        assert!(matches!(
            Package::read(&sample(), &limits),
            Err(OpcError::LimitExceeded(_))
        ));
    }

    #[test]
    fn rejects_oversized_part() {
        let bytes = zip_of(&[("big.xml", &[0; 4096], CompressionMethod::Deflated)]);
        let limits = Limits {
            max_part_bytes: 1024,
            ..Limits::default()
        };
        assert!(matches!(
            Package::read(&bytes, &limits),
            Err(OpcError::LimitExceeded(_))
        ));
    }

    #[test]
    fn rejects_zip_bomb_ratio() {
        let zeros = vec![0; 4 * 1024 * 1024];
        let bytes = zip_of(&[("bomb.xml", &zeros, CompressionMethod::Deflated)]);
        assert!(matches!(
            Package::read(&bytes, &Limits::default()),
            Err(OpcError::LimitExceeded(_))
        ));
    }

    #[test]
    fn rejects_non_zip_input() {
        assert!(matches!(
            Package::read(b"not a zip", &Limits::default()),
            Err(OpcError::Zip(_))
        ));
    }

    #[test]
    fn index_is_case_insensitive_and_survives_set_and_remove() {
        let mut package = Package::read(&sample(), &Limits::default()).unwrap();
        for name in [
            "WORD/DOCUMENT.XML",
            "word/document.xml",
            "Word/Document.xml",
        ] {
            assert!(package.part(name).is_some(), "{name}");
        }
        assert!(package.part("word/missing.xml").is_none());
        package.set_part(PartName::new("Extra/New.xml").unwrap(), b"n".to_vec());
        assert_eq!(package.part("extra/new.xml").unwrap().data(), b"n");
        let removed = package.remove_part("_RELS/.RELS").unwrap();
        assert_eq!(removed.name().as_str(), "_rels/.rels");
        assert!(package.part("_rels/.rels").is_none());
        // Positions after the removed part still resolve to the right part.
        for part in package
            .parts()
            .map(|p| p.name().clone())
            .collect::<Vec<_>>()
        {
            assert_eq!(package.part(part.as_str()).unwrap().name(), &part);
        }
        let names: Vec<_> = package.parts().map(|p| p.name().as_str()).collect();
        assert_eq!(
            names,
            [
                "[Content_Types].xml",
                "word/document.xml",
                "word/media/image1.png",
                "Extra/New.xml"
            ]
        );
    }

    #[test]
    fn remove_part_and_override_leaves_other_bytes_alone() {
        let types = b"<Types xmlns=\"x\">
 <Override PartName=\"/a.xml\" ContentType=\"a\"/>
 <Override PartName=\"/b.xml\" ContentType=\"b\"/>
</Types>";
        let mut package = Package::default();
        package.set_part(
            PartName::new("[Content_Types].xml").unwrap(),
            types.to_vec(),
        );
        package.set_part(PartName::new("a.xml").unwrap(), b"a".to_vec());
        assert!(package.remove_part_and_override("A.xml").is_some());
        assert!(package.part("a.xml").is_none());
        assert_eq!(
            package.part("[Content_Types].xml").unwrap().data(),
            b"<Types xmlns=\"x\">
 
 <Override PartName=\"/b.xml\" ContentType=\"b\"/>
</Types>"
        );
    }
}
