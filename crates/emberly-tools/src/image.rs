//! Shared image validation + encoding (T-12, FR-10): turn raw file bytes into
//! a validated, base64-encoded [`ImageContent`] plus the display metadata a
//! tool-activity line or an attachment chip needs. One place so the
//! `read_image` tool (model-initiated) and the engine's user-attach path
//! (FR-10, user-initiated) enforce the identical format/size rules — reuse
//! over reinvention, since both ultimately produce the same content block
//! (Design §4.14, Tech Spec §4.1).

use base64::{engine::general_purpose, Engine as _};

use crate::tool::ImageContent;

/// A successfully validated and encoded image.
pub struct EncodedImage {
    pub content: ImageContent,
    pub width: usize,
    pub height: usize,
    /// A short uppercase label for a reference line or chip (Design §4.8/§4.14).
    pub format_label: &'static str,
}

/// Validate `bytes` against `max_bytes` and a supported format (PNG, JPEG,
/// GIF, WebP), returning a base64-encoded [`ImageContent`] plus display
/// metadata. `Err` names precisely what's wrong (HC-6 — data, not a crash).
pub fn encode_image_bytes(bytes: &[u8], max_bytes: usize) -> Result<EncodedImage, String> {
    if bytes.len() > max_bytes {
        return Err(format!(
            "{} bytes — exceeds the image size limit of {max_bytes} bytes",
            bytes.len()
        ));
    }

    let dim = imagesize::blob_size(bytes).map_err(|_| {
        "not a recognized image format (accepted: PNG, JPEG, GIF, WebP)".to_string()
    })?;
    let img_type = imagesize::image_type(bytes).map_err(|_| {
        "not a recognized image format (accepted: PNG, JPEG, GIF, WebP)".to_string()
    })?;
    let media_type = media_type_for_type(img_type).ok_or_else(|| {
        "not a supported image format (accepted: PNG, JPEG, GIF, WebP)".to_string()
    })?;

    let data = general_purpose::STANDARD.encode(bytes);
    Ok(EncodedImage {
        content: ImageContent {
            media_type: media_type.to_string(),
            data,
        },
        width: dim.width,
        height: dim.height,
        format_label: format_label(media_type),
    })
}

/// Map an `imagesize::ImageType` to its MIME type, or `None` if unsupported.
fn media_type_for_type(ty: imagesize::ImageType) -> Option<&'static str> {
    match ty {
        imagesize::ImageType::Png => Some("image/png"),
        imagesize::ImageType::Jpeg => Some("image/jpeg"),
        imagesize::ImageType::Gif => Some("image/gif"),
        imagesize::ImageType::Webp => Some("image/webp"),
        _ => None,
    }
}

/// A short uppercase label for the reference line (Design §4.8) or an
/// attachment chip (Design §4.14).
fn format_label(media_type: &str) -> &'static str {
    match media_type {
        "image/png" => "PNG",
        "image/jpeg" => "JPEG",
        "image/gif" => "GIF",
        "image/webp" => "WebP",
        _ => "IMAGE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 1x1 transparent PNG.
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn encodes_a_valid_png() {
        let encoded = match encode_image_bytes(TINY_PNG, 1_000_000) {
            Ok(e) => e,
            Err(e) => panic!("expected success, got: {e}"),
        };
        assert_eq!(encoded.width, 1);
        assert_eq!(encoded.height, 1);
        assert_eq!(encoded.format_label, "PNG");
        assert_eq!(encoded.content.media_type, "image/png");
    }

    #[test]
    fn rejects_oversize_bytes() {
        let err = match encode_image_bytes(TINY_PNG, 10) {
            Ok(_) => panic!("expected a size-cap failure"),
            Err(e) => e,
        };
        assert!(err.contains("exceeds the image size limit"), "{err}");
    }

    #[test]
    fn rejects_unrecognized_bytes() {
        let err = match encode_image_bytes(b"not an image", 1_000_000) {
            Ok(_) => panic!("expected a format failure"),
            Err(e) => e,
        };
        assert!(err.contains("not a recognized image format"), "{err}");
    }
}
