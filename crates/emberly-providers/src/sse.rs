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

/// Cap on a single unterminated frame. A frame is only decoded once its
/// terminating blank line arrives, so until then its bytes accumulate — and a
/// peer that never sends that blank line would grow the buffer until the process
/// is killed, losing the session (HC-3). The #11 idle timeout cannot catch this
/// case: bytes keep arriving, so the stream never looks stalled. 8 MiB is far
/// above any real SSE frame (the largest are long reasoning blocks, orders of
/// magnitude smaller) and far below a memory problem.
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// The longest frame separator (`\r\n\r\n`), so an incremental scan knows how
/// many trailing bytes to re-examine in case a separator straddles two chunks.
const MAX_SEP_LEN: usize = 4;

/// Incremental SSE parser. Feed chunks with [`push`](SseParser::push); call
/// [`finish`](SseParser::finish) at end of stream to flush a trailing frame
/// that lacks a final blank line.
#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    /// How much of `buf` has already been searched for a terminator, so a chunk
    /// that completes no frame does not re-scan the whole buffer.
    ///
    /// Ordinary streaming never notices — frames are small and the buffer drains
    /// every chunk — but one large frame arriving in many chunks is O(n²) without
    /// this: a 4 MiB frame in 1 KiB chunks measured **205s** of pure scanning,
    /// against 0.3s with the offset. That is a hang, and a peer can choose to
    /// cause it, so the bound is not an optimization.
    scanned: usize,
    /// Set once a frame exceeded [`MAX_FRAME_BYTES`]; the buffer is dropped and
    /// the caller must end the stream.
    overflowed: bool,
}

impl SseParser {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of bytes; return every complete event now available.
    ///
    /// Check [`overflowed`](SseParser::overflowed) afterwards: a frame past the
    /// size cap is reported there rather than as an error return, so the
    /// complete events in this chunk are still delivered before the stream ends.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        if self.overflowed {
            return Vec::new();
        }
        self.buf.extend_from_slice(chunk);
        let mut events = Vec::new();
        // Resume scanning just behind the previous end: a separator can straddle
        // the boundary between two chunks.
        let mut from = self.scanned.saturating_sub(MAX_SEP_LEN - 1);
        while let Some((offset, sep_len)) = find_frame_boundary(&self.buf[from..]) {
            let end = from + offset;
            let frame: Vec<u8> = self.buf.drain(..end + sep_len).collect();
            if let Some(event) = parse_frame(&String::from_utf8_lossy(&frame[..end])) {
                events.push(event);
            }
            from = 0;
        }
        self.scanned = self.buf.len();
        if self.buf.len() > MAX_FRAME_BYTES {
            self.buf = Vec::new();
            self.scanned = 0;
            self.overflowed = true;
        }
        events
    }

    /// Whether a frame exceeded [`MAX_FRAME_BYTES`]. Once true the parser is
    /// spent: it holds no buffer and ignores further chunks.
    #[must_use]
    pub fn overflowed(&self) -> bool {
        self.overflowed
    }

    /// Flush any buffered bytes as a final frame (for servers that omit the
    /// trailing blank line). Returns at most one event.
    pub fn finish(&mut self) -> Option<SseEvent> {
        if self.buf.is_empty() {
            return None;
        }
        self.scanned = 0;
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

    #[test]
    fn an_unterminated_frame_stops_at_the_size_cap() {
        // A peer that never sends the terminating blank line must not be able to
        // grow the buffer without limit — the process would be killed and the
        // session lost (HC-3). The #11 idle timeout cannot catch this: bytes keep
        // arriving, so the stream never looks stalled.
        let mut p = SseParser::new();
        let chunk = vec![b'x'; 1024 * 1024];
        let mut pushes = 0;
        while !p.overflowed() {
            assert!(p.push(&chunk).is_empty());
            pushes += 1;
            assert!(pushes < 64, "the cap must engage well before this");
        }
        assert!(p.overflowed());
        // Spent: the buffer is released and later chunks are ignored, so a
        // still-open connection cannot keep feeding it.
        assert!(p.push(b"data: more\n\n").is_empty());
    }

    #[test]
    fn complete_events_before_an_overflow_are_still_delivered() {
        let mut p = SseParser::new();
        let mut chunk = b"data: first\n\n".to_vec();
        chunk.extend(std::iter::repeat_n(b'x', MAX_FRAME_BYTES + 1));
        let events = p.push(&chunk);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "first");
        assert!(p.overflowed());
    }

    #[test]
    fn a_separator_split_across_chunks_is_still_found() {
        // The incremental scan resumes just behind the previous end, so a
        // terminator straddling two chunks is not missed.
        for (a, b) in [
            (&b"data: x\n"[..], &b"\n"[..]),
            (&b"data: x\r\n"[..], &b"\r\n"[..]),
            (&b"data: x\r"[..], &b"\n\r\n"[..]),
            (&b"data: x\r\n\r"[..], &b"\n"[..]),
        ] {
            let mut p = SseParser::new();
            assert!(p.push(a).is_empty(), "{a:?} should not complete a frame");
            let events = p.push(b);
            assert_eq!(events.len(), 1, "split {a:?} | {b:?} lost its frame");
            assert_eq!(events[0].data, "x");
        }
    }

    #[test]
    fn many_chunks_of_one_frame_do_not_rescan_from_the_start() {
        // Guards the incremental scan. Measured with the offset removed, this
        // exact case took 205 seconds — every push re-searching the whole buffer
        // is ~8 GiB of byte comparisons. With the offset it is linear and the
        // whole suite runs in under a second.
        let mut p = SseParser::new();
        assert!(p.push(b"data: ").is_empty());
        let chunk = vec![b'x'; 1024];
        for _ in 0..4096 {
            assert!(p.push(&chunk).is_empty());
        }
        assert!(!p.overflowed());
        let events = p.push(b"\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data.len(), 4096 * 1024);
    }
}
