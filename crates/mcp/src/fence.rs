//! Fenced roots: every path an agent names must resolve, after symlinks are followed, to a
//! location inside one of the configured root folders.

use std::path::{Component, Path, PathBuf, Prefix};

/// Why a path was refused.
#[derive(Debug, PartialEq, Eq)]
pub struct Denied(pub String);

impl Denied {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

#[derive(Debug)]
pub struct Fence {
    roots: Vec<PathBuf>,
}

/// Windows device names, with or without an extension, in any case. Includes the console
/// devices `CONIN$` and `CONOUT$`, `COM0` and `LPT0`, and the superscript-digit forms
/// `COM¹²³` and `LPT¹²³`, which the Win32 path rules also treat as devices.
const RESERVED: &[&str] = &[
    "CON",
    "PRN",
    "AUX",
    "NUL",
    "CONIN$",
    "CONOUT$",
    "COM0",
    "COM1",
    "COM2",
    "COM3",
    "COM4",
    "COM5",
    "COM6",
    "COM7",
    "COM8",
    "COM9",
    "COM\u{b9}",
    "COM\u{b2}",
    "COM\u{b3}",
    "LPT0",
    "LPT1",
    "LPT2",
    "LPT3",
    "LPT4",
    "LPT5",
    "LPT6",
    "LPT7",
    "LPT8",
    "LPT9",
    "LPT\u{b9}",
    "LPT\u{b2}",
    "LPT\u{b3}",
];

/// Rejects shapes that are never a plain file below a root, before touching the file system:
/// UNC and device paths (`\host\share`, `\?\`, `\.\`), alternate data streams and
/// reserved device names.
fn check_shape(raw: &str) -> Result<(), Denied> {
    if raw.is_empty() || raw.contains('\0') {
        return Err(Denied::new("empty path or NUL byte"));
    }
    if raw.starts_with("\\\\") || raw.starts_with("//") {
        return Err(Denied::new("UNC and device paths are not allowed"));
    }
    for component in Path::new(raw).components() {
        match component {
            Component::Prefix(p) if !matches!(p.kind(), Prefix::Disk(_)) => {
                return Err(Denied::new("UNC and device paths are not allowed"));
            }
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                if name.contains(':') {
                    return Err(Denied::new("alternate data streams are not allowed"));
                }
                let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
                if RESERVED.contains(&stem.trim_end().to_string().as_str()) {
                    return Err(Denied::new("reserved device name"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

impl Fence {
    /// Canonicalises each root; every root must be an existing directory.
    pub fn new(roots: &[PathBuf]) -> Result<Self, String> {
        if roots.is_empty() {
            return Err("at least one --root folder is required".to_owned());
        }
        let mut out = Vec::new();
        for root in roots {
            check_shape(&root.to_string_lossy())
                .map_err(|d| format!("root {}: {}", root.display(), d.0))?;
            let canon = root
                .canonicalize()
                .map_err(|e| format!("root {}: {e}", root.display()))?;
            if !canon.is_dir() {
                return Err(format!("root {} is not a directory", root.display()));
            }
            out.push(canon);
        }
        Ok(Self { roots: out })
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    fn inside(&self, canonical: &Path) -> bool {
        self.roots.iter().any(|r| canonical.starts_with(r))
    }

    /// Relative paths resolve against the first root.
    fn absolute(&self, raw: &str) -> PathBuf {
        let p = Path::new(raw);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.roots[0].join(p)
        }
    }

    /// An existing file: canonical path inside a root. Symlinks are followed first, so a link
    /// that points outside is refused.
    pub fn resolve_existing(&self, raw: &str) -> Result<PathBuf, Denied> {
        check_shape(raw)?;
        let canon = self
            .absolute(raw)
            .canonicalize()
            .map_err(|e| Denied::new(format!("cannot resolve path: {e}")))?;
        if !self.inside(&canon) {
            return Err(Denied::new("path is outside the allowed roots"));
        }
        if !canon.is_file() {
            return Err(Denied::new("path is not a file"));
        }
        Ok(canon)
    }

    /// A file to create or overwrite: its parent must resolve inside a root, the name must be
    /// one plain component, and an existing symlink at the target is refused.
    pub fn resolve_new(&self, raw: &str) -> Result<PathBuf, Denied> {
        check_shape(raw)?;
        let path = self.absolute(raw);
        let name = path
            .file_name()
            .ok_or_else(|| Denied::new("path has no file name"))?
            .to_owned();
        let parent = path
            .parent()
            .ok_or_else(|| Denied::new("path has no parent"))?
            .canonicalize()
            .map_err(|e| Denied::new(format!("cannot resolve parent folder: {e}")))?;
        if !self.inside(&parent) {
            return Err(Denied::new("path is outside the allowed roots"));
        }
        let target = parent.join(&name);
        if let Ok(meta) = std::fs::symlink_metadata(&target) {
            if meta.file_type().is_symlink() {
                return Err(Denied::new("target is a symbolic link"));
            }
            let canon = target
                .canonicalize()
                .map_err(|e| Denied::new(format!("cannot resolve target: {e}")))?;
            if !self.inside(&canon) || !canon.is_file() {
                return Err(Denied::new("target is not a file inside the allowed roots"));
            }
        }
        Ok(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_windows_device_name_is_refused_in_any_case_and_with_an_extension() {
        for name in [
            "CONIN$",
            "conout$",
            "COM0",
            "lpt0.txt",
            "COM\u{b9}",
            "com\u{b2}.docx",
            "LPT\u{b3}",
            "CONIN$.docx",
            "NUL",
            "com1 .x",
            "Aux.tar.gz",
        ] {
            assert_eq!(
                check_shape(name),
                Err(Denied::new("reserved device name")),
                "{name}"
            );
            assert!(check_shape(&format!("sub/{name}")).is_err(), "{name}");
        }
        for name in ["COM10", "CONSOLE.docx", "LPT", "report.docx", "COM\u{2074}"] {
            assert!(check_shape(name).is_ok(), "{name}");
        }
    }
}
