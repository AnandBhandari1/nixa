#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

#[derive(Debug, Default)]
pub struct SseBuf {
    bytes: Vec<u8>,
}

fn parse_block(block: &str) -> Option<SseEvent> {
    let mut event = String::new();
    let mut data: Vec<&str> = Vec::new();
    for line in block.lines() {
        if line.starts_with(':') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("event:") {
            event = rest.trim().to_string();
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data.is_empty() && event.is_empty() {
        return None;
    }
    Some(SseEvent { event, data: data.join("\n") })
}

impl SseBuf {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.bytes.extend(chunk.iter().copied().filter(|b| *b != b'\r'));
        let mut out = Vec::new();
        while let Some(at) = self.bytes.windows(2).position(|pair| pair == b"\n\n") {
            let block: Vec<u8> = self.bytes.drain(..at + 2).collect();
            let text = String::from_utf8_lossy(&block[..at]);
            if let Some(event) = parse_block(&text) {
                out.push(event);
            }
        }
        out
    }

    pub fn finish(&mut self) -> Option<SseEvent> {
        let rest = std::mem::take(&mut self.bytes);
        let text = String::from_utf8_lossy(&rest);
        if text.trim().is_empty() { None } else { parse_block(&text) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_events_across_chunks() {
        let mut buf = SseBuf::default();
        assert!(buf.push(b"event: a\r\ndata: {\"x\"").is_empty());
        let got = buf.push(b":1}\r\n\r\ndata: two\n\n: ping\n\n");
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], SseEvent { event: "a".into(), data: "{\"x\":1}".into() });
        assert_eq!(got[1].data, "two");
        assert!(buf.finish().is_none());
    }

    #[test]
    fn keeps_utf8_split_bytes() {
        let mut buf = SseBuf::default();
        let text = "data: é\n\n".as_bytes();
        assert!(buf.push(&text[..7]).is_empty());
        let got = buf.push(&text[7..]);
        assert_eq!(got[0].data, "é");
    }
}
