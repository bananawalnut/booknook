use crate::{NativeError, Result};
use booknook_core::model::ContentAsset;
use std::{
    io::{Read, Write},
    path::{Component, Path},
};
const MAX: u64 = 128 * 1024 * 1024;
fn error(s: impl Into<String>) -> NativeError {
    NativeError::Failure { message: s.into() }
}
pub fn safe_relative(s: &str) -> Result<&Path> {
    let path = Path::new(s);
    if s.is_empty()
        || s.contains('\\')
        || s.contains('\0')
        || path
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err(error("Unsafe archive path"));
    }
    Ok(path)
}

pub fn import(root: &Path, bytes: &[u8], kind: &str) -> Result<ContentAsset> {
    if bytes.len() as u64 > MAX {
        return Err(error("Content exceeds 128 MiB"));
    }
    let hash = booknook_core::digest(bytes);
    let folder = root.join("assets").join(&hash);
    let staging = root
        .join("assets")
        .join(format!(".{}", booknook_core::new_id()));
    std::fs::create_dir_all(&staging)?;
    let result = (|| {
        let (media_type, spine) = match kind {
            "pdf" => {
                if !bytes.starts_with(b"%PDF-") {
                    return Err(error("Invalid PDF header"));
                }
                ("application/pdf", vec!["book.pdf".into()])
            }
            "epub" => {
                let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
                    .map_err(|e| error(e.to_string()))?;
                if zip.len() > 10000 {
                    return Err(error("EPUB has too many entries"));
                }
                let mut total = 0u64;
                for i in 0..zip.len() {
                    let mut entry = zip.by_index(i).map_err(|e| error(e.to_string()))?;
                    if entry.is_dir() {
                        continue;
                    }
                    if entry.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000) {
                        return Err(error("EPUB symlinks are forbidden"));
                    }
                    let relative = safe_relative(entry.name())?;
                    total = total
                        .checked_add(entry.size())
                        .ok_or_else(|| error("EPUB size overflow"))?;
                    if total > MAX || entry.size() > 32 * 1024 * 1024 {
                        return Err(error("EPUB expansion limit exceeded"));
                    }
                    let target = staging.join(relative);
                    if target.exists() {
                        return Err(error("Duplicate EPUB path"));
                    }
                    std::fs::create_dir_all(target.parent().unwrap())?;
                    let mut file = std::fs::File::create(target)?;
                    let copied =
                        std::io::copy(&mut entry.by_ref().take(32 * 1024 * 1024 + 1), &mut file)?;
                    if copied > 32 * 1024 * 1024 {
                        return Err(error("EPUB entry too large"));
                    }
                }
                let mime = std::fs::read_to_string(staging.join("mimetype"))?;
                if mime.trim() != "application/epub+zip" {
                    return Err(error("Not an EPUB archive"));
                }
                if staging.join("META-INF/encryption.xml").exists() {
                    return Err(error(
                        "Encrypted EPUB resources are not supported; catalog and notes remain available",
                    ));
                }
                let container = std::fs::read_to_string(staging.join("META-INF/container.xml"))?;
                let doc =
                    roxmltree::Document::parse(&container).map_err(|e| error(e.to_string()))?;
                let package = doc
                    .descendants()
                    .find(|n| n.has_tag_name("rootfile"))
                    .and_then(|n| n.attribute("full-path"))
                    .ok_or_else(|| error("EPUB package missing"))?;
                let package_path = safe_relative(package)?;
                let opf = std::fs::read_to_string(staging.join(package_path))?;
                let doc = roxmltree::Document::parse(&opf).map_err(|e| error(e.to_string()))?;
                let mut spine = vec![];
                for item in doc.descendants().filter(|n| n.has_tag_name("itemref")) {
                    let id = item
                        .attribute("idref")
                        .ok_or_else(|| error("Invalid spine"))?;
                    let manifest = doc
                        .descendants()
                        .find(|n| n.has_tag_name("item") && n.attribute("id") == Some(id))
                        .ok_or_else(|| error("Spine resource missing"))?;
                    if manifest.attribute("media-type") != Some("application/xhtml+xml") {
                        return Err(error("Only XHTML EPUB spine resources are supported"));
                    }
                    let href = manifest
                        .attribute("href")
                        .ok_or_else(|| error("Spine href missing"))?;
                    let relative = package_path
                        .parent()
                        .unwrap_or(Path::new(""))
                        .join(safe_relative(href)?);
                    if !staging.join(&relative).is_file() {
                        return Err(error("Spine file missing"));
                    }
                    spine.push(relative.to_string_lossy().into_owned());
                }
                if spine.is_empty() {
                    return Err(error("EPUB has no readable spine"));
                }
                ("application/epub+zip", spine)
            }
            _ => {
                return Err(error(
                    "Choose an accessible EPUB or PDF; protected Kindle content cannot be rendered",
                ));
            }
        };
        let original = if kind == "pdf" {
            "book.pdf"
        } else {
            "original.epub"
        };
        let mut file = std::fs::File::create(staging.join(original))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if folder.exists() {
            std::fs::remove_dir_all(&staging)?;
        } else {
            std::fs::rename(&staging, &folder)?;
        }
        Ok(ContentAsset {
            id: hash.clone(),
            media_type: media_type.into(),
            storage_key: hash.clone(),
            sha256: hash,
            spine,
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}
