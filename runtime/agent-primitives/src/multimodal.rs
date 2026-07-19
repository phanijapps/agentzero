//! Multimodal content helpers — base64 flush, MIME utilities, source resolution.

use std::path::Path;

use base64::Engine;
use sha2::{Digest, Sha256};

use crate::types::{ContentSource, Part};

/// Map a MIME type string to a file extension.
pub fn mime_to_extension(mime_type: &str) -> &str {
    match mime_type {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        "application/pdf" => "pdf",
        "text/csv" => "csv",
        "text/plain" => "txt",
        "text/html" => "html",
        "application/json" => "json",
        _ => "bin",
    }
}

/// Convert a single `Part` with a `Base64` source to a `FileRef` by writing
/// the decoded bytes to a content-addressed file under `attachments_dir`.
///
/// Non-multimodal parts and parts with `Url` / `FileRef` sources pass through
/// unchanged.
pub fn flush_part_to_disk(part: Part, attachments_dir: &Path) -> std::io::Result<Part> {
    match part {
        Part::Image {
            source: ContentSource::Base64(ref data),
            ref mime_type,
            ref detail,
        } => {
            let path = write_content_addressed(data, mime_type, attachments_dir)?;
            Ok(Part::Image {
                source: ContentSource::FileRef(path),
                mime_type: mime_type.clone(),
                detail: detail.clone(),
            })
        }
        Part::File {
            source: ContentSource::Base64(ref data),
            ref mime_type,
            ref filename,
        } => {
            let path = write_content_addressed(data, mime_type, attachments_dir)?;
            Ok(Part::File {
                source: ContentSource::FileRef(path),
                mime_type: mime_type.clone(),
                filename: filename.clone(),
            })
        }
        other => Ok(other),
    }
}

/// Batch version of [`flush_part_to_disk`].
pub fn flush_parts_to_disk(parts: Vec<Part>, attachments_dir: &Path) -> std::io::Result<Vec<Part>> {
    parts
        .into_iter()
        .map(|p| flush_part_to_disk(p, attachments_dir))
        .collect()
}

/// Resolve a `FileRef` source back to `Base64` by reading the file from disk.
///
/// `Base64` and `Url` sources pass through unchanged.
pub fn rehydrate_source(source: &ContentSource) -> std::io::Result<ContentSource> {
    match source {
        ContentSource::FileRef(path) => {
            let bytes = std::fs::read(path)?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
            Ok(ContentSource::Base64(encoded))
        }
        other => Ok(other.clone()),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Decode base64, compute SHA-256, write to `{dir}/{hash}.{ext}` if absent.
/// Returns the absolute path as a `String`.
fn write_content_addressed(
    base64_data: &str,
    mime_type: &str,
    attachments_dir: &Path,
) -> std::io::Result<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let hash = format!("{:x}", hasher.finalize());

    let ext = mime_to_extension(mime_type);
    let filename = format!("{}.{}", hash, ext);
    let file_path = attachments_dir.join(&filename);

    if !file_path.exists() {
        std::fs::create_dir_all(attachments_dir)?;
        std::fs::write(&file_path, &bytes)?;
    }

    Ok(file_path.to_string_lossy().to_string())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ContentSource, ImageDetail};
    use base64::Engine;
    use std::fs;

    #[test]
    fn test_mime_to_extension() {
        assert_eq!(mime_to_extension("image/png"), "png");
        assert_eq!(mime_to_extension("image/jpeg"), "jpg");
        assert_eq!(mime_to_extension("image/webp"), "webp");
        assert_eq!(mime_to_extension("image/gif"), "gif");
        assert_eq!(mime_to_extension("application/pdf"), "pdf");
        assert_eq!(mime_to_extension("text/csv"), "csv");
        assert_eq!(mime_to_extension("application/octet-stream"), "bin");
    }

    #[test]
    fn test_flush_text_part_unchanged() {
        let part = Part::text("hello");
        let dir = tempfile::tempdir().unwrap();
        let result = flush_part_to_disk(part, dir.path()).unwrap();
        assert!(matches!(result, Part::Text { .. }));
    }

    #[test]
    fn test_flush_image_base64_to_fileref() {
        let dir = tempfile::tempdir().unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(b"fake png data");
        let part = Part::Image {
            source: ContentSource::Base64(data),
            mime_type: "image/png".to_string(),
            detail: Some(ImageDetail::Auto),
        };
        let flushed = flush_part_to_disk(part, dir.path()).unwrap();
        match &flushed {
            Part::Image {
                source: ContentSource::FileRef(path),
                mime_type,
                detail,
            } => {
                assert!(std::path::Path::new(path).exists());
                assert_eq!(mime_type, "image/png");
                assert!(matches!(detail, Some(ImageDetail::Auto)));
                let bytes = fs::read(path).unwrap();
                assert_eq!(bytes, b"fake png data");
            }
            _ => panic!("Expected Image with FileRef, got {:?}", flushed),
        }
    }

    #[test]
    fn test_flush_deduplicates_by_hash() {
        let dir = tempfile::tempdir().unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(b"same content");
        let part1 = Part::Image {
            source: ContentSource::Base64(data.clone()),
            mime_type: "image/png".to_string(),
            detail: None,
        };
        let part2 = Part::Image {
            source: ContentSource::Base64(data),
            mime_type: "image/png".to_string(),
            detail: None,
        };
        let flushed1 = flush_part_to_disk(part1, dir.path()).unwrap();
        let flushed2 = flush_part_to_disk(part2, dir.path()).unwrap();
        match (&flushed1, &flushed2) {
            (
                Part::Image {
                    source: ContentSource::FileRef(p1),
                    ..
                },
                Part::Image {
                    source: ContentSource::FileRef(p2),
                    ..
                },
            ) => {
                assert_eq!(p1, p2);
            }
            _ => panic!("Expected FileRef parts"),
        }
    }

    #[test]
    fn test_flush_url_source_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let part = Part::Image {
            source: ContentSource::Url("https://example.com/img.png".to_string()),
            mime_type: "image/png".to_string(),
            detail: None,
        };
        let flushed = flush_part_to_disk(part, dir.path()).unwrap();
        match &flushed {
            Part::Image {
                source: ContentSource::Url(url),
                ..
            } => {
                assert_eq!(url, "https://example.com/img.png");
            }
            _ => panic!("URL source should be unchanged"),
        }
    }

    #[test]
    fn test_flush_file_base64_to_fileref() {
        let dir = tempfile::tempdir().unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(b"fake pdf");
        let part = Part::File {
            source: ContentSource::Base64(data),
            mime_type: "application/pdf".to_string(),
            filename: Some("report.pdf".to_string()),
        };
        let flushed = flush_part_to_disk(part, dir.path()).unwrap();
        match &flushed {
            Part::File {
                source: ContentSource::FileRef(path),
                filename,
                ..
            } => {
                assert!(std::path::Path::new(path).exists());
                assert_eq!(filename.as_deref(), Some("report.pdf"));
            }
            _ => panic!("Expected File with FileRef"),
        }
    }

    #[test]
    fn test_rehydrate_base64_unchanged() {
        let source = ContentSource::Base64("aGVsbG8=".to_string());
        let result = rehydrate_source(&source).unwrap();
        assert!(matches!(result, ContentSource::Base64(ref s) if s == "aGVsbG8="));
    }

    #[test]
    fn test_rehydrate_url_unchanged() {
        let source = ContentSource::Url("https://example.com/img.png".to_string());
        let result = rehydrate_source(&source).unwrap();
        assert!(matches!(result, ContentSource::Url(_)));
    }

    #[test]
    fn test_rehydrate_fileref_to_base64() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.png");
        fs::write(&file_path, b"image bytes").unwrap();
        let source = ContentSource::FileRef(file_path.to_str().unwrap().to_string());
        let result = rehydrate_source(&source).unwrap();
        match result {
            ContentSource::Base64(data) => {
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(&data)
                    .unwrap();
                assert_eq!(decoded, b"image bytes");
            }
            _ => panic!("Expected Base64 from rehydrated FileRef"),
        }
    }

    #[test]
    fn test_flush_parts_batch() {
        let dir = tempfile::tempdir().unwrap();
        let data = base64::engine::general_purpose::STANDARD.encode(b"data");
        let parts = vec![
            Part::text("hello"),
            Part::Image {
                source: ContentSource::Base64(data),
                mime_type: "image/png".to_string(),
                detail: None,
            },
        ];
        let flushed = flush_parts_to_disk(parts, dir.path()).unwrap();
        assert_eq!(flushed.len(), 2);
        assert!(matches!(flushed[0], Part::Text { .. }));
        assert!(matches!(
            flushed[1],
            Part::Image {
                source: ContentSource::FileRef(_),
                ..
            }
        ));
    }
}
