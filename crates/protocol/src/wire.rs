use crate::reply::Reply;
use crate::request::Request;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::{self, BufRead, Read, Write};

pub const MAX_LINE: usize = 64 * 1024 * 1024;

fn append_json<T: Serialize>(msg: &T, buf: &mut Vec<u8>) {
    let start = buf.len();
    if let Err(e) = serde_json::to_writer(&mut *buf, msg) {
        buf.truncate(start);
        let _ = serde_json::to_writer(
            &mut *buf,
            &Reply::error(crate::ErrorCode::Internal, e.to_string()),
        );
    }
    buf.push(b'\n');
}

fn to_line<T: Serialize>(msg: &T) -> String {
    let mut buf = Vec::with_capacity(128);
    append_json(msg, &mut buf);
    String::from_utf8(buf).unwrap_or_default()
}

fn parse<T: DeserializeOwned>(line: &str) -> io::Result<T> {
    serde_json::from_str(line.trim_end()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

fn read_one<T: DeserializeOwned>(r: &mut dyn BufRead) -> io::Result<Option<T>> {
    let mut line = String::new();
    read_capped_into(r, MAX_LINE, &mut line)
}

#[cfg(test)]
fn read_capped<T: DeserializeOwned>(r: &mut dyn BufRead, cap: usize) -> io::Result<Option<T>> {
    let mut line = String::new();
    read_capped_into(r, cap, &mut line)
}

fn read_capped_into<T: DeserializeOwned>(
    r: &mut dyn BufRead,
    cap: usize,
    line: &mut String,
) -> io::Result<Option<T>> {
    loop {
        line.clear();
        let limit = u64::try_from(cap).unwrap_or(u64::MAX).saturating_add(1);
        let n = r.take(limit).read_line(line)?;
        if n == 0 {
            return Ok(None);
        }
        if line.len() > cap {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "line too long"));
        }
        if !line.trim().is_empty() {
            return parse(line).map(Some);
        }
    }
}

pub fn request_line(req: &Request) -> String {
    to_line(req)
}

pub fn reply_line(reply: &Reply) -> String {
    to_line(reply)
}

pub fn append_reply(reply: &Reply, buf: &mut Vec<u8>) {
    append_json(reply, buf);
}

pub fn parse_request(line: &str) -> io::Result<Request> {
    parse(line)
}

pub fn parse_reply(line: &str) -> io::Result<Reply> {
    parse(line)
}

pub fn write_request_buf(w: &mut dyn Write, req: &Request, buf: &mut Vec<u8>) -> io::Result<()> {
    buf.clear();
    append_json(req, buf);
    w.write_all(buf)?;
    w.flush()
}

pub fn write_reply_buf(w: &mut dyn Write, reply: &Reply, buf: &mut Vec<u8>) -> io::Result<()> {
    buf.clear();
    append_json(reply, buf);
    w.write_all(buf)?;
    w.flush()
}

pub fn write_request(w: &mut dyn Write, req: &Request) -> io::Result<()> {
    let mut buf = Vec::with_capacity(128);
    write_request_buf(w, req, &mut buf)
}

pub fn write_reply(w: &mut dyn Write, reply: &Reply) -> io::Result<()> {
    let mut buf = Vec::with_capacity(128);
    write_reply_buf(w, reply, &mut buf)
}

pub fn read_request(r: &mut dyn BufRead) -> io::Result<Option<Request>> {
    read_one(r)
}

pub fn read_request_into(r: &mut dyn BufRead, line: &mut String) -> io::Result<Option<Request>> {
    read_capped_into(r, MAX_LINE, line)
}

pub fn read_reply(r: &mut dyn BufRead) -> io::Result<Option<Reply>> {
    read_one(r)
}

pub fn read_reply_into(r: &mut dyn BufRead, line: &mut String) -> io::Result<Option<Reply>> {
    read_capped_into(r, MAX_LINE, line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Endless {
        given: usize,
    }

    impl Read for Endless {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            buf.fill(b'a');
            self.given += buf.len();
            Ok(buf.len())
        }
    }

    #[test]
    fn a_long_line_stops_at_the_cap() {
        let mut r = io::BufReader::with_capacity(64, Endless { given: 0 });
        let err = read_capped::<Request>(&mut r, 1000).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(r.get_ref().given < 1200);
    }

    #[test]
    fn lines_up_to_the_cap_are_read() {
        let line = request_line(&Request::Ping);
        let mut r = Cursor::new(format!("\n{line}{line}").into_bytes());
        let cap = line.len();
        assert!(matches!(read_capped::<Request>(&mut r, cap).unwrap(), Some(Request::Ping)));
        assert!(matches!(read_capped::<Request>(&mut r, cap).unwrap(), Some(Request::Ping)));
        assert!(read_capped::<Request>(&mut r, cap).unwrap().is_none());
        let mut r = Cursor::new(line.clone().into_bytes());
        assert!(read_capped::<Request>(&mut r, cap - 1).is_err());
    }
}
