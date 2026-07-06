//! A first-party Server-Sent Events parser (Tech Spec §4.3, §16 — first-party
//! over `eventsource-stream`). It is a few dozen lines, dependency-free, and
//! gives us exact control over frame boundaries and the `[DONE]` sentinel.
//!
//! It buffers raw **bytes** (not `str`) so a multibyte UTF-8 code point split
//! across two network chunks is never decoded mid-character: a frame is only
//! decoded once its terminating blank line has arrived, at which point all of
//! its bytes are present.

/// One dispatched SSE event: an optional `event:` name (Anthropic uses these;
/// OpenAI does not) and the concatenated `data:` payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Incremental SSE parser. Feed chunks with [`push`](SseParser::push); call
/// [`finish`](SseParser::finish) at end of stream to flush a trailing frame
/// that lacks a final blank line.
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
}

impl SseParser {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of bytes; return every complete event now available.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.buf.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some((end, sep_len)) = find_frame_boundary(&self.buf) {
            let frame: Vec<u8> = self.buf.drain(..end + sep_len).collect();
            if let Some(event) = parse_frame(&String::from_utf8_lossy(&frame[..end])) {
                events.push(event);
            }
        }
        events
    }

    /// Flush any buffered bytes as a final frame (for servers that omit the
    /// trailing blank line). Returns at most one event.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if self.buf.is_empty() {
            return None;
        }
        let frame = std::mem::take(&mut self.buf);
        parse_frame(&String::from_utf8_lossy(&frame))
    }
}

/// Find the first frame terminator (`\n\n` or `\r\n\r\n`), returning the index
/// where the frame content ends and the length of the separator.
fn find_frame_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = find_subslice(buf, b"\n\n").map(|i| (i, 2));
    let crlf = find_subslice(buf, b"\r\n\r\n").map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Parse one frame's text into an [`SseEvent`], following the SSE field rules:
/// comment lines (`:`) are ignored, `data:` lines are joined with `\n`, and a
/// single leading space after the colon is stripped. Returns `None` if the
/// frame carries no `data`.
fn parse_frame(text: &str) -> Option<SseEvent> {
    let mut event: Option<String> = None;
    let mut data = String::new();
    let mut has_data = false;

    for raw_line in text.split('\n') {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""), // a field name with no colon
        };
        match field {
            "data" => {
                if has_data {
                    data.push('\n');
                }
                data.push_str(value);
                has_data = true;
            }
            "event" => event = Some(value.to_string()),
            _ => {} // id, retry, unknown fields ignored
        }
    }

    has_data.then_some(SseEvent { event, data })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_event() {
        let mut p = SseParser::new();
        let events = p.push(b"data: hello\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "hello");
        assert_eq!(events[0].event, None);
    }

    #[test]
    fn parses_event_name_and_multiline_data() {
        let mut p = SseParser::new();
        let events = p.push(b"event: content_block_delta\ndata: {\"a\":1}\ndata: more\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.as_deref(), Some("content_block_delta"));
        assert_eq!(events[0].data, "{\"a\":1}\nmore");
    }

    #[test]
    fn ignores_comments_and_blank_frames() {
        let mut p = SseParser::new();
        let events = p.push(b": this is a comment\n\ndata: real\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "real");
    }

    #[test]
    fn handles_frames_split_across_chunks() {
        let mut p = SseParser::new();
        assert!(p.push(b"data: par").is_empty());
        assert!(p.push(b"tial ").is_empty());
        let events = p.push(b"done\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "partial done");
    }

    #[test]
    fn handles_multibyte_split_across_chunks() {
        // "ก" is 0xE0 0xB8 0x81; split it across two chunks.
        let mut p = SseParser::new();
        let mut bytes = b"data: ".to_vec();
        bytes.push(0xE0);
        assert!(p.push(&bytes).is_empty());
        let events = p.push(&[0xB8, 0x81, b'\n', b'\n']);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "ก");
    }

    #[test]
    fn handles_crlf_terminators() {
        let mut p = SseParser::new();
        let events = p.push(b"data: x\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "x");
    }

    #[test]
    fn done_sentinel_is_plain_data() {
        let mut p = SseParser::new();
        let events = p.push(b"data: [DONE]\n\n");
        assert_eq!(events[0].data, "[DONE]");
    }

    #[test]
    fn finish_flushes_trailing_frame() {
        let mut p = SseParser::new();
        assert!(p.push(b"data: tail").is_empty());
        let event = p.finish();
        assert_eq!(event.map(|e| e.data), Some("tail".to_string()));
    }
}
