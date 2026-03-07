use futures_core::Stream;
use ucode_core::{Event, EventStream};

// ── LineBuffer ────────────────────────────────────────────────────────────────

/// Accumulates bytes and yields complete lines.
#[derive(Debug, Default)]
pub struct LineBuffer {
    buffer: String,
}

impl LineBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push raw bytes into the buffer.
    pub fn push_bytes(&mut self, bytes: &[u8]) {
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
    }

    /// Extract the next complete line (up to `\n`), if available.
    pub fn next_line(&mut self) -> Option<String> {
        let pos = self.buffer.find('\n')?;
        let line = self.buffer[..pos].to_string();
        self.buffer = self.buffer[pos + 1..].to_string();
        Some(line)
    }

    /// Drain any remaining content as a final line (for stream end).
    ///
    /// Returns `None` if the buffer is empty or whitespace-only.
    pub fn drain(&mut self) -> Option<String> {
        if self.buffer.trim().is_empty() {
            self.buffer.clear();
            None
        } else {
            Some(std::mem::take(&mut self.buffer))
        }
    }
}

// ── stream_lines ──────────────────────────────────────────────────────────────

/// Build a flat `EventStream` from a byte stream using the given line parser.
///
/// This is the standard streaming pattern used by SSE and NDJSON providers.
/// The parser receives each line and a mutable accumulator for cross-line state.
pub fn stream_lines<S, A, F>(byte_stream: S, accumulator: A, parse_line: F) -> EventStream
where
    S: Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + Unpin + 'static,
    A: Send + 'static,
    F: Fn(&str, &mut A) -> Vec<Event> + Send + 'static,
{
    let event_stream = futures_util::stream::unfold(
        (byte_stream, accumulator, LineBuffer::new(), parse_line),
        |(mut byte_stream, mut acc, mut line_buf, parse)| async move {
            use futures_util::StreamExt;
            loop {
                while let Some(line) = line_buf.next_line() {
                    let events = parse(&line, &mut acc);
                    if !events.is_empty() {
                        return Some((events, (byte_stream, acc, line_buf, parse)));
                    }
                }
                match byte_stream.next().await {
                    Some(Ok(bytes)) => line_buf.push_bytes(&bytes),
                    Some(Err(_)) | None => {
                        if let Some(remaining) = line_buf.drain() {
                            let events = parse(&remaining, &mut acc);
                            if !events.is_empty() {
                                return Some((events, (byte_stream, acc, line_buf, parse)));
                            }
                        }
                        return None;
                    }
                }
            }
        },
    );

    let flat = futures_util::stream::StreamExt::flat_map(event_stream, |events| {
        futures_util::stream::iter(events)
    });

    Box::pin(flat) as EventStream
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_line() {
        let mut buf = LineBuffer::new();
        buf.push_bytes(b"hello\n");
        assert_eq!(buf.next_line(), Some("hello".into()));
        assert_eq!(buf.next_line(), None);
    }

    #[test]
    fn multiple_lines() {
        let mut buf = LineBuffer::new();
        buf.push_bytes(b"foo\nbar\nbaz\n");
        assert_eq!(buf.next_line(), Some("foo".into()));
        assert_eq!(buf.next_line(), Some("bar".into()));
        assert_eq!(buf.next_line(), Some("baz".into()));
        assert_eq!(buf.next_line(), None);
    }

    #[test]
    fn partial_then_complete() {
        let mut buf = LineBuffer::new();
        buf.push_bytes(b"hel");
        assert_eq!(buf.next_line(), None);
        buf.push_bytes(b"lo\n");
        assert_eq!(buf.next_line(), Some("hello".into()));
    }

    #[test]
    fn drain_remaining() {
        let mut buf = LineBuffer::new();
        buf.push_bytes(b"leftover");
        assert_eq!(buf.drain(), Some("leftover".into()));
        // Buffer is now empty.
        assert_eq!(buf.drain(), None);
    }

    #[test]
    fn drain_empty() {
        let mut buf = LineBuffer::new();
        assert_eq!(buf.drain(), None);
    }

    #[test]
    fn drain_whitespace_only() {
        let mut buf = LineBuffer::new();
        buf.push_bytes(b"   \n  \t  ");
        // Consume the newline-terminated line first.
        let _ = buf.next_line();
        // Only whitespace remains.
        assert_eq!(buf.drain(), None);
    }
}
