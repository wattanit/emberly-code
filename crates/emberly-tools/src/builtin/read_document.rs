//! `read_document` (T-16, P-12): read a PDF file within the project root into
//! the conversation as a `ContentBlock::Document`. Mirrors `read_image`'s path
//! confinement (§6.2 read rules) and the same outside-root permission gate
//! (HC-4). Sniffs the format by a leading `%PDF-` magic prefix — no parser, no
//! page count (HC-2) — rejects over `document.max_bytes` (default 32 MiB), and
//! base64-encodes the bytes.
//!
//! On a model without document support, returns the structured HC-6
//! unsupported-capability result *before* encoding — the model learns it
//! could not read the document, rather than assuming it did (P-12). This is
//! data, not a harness error.

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::ctx::ToolCtx;
use crate::path::{display_relative, resolve_in_root};
use crate::permission::PermissionRequest;
use crate::tool::{DocumentContent, Tool, ToolOutcome, ToolSpec};

/// The `%PDF-` magic prefix every PDF file begins with.
const PDF_MAGIC: &[u8] = b"%PDF-";

#[derive(Deserialize)]
struct ReadDocumentArgs {
    path: String,
}

/// The `read_document` tool.
pub struct ReadDocumentTool;

#[async_trait]
impl Tool for ReadDocumentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_document".into(),
            description: "Read a PDF file within the project root into the conversation so the \
                          model can read it. The document is sent as a content block; the harness \
                          never parses it — no page count or extracted text. Respects the same \
                          project-read rules as read_file."
                .into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "PDF file path, relative to the project root or absolute."
                    }
                },
                "required": ["path"]
            }),
        }
    }

    fn describe(&self, args: &Value) -> Option<String> {
        serde_json::from_value::<ReadDocumentArgs>(args.clone())
            .ok()
            .map(|a| format!("read document {}", a.path))
    }

    async fn execute(&self, args: Value, ctx: &ToolCtx) -> ToolOutcome {
        let args: ReadDocumentArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutcome::failure(format!("invalid arguments: {e}"), "bad args"),
        };

        // Document gate (HC-6): before encoding, tell the model it cannot read
        // documents on a non-document model — data, not a harness error (P-12).
        if !ctx.documents() {
            return ToolOutcome::failure(
                "This model has no document support; the document was not sent. Switch to a \
                 document-capable model or summarize the document another way.",
                "no document support",
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
                tool: "read_document".into(),
                summary: format!("read document {} (outside project root)", args.path),
                detail: format!(
                    "Read a document OUTSIDE the project root:\n{}",
                    resolved.path.display()
                ),
                affected_paths: vec![resolved.path.clone()],
                outside_root: true,
            };
            if !ctx.authorize(request).await.is_allowed() {
                return ToolOutcome::denied(&format!("read document {}", args.path));
            }
        }

        // Read raw bytes — PDFs are not UTF-8.
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
        let max = ctx.document_max_bytes();
        if bytes.len() > max {
            return ToolOutcome::failure(
                format!(
                    "{} is {} bytes — exceeds the document size limit of {} bytes",
                    args.path,
                    bytes.len(),
                    max
                ),
                "oversize",
            );
        }

        // Sniff the type by magic prefix — no parser, no page count (HC-2).
        if !bytes.starts_with(PDF_MAGIC) {
            return ToolOutcome::failure(
                format!(
                    "{} is not a recognized PDF file (expected a %PDF- header)",
                    args.path
                ),
                "format unknown",
            );
        }

        let rel = display_relative(ctx.project_root(), &resolved.path);
        let size = bytes.len();

        // Base64-encode and hand the document block to the engine.
        let data = general_purpose::STANDARD.encode(&bytes);
        let summary = format!("read document {rel} · {} · PDF", format_size(size));
        ToolOutcome::success(
            format!(
                "Document loaded: {rel} ({}, PDF). It has been added to the conversation as a \
                 document content block.",
                format_size(size)
            ),
            summary,
        )
        .with_document(DocumentContent {
            media_type: "application/pdf".to_string(),
            data,
        })
    }
}

/// A human-readable size for the reference line (Design §4.11): KB below 1
/// MiB, MB at and above it, one decimal place.
fn format_size(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= MIB {
        format!("{:.1} MB", bytes / MIB)
    } else {
        format!("{:.1} KB", bytes / KIB)
    }
}
