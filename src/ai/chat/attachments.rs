//! Helpers for turning uploaded image references into model-ready content.
//!
//! Uploaded images live in the session workspace as `/files/{name}`. The
//! transcript stores those references (so the database stays small); before a
//! request is sent, [`hydrate_images`] reads the files and replaces each
//! reference with a base64 `data:` URL the model can consume.

use std::io::Cursor;
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use image::{ImageFormat, ImageReader, imageops::FilterType};

use crate::openai::{ContentPart, Message, MessageContent, Role};

/// Maximum width/height (in pixels) an image is scaled down to before being
/// sent to the model. Images already within this bound are sent unchanged.
///
/// 1568 is the long-edge cap providers recommend for vision input; sending
/// larger images costs more tokens and latency without improving recognition.
pub const MAX_IMAGE_DIMENSION: u32 = 1568;

/// Build a user message from text and attachment filenames. Images are stored
/// as workspace-relative references (`/files/{name}`) that
/// [`hydrate_images`] expands later.
pub fn user_message_with_attachments(text: &str, filenames: &[String]) -> Message {
    if filenames.is_empty() {
        return Message::new(Role::User, text);
    }

    let mut parts: Vec<ContentPart> = Vec::with_capacity(filenames.len() + 1);
    if !text.is_empty() {
        parts.push(ContentPart::text(text));
    }
    for filename in filenames {
        parts.push(ContentPart::image(&format!("/files/{filename}")));
    }

    Message::new_with_parts(Role::User, parts)
}

/// Replace workspace-relative image references in `messages` with base64
/// `data:` URLs. Missing or unreadable images are dropped with a warning so a
/// deleted upload doesn't break the conversation.
pub async fn hydrate_images(workspace: Option<&Path>, messages: &mut [Message]) {
    let Some(workspace) = workspace else {
        return;
    };

    for message in messages.iter_mut() {
        let Some(MessageContent::Parts(parts)) = message.content.as_mut() else {
            continue;
        };

        let mut hydrated: Vec<ContentPart> = Vec::with_capacity(parts.len());
        for part in parts.drain(..) {
            match part {
                ContentPart::ImageUrl { image_url } if !image_url.url.starts_with("data:") => {
                    match load_image_part(workspace, &image_url.url).await {
                        Ok(url) => hydrated.push(ContentPart::image(&url)),
                        Err(e) => {
                            tracing::warn!("Dropping image attachment {}: {}", image_url.url, e)
                        }
                    }
                }
                other => hydrated.push(other),
            }
        }
        *parts = hydrated;
    }
}

/// Read an image from the workspace and return it as a `data:` URL.
async fn load_image_part(workspace: &Path, reference: &str) -> Result<String> {
    let path = resolve_workspace_path(workspace, reference)?;
    let bytes = tokio::fs::read(&path).await?;
    let mime = mime_for_path(&path);
    let (mime, bytes) = downscale(&bytes, mime)?;
    Ok(format!("data:{};base64,{}", mime, STANDARD.encode(&bytes)))
}

/// Resolve a `/files/...` reference against the workspace, rejecting absolute
/// paths and parent-directory components so a reference can't escape it.
fn resolve_workspace_path(workspace: &Path, reference: &str) -> Result<PathBuf> {
    let relative = reference.trim_start_matches('/');
    let relative = Path::new(relative);
    let unsafe_component = relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    });
    if relative.as_os_str().is_empty() || relative.is_absolute() || unsafe_component {
        return Err(anyhow!("invalid image reference: {reference}"));
    }
    Ok(workspace.join(relative))
}

fn mime_for_path(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    match extension.as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Scale an image down so neither side exceeds [`MAX_IMAGE_DIMENSION`]. Images
/// already within the bound are returned unchanged.
fn downscale(bytes: &[u8], mime: &'static str) -> Result<(&'static str, Vec<u8>)> {
    let format = image::guess_format(bytes)?;

    // Read only the header to check dimensions; a full decode is only needed
    // when the image actually has to be resized.
    let (width, height) = ImageReader::with_format(Cursor::new(bytes), format).into_dimensions()?;

    if width <= MAX_IMAGE_DIMENSION && height <= MAX_IMAGE_DIMENSION {
        return Ok((mime, bytes.to_vec()));
    }

    let image = image::load_from_memory_with_format(bytes, format)?;
    let resized = image.resize(
        MAX_IMAGE_DIMENSION,
        MAX_IMAGE_DIMENSION,
        FilterType::Lanczos3,
    );

    // Keep JPEGs as JPEG (smaller for photos); everything else becomes PNG,
    // which is lossless and available for all accepted input formats.
    let (out_mime, out_format) = if format == ImageFormat::Jpeg {
        ("image/jpeg", ImageFormat::Jpeg)
    } else {
        ("image/png", ImageFormat::Png)
    };

    let mut out = Vec::new();
    resized.write_to(&mut Cursor::new(&mut out), out_format)?;
    Ok((out_mime, out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openai::{ContentPart, MessageContent};
    use image::{Rgb, RgbImage};
    use tempfile::TempDir;

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = RgbImage::from_pixel(width, height, Rgb([10, 20, 30]));
        let mut out = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn user_message_has_text_and_image_parts() {
        let msg = user_message_with_attachments("look", &["a.png".to_string()]);
        let MessageContent::Parts(parts) = msg.content.as_ref().unwrap() else {
            panic!("expected parts");
        };
        assert_eq!(parts.len(), 2);
        assert_eq!(msg.text(), Some("look"));
    }

    #[test]
    fn user_message_without_attachments_is_text() {
        let msg = user_message_with_attachments("hello", &[]);
        assert!(matches!(msg.content, Some(MessageContent::Text(_))));
    }

    #[tokio::test]
    async fn hydrates_reference_into_data_url() {
        let temp = TempDir::new().unwrap();
        let files = temp.path().join("files");
        tokio::fs::create_dir_all(&files).await.unwrap();
        tokio::fs::write(files.join("a.png"), png_bytes(4, 4))
            .await
            .unwrap();

        let mut messages = vec![user_message_with_attachments("hi", &["a.png".to_string()])];
        hydrate_images(Some(temp.path()), &mut messages).await;

        let MessageContent::Parts(parts) = messages[0].content.as_ref().unwrap() else {
            panic!("expected parts");
        };
        let ContentPart::ImageUrl { image_url } = &parts[1] else {
            panic!("expected image part");
        };
        assert!(image_url.url.starts_with("data:image/png;base64,"));
    }

    #[tokio::test]
    async fn drops_missing_image() {
        let temp = TempDir::new().unwrap();
        let mut messages = vec![user_message_with_attachments(
            "hi",
            &["gone.png".to_string()],
        )];
        hydrate_images(Some(temp.path()), &mut messages).await;

        let MessageContent::Parts(parts) = messages[0].content.as_ref().unwrap() else {
            panic!("expected parts");
        };
        assert_eq!(parts.len(), 1);
        assert_eq!(messages[0].text(), Some("hi"));
    }

    #[test]
    fn rejects_traversal_reference() {
        let workspace = Path::new("/tmp/ws");
        assert!(resolve_workspace_path(workspace, "/../etc/passwd").is_err());
        assert!(resolve_workspace_path(workspace, "/files/../../etc/passwd").is_err());
        assert!(resolve_workspace_path(workspace, "/files/a.png").is_ok());
    }

    #[test]
    fn downscales_oversized_image() {
        // One pixel over the bound forces scaling on the tall side while
        // staying cheap to encode.
        let bytes = png_bytes(1, MAX_IMAGE_DIMENSION + 1);
        let (mime, out) = downscale(&bytes, "image/png").unwrap();
        assert_eq!(mime, "image/png");
        let resized = image::load_from_memory(&out).unwrap();
        assert_eq!(resized.height(), MAX_IMAGE_DIMENSION);
    }

    #[test]
    fn leaves_small_image_untouched() {
        let bytes = png_bytes(2, 2);
        let (mime, out) = downscale(&bytes, "image/png").unwrap();
        assert_eq!(mime, "image/png");
        assert_eq!(out, bytes);
    }
}
