//! Incremental Server-Sent Events decoder for the bridge's HTTP streams.
//!
//! Fed raw body chunks as they arrive; yields complete events. Follows the
//! WHATWG event-stream rules the bridge needs: `\n`, `\r\n` or `\r` line
//! endings, `:` comments, `event:` names, multi-line `data:` joined by `\n`,
//! one optional space after the colon, and dispatch on a blank line. Lines are
//! split on bytes, so a UTF-8 character cut across chunks is never corrupted.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    /// `event:` field; `None` means the default type (`message`).
    pub event: Option<String>,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
    /// A `\r` ended the previous chunk; a leading `\n` in the next belongs to it.
    pending_cr: bool,
    event: Option<String>,
    data: Option<String>,
}

/// Upper bound on one buffered line, so a peer that never sends a newline
/// cannot grow memory without limit.
const MAX_LINE_BYTES: usize = 64 * 1024 * 1024;

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk; returns every event it completed.
    pub fn feed(&mut self, mut chunk: &[u8]) -> Result<Vec<SseEvent>, String> {
        let mut events = Vec::new();
        if self.pending_cr {
            self.pending_cr = false;
            if chunk.first() == Some(&b'\n') {
                chunk = &chunk[1..];
            }
        }
        let mut start = 0;
        let mut i = 0;
        while i < chunk.len() {
            let b = chunk[i];
            if b == b'\n' || b == b'\r' {
                self.buffer.extend_from_slice(&chunk[start..i]);
                let line = std::mem::take(&mut self.buffer);
                self.process_line(&line, &mut events);
                if b == b'\r' {
                    if i + 1 < chunk.len() {
                        if chunk[i + 1] == b'\n' {
                            i += 1;
                        }
                    } else {
                        self.pending_cr = true;
                    }
                }
                start = i + 1;
            }
            i += 1;
        }
        self.buffer.extend_from_slice(&chunk[start..]);
        if self.buffer.len() > MAX_LINE_BYTES {
            return Err(format!("SSE line exceeds {MAX_LINE_BYTES} bytes"));
        }
        Ok(events)
    }

    fn process_line(&mut self, line: &[u8], events: &mut Vec<SseEvent>) {
        if line.is_empty() {
            if let Some(data) = self.data.take() {
                events.push(SseEvent {
                    event: self.event.take(),
                    data,
                });
            }
            self.event = None;
            return;
        }
        if line[0] == b':' {
            return;
        }
        let text = String::from_utf8_lossy(line);
        let (field, value) = match text.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (text.as_ref(), ""),
        };
        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => match self.data.as_mut() {
                Some(d) => {
                    d.push('\n');
                    d.push_str(value);
                }
                None => self.data = Some(value.to_string()),
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_events_split_across_chunks() {
        let mut d = SseDecoder::new();
        let mut out = d
            .feed(b"event: endpoint\ndata: /?s=1\n\n: keep-alive\n\nda")
            .unwrap();
        out.extend(d.feed(b"ta: {\"a\":\r").unwrap());
        out.extend(d.feed(b"\ndata: 1}\r\n\r\n").unwrap());
        assert_eq!(
            out,
            vec![
                SseEvent {
                    event: Some("endpoint".into()),
                    data: "/?s=1".into()
                },
                SseEvent {
                    event: None,
                    data: "{\"a\":\n1}".into()
                },
            ]
        );
    }

    #[test]
    fn utf8_cut_across_chunks_survives() {
        let mut d = SseDecoder::new();
        let bytes = "data: ž\n\n".as_bytes();
        let mut out = d.feed(&bytes[..7]).unwrap();
        out.extend(d.feed(&bytes[7..]).unwrap());
        assert_eq!(out[0].data, "ž");
    }
}
