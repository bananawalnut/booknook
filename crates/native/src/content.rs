use crate::{NativeError, Result};
use booknook_core::model::{ContentAsset, MOBI6_MEDIA_TYPE};

mod mobi6;
use std::{
    fs::{self, File},
    io::{ErrorKind, Read, Write},
    path::{Component, Path},
};
const MAX: u64 = 128 * 1024 * 1024;
// Imported positions and annotations store this resource identifier. Keep it
// stable for the v1 transformation, including when rehydrating old backups.
const MOBI6_SPINE: &str = "mobi6-v1/book.xhtml";

#[derive(Clone, Copy)]
pub(crate) enum ContentKind {
    Epub,
    Pdf,
    Mobi6,
}

impl ContentKind {
    pub(crate) fn from_extension(extension: &str) -> Result<Self> {
        match extension {
            "epub" => Ok(Self::Epub),
            "pdf" => Ok(Self::Pdf),
            "mobi" | "azw" => Ok(Self::Mobi6),
            _ => Err(error(
                "Choose an EPUB, PDF, or DRM-free legacy MOBI (.mobi or .azw) file",
            )),
        }
    }

    pub(crate) fn from_media_type(media_type: &str) -> Result<Self> {
        match media_type {
            "application/epub+zip" => Ok(Self::Epub),
            "application/pdf" => Ok(Self::Pdf),
            MOBI6_MEDIA_TYPE => Ok(Self::Mobi6),
            _ => Err(error("Unsupported book content media type")),
        }
    }

    pub(crate) fn original_name(self) -> &'static str {
        match self {
            Self::Epub => "original.epub",
            Self::Pdf => "book.pdf",
            Self::Mobi6 => "original.mobi",
        }
    }
}
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

fn file_matches(path: &Path, expected: &[u8]) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err.into()),
    };
    if !metadata.is_file() {
        return Err(error("Stored MOBI content must use regular files"));
    }
    if metadata.len() != expected.len() as u64 {
        return Ok(false);
    }
    let mut file = File::open(path)?;
    let mut buffer = [0; 8192];
    for chunk in expected.chunks(buffer.len()) {
        let read = &mut buffer[..chunk.len()];
        file.read_exact(read)?;
        if read != chunk {
            return Ok(false);
        }
    }
    Ok(file.read(&mut buffer[..1])? == 0)
}

fn refresh_mobi_reader(
    folder: &Path,
    staging: &Path,
    original: &str,
    bytes: &[u8],
    xhtml: &[u8],
) -> Result<()> {
    if !fs::symlink_metadata(folder)?.is_dir() || !file_matches(&folder.join(original), bytes)? {
        return Err(error(
            "Stored MOBI original does not match the imported file",
        ));
    }
    let target = folder.join(MOBI6_SPINE);
    let parent = target.parent().unwrap();
    match fs::create_dir(parent) {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {
            if !fs::symlink_metadata(parent)?.is_dir() {
                return Err(error("Stored MOBI reader path is not a directory"));
            }
        }
        Err(err) => return Err(err.into()),
    }
    if !file_matches(&target, xhtml)? {
        // The freshly rendered file was synced in staging on the same volume.
        fs::rename(staging.join(MOBI6_SPINE), target)?;
    }
    Ok(())
}

pub(crate) fn import(root: &Path, bytes: &[u8], kind: ContentKind) -> Result<ContentAsset> {
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
        let mut rendered_mobi = None;
        let (media_type, spine) = match kind {
            ContentKind::Pdf => {
                if !bytes.starts_with(b"%PDF-") {
                    return Err(error("Invalid PDF header"));
                }
                ("application/pdf", vec!["book.pdf".into()])
            }
            ContentKind::Epub => {
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
                    let name = entry.name().map_err(|e| error(e.to_string()))?;
                    let relative = safe_relative(name.as_ref())?;
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
            ContentKind::Mobi6 => {
                let xhtml = mobi6::render_mobi6(bytes)?;
                let target = staging.join(MOBI6_SPINE);
                std::fs::create_dir_all(target.parent().unwrap())?;
                let mut file = std::fs::File::create(target)?;
                file.write_all(&xhtml)?;
                file.sync_all()?;
                rendered_mobi = Some(xhtml);
                (MOBI6_MEDIA_TYPE, vec![MOBI6_SPINE.into()])
            }
        };
        let original = kind.original_name();
        let mut file = std::fs::File::create(staging.join(original))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if folder.exists() {
            if let Some(xhtml) = rendered_mobi {
                refresh_mobi_reader(&folder, &staging, original, bytes, &xhtml)?;
            } else if !folder.join(original).is_file()
                || spine.iter().any(|path| !folder.join(path).is_file())
            {
                // An existing digest must identify a complete asset of this format.
                return Err(error("Stored book files do not match this content format"));
            }
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
