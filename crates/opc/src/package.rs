use std::collections::HashSet;
use std::io::{Cursor, Read, Write};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

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
        let mut seen = HashSet::new();
        let mut total: u64 = 0;
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index)?;
            if entry.is_dir() {
                continue;
            }
            let name = PartName::new(&entry.name()?)?;
            if !seen.insert(name.key()) {
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
        Ok(Self { parts })
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

    /// Looks up a part by name, case-insensitively.
    pub fn part(&self, name: &str) -> Option<&Part> {
        let key = name.to_ascii_lowercase();
        self.parts.iter().find(|p| p.name.key() == key)
    }

    /// Replaces a part's bytes, or appends a new deflated part. Either way it becomes dirty.
    pub fn set_part(&mut self, name: PartName, data: Vec<u8>) {
        let key = name.key();
        match self.parts.iter_mut().find(|p| p.name.key() == key) {
            Some(part) => {
                part.data = data;
                part.dirty = true;
            }
            None => self.parts.push(Part {
                name,
                data,
                compression: Compression::Deflated,
                dirty: true,
            }),
        }
    }

    /// Removes a part by name, case-insensitively.
    pub fn remove_part(&mut self, name: &str) -> Option<Part> {
        let key = name.to_ascii_lowercase();
        let index = self.parts.iter().position(|p| p.name.key() == key)?;
        Some(self.parts.remove(index))
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
}
