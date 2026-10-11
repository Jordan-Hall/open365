//! Content-addressed immutable blobs behind a trait. The local-disk store derives every path from validated
//! hex ids, never from user input, so path traversal is impossible by construction.

use std::fmt::Debug;
use std::io::{self, Write as _};
use std::path::PathBuf;

use crate::util::{is_hex_id, make_private_dir, new_id, sha256_hex};

pub trait BlobStore: Send + Sync + Debug {
    /// Stores `data` for `tenant` and returns its blob id (lowercase SHA-256 hex).
    fn put(&self, tenant: &str, data: &[u8]) -> io::Result<String>;
    /// Returns the bytes after re-checking their hash.
    fn get(&self, tenant: &str, blob_id: &str) -> io::Result<Vec<u8>>;
    /// Removes a blob; a missing blob is not an error. The caller checks that no version still references it.
    fn delete(&self, tenant: &str, blob_id: &str) -> io::Result<()>;
}

#[derive(Debug)]
pub struct LocalDiskBlobStore {
    root: PathBuf,
}

fn bad(msg: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, msg)
}

impl LocalDiskBlobStore {
    pub fn new(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        make_private_dir(&root)?;
        Ok(Self { root })
    }

    fn path(&self, tenant: &str, blob_id: &str) -> io::Result<PathBuf> {
        if !is_hex_id(tenant, 32) {
            return Err(bad("invalid tenant id"));
        }
        if !is_hex_id(blob_id, 64) {
            return Err(bad("invalid blob id"));
        }
        Ok(self.root.join(tenant).join(&blob_id[..2]).join(blob_id))
    }
}

impl BlobStore for LocalDiskBlobStore {
    fn put(&self, tenant: &str, data: &[u8]) -> io::Result<String> {
        let id = sha256_hex(data);
        let dest = self.path(tenant, &id)?;
        if dest.exists() {
            // Refresh the modification time so that `gc-blobs` (which spares recent files) cannot delete a blob that
            // is about to be referenced again.
            if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&dest) {
                let _ = f.set_modified(std::time::SystemTime::now());
            }
            return Ok(id);
        }
        let dir = dest.parent().ok_or_else(|| bad("no parent"))?;
        make_private_dir(dir)?;
        let tmp = dir.join(format!(".tmp-{}", new_id()));
        {
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = opts.open(&tmp)?;
            f.write_all(data)?;
            f.sync_all()?;
        }
        match std::fs::rename(&tmp, &dest) {
            Ok(()) => Ok(id),
            Err(_) if dest.exists() => {
                let _ = std::fs::remove_file(&tmp);
                Ok(id)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    fn get(&self, tenant: &str, blob_id: &str) -> io::Result<Vec<u8>> {
        let data = std::fs::read(self.path(tenant, blob_id)?)?;
        if sha256_hex(&data) != blob_id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "blob hash mismatch",
            ));
        }
        Ok(data)
    }

    fn delete(&self, tenant: &str, blob_id: &str) -> io::Result<()> {
        match std::fs::remove_file(self.path(tenant, blob_id)?) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T1: &str = "00000000000000000000000000000001";
    const T2: &str = "00000000000000000000000000000002";

    #[test]
    fn round_trip_is_byte_identical_and_tenant_scoped() {
        let dir = tempfile::tempdir().expect("tmp");
        let s = LocalDiskBlobStore::new(dir.path()).expect("store");
        let data = vec![0u8, 1, 2, 255, 254];
        let id = s.put(T1, &data).expect("put");
        assert_eq!(s.get(T1, &id).expect("get"), data);
        assert_eq!(s.put(T1, &data).expect("again"), id);
        assert!(
            s.get(T2, &id).is_err(),
            "another tenant must not see the blob"
        );
    }

    #[test]
    fn traversal_ids_are_rejected() {
        let dir = tempfile::tempdir().expect("tmp");
        let s = LocalDiskBlobStore::new(dir.path()).expect("store");
        let ok = "a".repeat(64);
        assert!(s.get("../../etc", &ok).is_err());
        assert!(s.get(T1, "../../../secret").is_err());
        assert!(s.get(T1, &format!("{}/..", "a".repeat(61))).is_err());
        assert!(s.put("..", b"x").is_err());
    }

    #[test]
    fn corrupted_blob_is_detected() {
        let dir = tempfile::tempdir().expect("tmp");
        let s = LocalDiskBlobStore::new(dir.path()).expect("store");
        let id = s.put(T1, b"hello").expect("put");
        let p = dir.path().join(T1).join(&id[..2]).join(&id);
        std::fs::write(p, b"tampered").expect("write");
        assert!(s.get(T1, &id).is_err());
    }
}
