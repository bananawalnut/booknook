// SPDX-License-Identifier: MIT OR Apache-2.0
use crate::{NativeError, Result};
use std::{
    io::{Read, Write},
    path::Path,
};

pub fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let limit = booknook_core::MAX_JSON_BYTES;
    if file.metadata()?.len() > limit as u64 {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(too_large());
    }
    Ok(bytes)
}
fn too_large() -> NativeError {
    NativeError::Failure {
        message: "Portable file exceeds the development build's 128 MiB limit".into(),
    }
}
struct BoundedWriter<W> {
    inner: W,
    remaining: usize,
}
impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.remaining {
            return Err(std::io::Error::other(
                "Portable export exceeds 128 MiB; existing export is unchanged",
            ));
        }
        let written = self.inner.write(bytes)?;
        self.remaining -= written;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
pub fn write_atomic(path: &Path, value: &impl serde::Serialize, limit: usize) -> Result<()> {
    let temp = path.with_extension(format!("{}.tmp", booknook_core::new_id()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        {
            let mut buffered = std::io::BufWriter::new(&mut file);
            serde_json::to_writer(
                BoundedWriter {
                    inner: &mut buffered,
                    remaining: limit,
                },
                value,
            )?;
            buffered.flush()?;
        }
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejected_export_preserves_previous_file_and_removes_partial_output() {
        let dir = std::env::temp_dir().join(format!("booknook-export-{}", booknook_core::new_id()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("export.json");
        std::fs::write(&path, "previous export").unwrap();
        assert!(write_atomic(&path, &vec!["a"; 100], 24).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous export");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        write_atomic(&path, &vec!["a"], 24).unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "[\"a\"]");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
