//! `read_image` (T-12, P-11): read an image file within the project root into
//! the conversation as a `ContentBlock::Image`. Mirrors `read_file`'s path
//! confinement (§6.2 read rules) and the same outside-root permission gate
//! (HC-4). Detects format + dimensions header-only via `imagesize`, rejects
//! over `image.max_bytes` (default 5 MiB), and base64-encodes the bytes.
//!
//! On a model without vision support, returns the structured HC-6
//! unsupported-capability result *before* encoding — the model learns it could
//! not see, rather than assuming it saw (P-11). This is data, not a harness
//! error.

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::path::{display_relative, resolve_in_root};
use crate::permission::PermissionRequest;
use crate::tool::{ImageContent, Tool, ToolOutcome, ToolSpec};

#[derive(Deserialize)]
struct ReadImageArgs {
    path: String,
}

/// The `read_image` tool.
pub struct ReadImageTool;

#[async_trait]
impl Tool for ReadImageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_image".into(),
            description: "Read an image file (PNG, JPEG, GIF, WebP) within the project root into \
                          the conversation so the model can see it. The image is sent as a content \
                          block; no pixels are rendered in the terminal. Respects the same \
                          project-read rules as read_file."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Image file path, relative to the project root or absolute."
                    }
                },
                "required": ["path"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        serde_json::from_value::<ReadImageArgs>(args.clone())
            .ok()
            .map(|a| format!("read image {}", a.path))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: ReadImageArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::invalid_args(&self.spec().name, &e),
        };

        // Vision gate (HC-6): before encoding, tell the model it cannot see on
        // a non-vision model — data, not a harness error (P-11).
        if !ctx.vision() {
            return ToolOutcome::failure(
                "This model has no vision support; the image was not sent. Switch to a \
                 vision-capable model or describe the image.",
                "no vision",
            );
        }

        let resolved = match resolve_in_root(ctx.project_root(), &args.path) {
            Ok(r) => r,
            Err(e) => return ToolOutcome::failure(e, "path error"),
        };

        // HC-4: reading outside the project root requires explicit approval —
        // same §6.2 read rules as read_file.
        if resolved.outside_root {
            let request = PermissionRequest {
                tool: "read_image".into(),
                summary: format!("read image {} (outside project root)", args.path),
                detail: format!(
                    "Read an image OUTSIDE the project root:\n{}",
                    resolved.path.display()
                ),
                affected_paths: vec![resolved.path.clone()],
                outside_root: true,
            };
            if !ctx.authorize(request).await.is_allowed() {
                return ToolOutcome::denied(&format!("read image {}", args.path));
            }
        }

        // Read raw bytes — images are not UTF-8.
        let bytes = match tokio::fs::read(&resolved.path).await {
            Ok(b) => b,
            Err(e) => {
                return ToolOutcome::failure(
                    format!("cannot read {}: {e}", args.path),
                    "read failed",
                );
            }
        };

        // Enforce the size cap before encoding (HC-6 precise failure).
        let max = ctx.image_max_bytes();
        if bytes.len() > max {
            return ToolOutcome::failure(
                format!(
                    "{} is {} bytes — exceeds the image size limit of {} bytes",
                    args.path,
                    bytes.len(),
                    max
                ),
                "oversize",
            );
        }

        // Detect format + dimensions header-only (no codec tree).
        let dim = match imagesize::blob_size(&bytes) {
            Ok(d) => d,
            Err(_) => {
                return ToolOutcome::failure(
                    format!(
                        "{} is not a recognized image format (accepted: PNG, JPEG, GIF, WebP)",
                        args.path
                    ),
                    "format unknown",
                );
            }
        };
        let img_type = match imagesize::image_type(&bytes) {
            Ok(t) => t,
            Err(_) => {
                return ToolOutcome::failure(
                    format!(
                        "{} is not a recognized image format (accepted: PNG, JPEG, GIF, WebP)",
                        args.path
                    ),
                    "format unknown",
                );
            }
        };
        let media_type = match media_type_for_type(img_type) {
            Some(mt) => mt,
            None => {
                return ToolOutcome::failure(
                    format!(
                        "{} is not a supported image format (accepted: PNG, JPEG, GIF, WebP)",
                        args.path
                    ),
                    "format unsupported",
                );
            }
        };
        let width = dim.width;
        let height = dim.height;

        let rel = display_relative(ctx.project_root(), &resolved.path);
        let format_name = format_label(media_type);

        // Base64-encode and hand the image block to the engine.
        let data = general_purpose::STANDARD.encode(&bytes);
        let summary = format!("read image {rel} · {width}×{height} · {format_name}");
        ToolOutcome::success(
            format!(
                "Image loaded: {rel} ({width}×{height}, {format_name}). It has been added to the conversation as an image content block."
            ),
            summary,
        )
        .with_image(ImageContent {
            media_type: media_type.to_string(),
            data,
        })
    }
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

/// A short uppercase label for the reference line (Design §4.8).
fn format_label(media_type: &str) -> &'static str {
    match media_type {
        "image/png" => "PNG",
        "image/jpeg" => "JPEG",
        "image/gif" => "GIF",
        "image/webp" => "WebP",
        _ => "IMAGE",
    }
}
