// Copyright 2026 ByteLand Technology Limited
// SPDX-License-Identifier: MIT

//! Managed download file naming, media header sniffing, path safety, and chunked reads.

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use url::Url;

use crate::config::downloads_dir;
use crate::db::{DownloadRecord, MusicDb, effective_mime_type};

/// Error marker used when a candidate URL serves HTML, JSON, playlists, or the wrong media type.
pub(crate) const MEDIA_KIND_MISMATCH_TAG: &str = "[media_kind_mismatch]";

/// Default chunk size (`1 MiB`) returned by `music_downloads` `read_chunk`.
const DEFAULT_CHUNK_BYTES: usize = 1_048_576;

/// Maximum chunk size (`4 MiB`) allowed by `music_downloads` `read_chunk`.
const MAX_CHUNK_BYTES: usize = 4_194_304;

/// Decodes one hexadecimal ASCII byte into its 4-bit integer value.
fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Decodes percent-escaped UTF-8 bytes in header and URL path strings.
fn percent_decode_utf8(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Sanitizes a candidate file name down to a safe non-empty leaf file name.
fn sanitize_filename_leaf(raw: &str) -> Option<String> {
    let leaf = raw.rsplit(['/', '\\']).next()?.trim();
    let cleaned: String = leaf
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('.').trim();
    let name = Path::new(trimmed).file_name().and_then(|n| n.to_str())?;
    if name.is_empty() || name == "." || name == ".." {
        None
    } else {
        Some(name.to_string())
    }
}

/// Normalizes an asset ID into a bounded ASCII stem for file names.
fn sanitize_asset_id(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches('_');
    if trimmed.is_empty() {
        "asset".to_string()
    } else {
        trimmed.chars().take(64).collect()
    }
}

/// Extracts a leaf file name from an HTTP `Content-Disposition` header.
fn extract_disposition_filename(disposition: &str) -> Option<String> {
    let mut plain_name: Option<String> = None;
    let mut ext_name: Option<String> = None;

    for part in disposition.split(';') {
        let trimmed = part.trim();
        let Some((raw_key, raw_val)) = trimmed.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        let val = raw_val.trim();

        if key.eq_ignore_ascii_case("filename*") {
            let unquoted = val.trim_matches('"');
            // RFC 5987 ext-value syntax: `charset'language'value-chars`.
            let mut parts = unquoted.splitn(3, '\'');
            let encoded = match (parts.next(), parts.next(), parts.next()) {
                (Some(_), Some(_), Some(value_chars)) => value_chars,
                _ => unquoted,
            };
            let decoded = percent_decode_utf8(encoded);
            if let Some(name) = sanitize_filename_leaf(&decoded) {
                ext_name = Some(name);
            }
        } else if key.eq_ignore_ascii_case("filename") && plain_name.is_none() {
            let clean = val.trim_matches('"').trim_matches('\'');
            let decoded = percent_decode_utf8(clean);
            if let Some(name) = sanitize_filename_leaf(&decoded) {
                plain_name = Some(name);
            }
        }
    }

    ext_name.or(plain_name)
}

/// Media family that a download must contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaKind {
    /// Audio file (MP3, M4A, WAV, FLAC, or Ogg).
    Audio,
    /// Video file (MP4 or WebM).
    Video,
    /// Image file (JPEG, PNG, GIF, or WebP).
    Image,
}

impl MediaKind {
    /// Returns the lowercase name of this media family.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            MediaKind::Audio => "audio",
            MediaKind::Video => "video",
            MediaKind::Image => "image",
        }
    }

    /// Returns the fallback file extension of this media family.
    fn default_extension(self) -> &'static str {
        match self {
            MediaKind::Audio => "m4a",
            MediaKind::Video => "mp4",
            MediaKind::Image => "jpg",
        }
    }
}

/// Picks the file extension from `content_type`, or the fallback extension of `kind`.
fn extension_for(kind: MediaKind, content_type: Option<&str>) -> &'static str {
    match mime_essence(content_type).as_str() {
        "audio/mpeg" | "audio/mp3" => return "mp3",
        "audio/mp4" | "audio/x-m4a" | "audio/aac" => return "m4a",
        "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => return "wav",
        "video/mp4" => return "mp4",
        "image/jpeg" => return "jpg",
        "image/png" => return "png",
        "image/webp" => return "webp",
        _ => {}
    }
    kind.default_extension()
}

/// Returns `true` when `name` has a non-empty alphanumeric file extension.
fn has_extension(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// Returns `true` when file extension `ext` fits the expected extension `expected`.
fn extension_fits(ext: &str, expected: &str) -> bool {
    let ext = ext.to_ascii_lowercase();
    match expected {
        "m4a" => matches!(ext.as_str(), "m4a" | "mp4" | "aac"),
        "wav" => matches!(ext.as_str(), "wav" | "wave"),
        "jpg" => matches!(ext.as_str(), "jpg" | "jpeg"),
        "mp4" => matches!(ext.as_str(), "mp4" | "m4v"),
        other => ext == other,
    }
}

/// Returns `name` with a lowercase extension when the extension fits the media type, else `None`.
///
/// The MP3 endpoint names files `<id>.AudioFormat.MP3`, thus this function also removes
/// the `.AudioFormat` segment.
fn fitting_name(name: String, kind: MediaKind, content_type: Option<&str>) -> Option<String> {
    let expected = extension_for(kind, content_type);
    let path = Path::new(&name);
    let ext = path.extension().and_then(|e| e.to_str())?;
    if !extension_fits(ext, expected) {
        return None;
    }
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    let stem = stem.strip_suffix(".AudioFormat").unwrap_or(stem);
    Some(format!("{}.{}", stem, ext.to_ascii_lowercase()))
}

/// Resolves the natural download file name from `Content-Disposition`, the URL path, or `<id>.<ext>`.
///
/// A name from a header or URL is used only when its extension fits the media type.
pub(crate) fn natural_filename(
    id: &str,
    kind: MediaKind,
    media_url: &str,
    content_type: Option<&str>,
    disposition: Option<&str>,
) -> String {
    if let Some(name) = disposition.and_then(extract_disposition_filename) {
        if !has_extension(&name) {
            return format!("{}.{}", name, extension_for(kind, content_type));
        }
        if let Some(fit) = fitting_name(name, kind, content_type) {
            return fit;
        }
    }
    if let Ok(parsed) = Url::parse(media_url)
        && let Some(raw_seg) = parsed.path_segments().and_then(|mut segs| segs.next_back())
        && let Some(clean) = sanitize_filename_leaf(&percent_decode_utf8(raw_seg))
        && has_extension(&clean)
        && let Some(fit) = fitting_name(clean, kind, content_type)
    {
        return fit;
    }
    format!(
        "{}.{}",
        sanitize_asset_id(id),
        extension_for(kind, content_type)
    )
}

/// Extracts the lowercased MIME essence (`type/subtype`) from a `Content-Type` header.
fn mime_essence(ct: Option<&str>) -> String {
    ct.and_then(|raw| raw.split(';').next())
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Detects the MIME type of a media file from its leading bytes.
fn sniff_mime(hdr: &[u8]) -> Option<&'static str> {
    let riff = |tag: &[u8]| hdr.len() >= 12 && &hdr[0..4] == b"RIFF" && &hdr[8..12] == tag;
    if hdr.starts_with(b"ID3") || (hdr.len() >= 2 && hdr[0] == 0xFF && hdr[1] & 0xE0 == 0xE0) {
        return Some("audio/mpeg");
    }
    if riff(b"WAVE") {
        return Some("audio/wav");
    }
    if hdr.starts_with(b"fLaC") {
        return Some("audio/flac");
    }
    if hdr.starts_with(b"OggS") {
        return Some("audio/ogg");
    }
    if hdr.len() >= 12 && &hdr[4..8] == b"ftyp" {
        let brand = &hdr[8..12];
        return Some(if matches!(brand, b"M4A " | b"M4B " | b"M4P ") {
            "audio/mp4"
        } else {
            "video/mp4"
        });
    }
    if hdr.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return Some("video/webm");
    }
    if hdr.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if hdr.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if hdr.starts_with(b"GIF87a") || hdr.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if riff(b"WEBP") {
        return Some("image/webp");
    }
    None
}

/// Checks the leading bytes of `path` against `kind` and returns the MIME type to index.
///
/// An MP4 container is valid for audio because M4A files use the same container.
pub(crate) async fn validate_downloaded_file(
    path: &Path,
    content_type: Option<&str>,
    kind: MediaKind,
) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut buf = [0u8; 64];
    let mut total = 0usize;
    while total < buf.len() {
        let n = file.read(&mut buf[total..]).await?;
        if n == 0 {
            break;
        }
        total += n;
    }
    let sniffed = sniff_mime(&buf[..total]).ok_or_else(|| {
        anyhow::anyhow!(
            "{} The download is not a known {} file",
            MEDIA_KIND_MISMATCH_TAG,
            kind.as_str()
        )
    })?;
    let family_ok = match kind {
        MediaKind::Audio => sniffed.starts_with("audio/") || sniffed == "video/mp4",
        MediaKind::Video => sniffed.starts_with("video/"),
        MediaKind::Image => sniffed.starts_with("image/"),
    };
    if !family_ok {
        anyhow::bail!(
            "{} The download contains {} instead of {}",
            MEDIA_KIND_MISMATCH_TAG,
            sniffed,
            kind.as_str()
        );
    }
    let declared = mime_essence(content_type);
    let mime = if kind == MediaKind::Audio && sniffed == "video/mp4" {
        "audio/mp4".to_string()
    } else if declared.is_empty() || declared.ends_with("octet-stream") {
        sniffed.to_string()
    } else {
        declared
    };
    Ok(mime)
}

/// Resolves an absolute path string for SQLite indexing.
pub(crate) fn resolve_stored_path(path: &Path) -> String {
    if path.is_absolute() {
        path.to_string_lossy().to_string()
    } else if let Ok(cwd) = std::env::current_dir() {
        cwd.join(path).to_string_lossy().to_string()
    } else {
        path.to_string_lossy().to_string()
    }
}

/// Disambiguates destination file paths in `dir` unless `asset_id` already owns that path.
pub(crate) async fn disambiguate_dest(
    db: &MusicDb,
    dir: &Path,
    filename: &str,
    asset_id: &str,
) -> PathBuf {
    let candidate = dir.join(filename);
    let abs_candidate = resolve_stored_path(&candidate);
    let exists = tokio::fs::try_exists(&candidate).await.unwrap_or(false);
    if !exists {
        return candidate;
    }

    if let Ok(Some(owner)) = db.find_owner_for_path(&abs_candidate).await
        && owner == asset_id
    {
        return candidate;
    }

    let path_obj = Path::new(filename);
    let stem = path_obj
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("asset");
    let ext = path_obj.extension().and_then(|s| s.to_str());
    let short_id = sanitize_asset_id(asset_id);
    let short_tag: String = short_id.chars().take(8).collect();
    let alt = match ext {
        Some(e) if !e.is_empty() => format!("{}-{}.{}", stem, short_tag, e),
        _ => format!("{}-{}", stem, short_tag),
    };
    let alt_path = dir.join(alt);
    if !tokio::fs::try_exists(&alt_path).await.unwrap_or(false) {
        return alt_path;
    }
    let uniq = match ext {
        Some(e) if !e.is_empty() => format!("{}-{}.{}", stem, short_id, e),
        _ => format!("{}-{}", stem, short_id),
    };
    dir.join(uniq)
}

/// Returns the canonical path when `file_path` is an existing regular file inside `downloads_dir()`.
pub(crate) fn verified_managed_download_path(file_path: &str) -> Option<PathBuf> {
    let raw_dir = downloads_dir();
    let base = std::fs::canonicalize(&raw_dir).ok()?;
    let canon = std::fs::canonicalize(file_path).ok()?;
    if canon.starts_with(&base) && canon != base && canon.is_file() {
        Some(canon)
    } else {
        None
    }
}

/// Reads a Base64-encoded byte range from the managed download file in `rec`.
pub(crate) async fn read_download_chunk(
    rec: &DownloadRecord,
    offset: u64,
    length: Option<usize>,
) -> Result<Value> {
    let path = verified_managed_download_path(&rec.file_path).with_context(|| {
        format!(
            "Downloaded file does not exist in managed downloads directory: {}",
            rec.file_path
        )
    })?;
    let mut file = tokio::fs::File::open(&path)
        .await
        .with_context(|| format!("Failed to open downloaded file {}", path.display()))?;
    let total_bytes = file.metadata().await?.len();
    let chunk_len = length
        .unwrap_or(DEFAULT_CHUNK_BYTES)
        .clamp(1, MAX_CHUNK_BYTES);
    let mime_type = effective_mime_type(rec.mime_type.as_deref(), &rec.file_path);

    if offset >= total_bytes {
        return Ok(json!({
            "action": "read_chunk",
            "id": rec.id,
            "asset_id": rec.asset_id,
            "resource_uri": rec.resource_uri,
            "file_name": rec.file_name,
            "mime_type": mime_type,
            "offset": offset,
            "bytes_read": 0,
            "total_bytes": total_bytes,
            "eof": true,
            "next_offset": Value::Null,
            "data_base64": "",
        }));
    }

    file.seek(std::io::SeekFrom::Start(offset)).await?;
    let to_read = (total_bytes - offset).min(chunk_len as u64);
    let mut buf = Vec::with_capacity(to_read as usize);
    file.take(to_read).read_to_end(&mut buf).await?;

    let next = offset + buf.len() as u64;
    let eof = next >= total_bytes;
    Ok(json!({
        "action": "read_chunk",
        "id": rec.id,
        "asset_id": rec.asset_id,
        "resource_uri": rec.resource_uri,
        "file_name": rec.file_name,
        "mime_type": mime_type,
        "offset": offset,
        "bytes_read": buf.len(),
        "total_bytes": total_bytes,
        "eof": eof,
        "next_offset": (!eof).then_some(next),
        "data_base64": BASE64.encode(&buf),
    }))
}

/// Removes managed files on disk for `records` and returns `(ids_to_delete, deleted_files, freed_bytes, items)`.
pub(crate) async fn remove_managed_download_files(
    records: &[DownloadRecord],
) -> Result<(Vec<i64>, usize, u64, Vec<Value>)> {
    let mut ids_to_delete = Vec::with_capacity(records.len());
    let mut deleted_files = 0usize;
    let mut freed_bytes = 0u64;
    let mut items = Vec::with_capacity(records.len());

    for rec in records {
        let mut file_deleted = false;
        if let Some(canon) = verified_managed_download_path(&rec.file_path) {
            let disk_size = tokio::fs::metadata(&canon)
                .await
                .map(|m| m.len())
                .unwrap_or(rec.size_bytes);
            match tokio::fs::remove_file(&canon).await {
                Ok(()) => {
                    file_deleted = true;
                    deleted_files += 1;
                    freed_bytes = freed_bytes.saturating_add(disk_size);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("Failed to delete downloaded file {}", canon.display())
                    });
                }
            }
        }
        ids_to_delete.push(rec.id);
        items.push(json!({
            "id": rec.id,
            "asset_id": rec.asset_id,
            "file_name": rec.file_name,
            "file_deleted": file_deleted,
        }));
    }

    Ok((ids_to_delete, deleted_files, freed_bytes, items))
}
