use crate::term::Status;

pub const STATUS_OSC: &str = "7421";

pub fn status_osc(status: Status, note: Option<&str>) -> String {
    match note {
        Some(n) if !n.is_empty() => {
            let clean: String = n.chars().filter(|c| !c.is_control()).collect();
            format!("\x1b]{STATUS_OSC};status={status};note={clean}\x07")
        }
        _ => format!("\x1b]{STATUS_OSC};status={status}\x07"),
    }
}

pub fn parse_status_osc(params: &[&[u8]]) -> Option<(Status, Option<String>)> {
    let (first, rest) = params.split_first()?;
    if *first != STATUS_OSC.as_bytes() {
        return None;
    }
    let mut status = None;
    let mut note = None;
    for (i, part) in rest.iter().enumerate() {
        let text = std::str::from_utf8(part).ok()?;
        if let Some(v) = text.strip_prefix("status=") {
            status = Some(v.parse::<Status>().ok()?);
        } else if text.starts_with("note=") {
            let joined: Vec<&str> = rest[i..]
                .iter()
                .filter_map(|p| std::str::from_utf8(p).ok())
                .collect();
            let all = joined.join(";");
            let n = all.strip_prefix("note=").unwrap_or(&all).to_string();
            if !n.is_empty() {
                note = Some(n);
            }
            break;
        }
    }
    Some((status?, note))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(seq: &str) -> Vec<Vec<u8>> {
        let body = seq.strip_prefix("\x1b]").unwrap().strip_suffix('\x07').unwrap();
        body.split(';').map(|s| s.as_bytes().to_vec()).collect()
    }

    #[test]
    fn round_trip() {
        let seq = status_osc(Status::Blocked, Some("needs a yes; or no"));
        let parts = split(&seq);
        let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
        assert_eq!(
            parse_status_osc(&refs),
            Some((Status::Blocked, Some("needs a yes; or no".to_string())))
        );
        let seq = status_osc(Status::Idle, None);
        let parts = split(&seq);
        let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
        assert_eq!(parse_status_osc(&refs), Some((Status::Idle, None)));
    }

    #[test]
    fn note_keeps_its_own_note_prefix() {
        let seq = status_osc(Status::Blocked, Some("note=check logs"));
        let parts = split(&seq);
        let refs: Vec<&[u8]> = parts.iter().map(|p| p.as_slice()).collect();
        assert_eq!(
            parse_status_osc(&refs),
            Some((Status::Blocked, Some("note=check logs".to_string())))
        );
    }

    #[test]
    fn other_osc_is_ignored() {
        assert_eq!(parse_status_osc(&[b"0", b"title"]), None);
        assert_eq!(parse_status_osc(&[b"7421", b"status=sleepy"]), None);
    }
}
