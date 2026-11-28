use crate::door::{Conn, Door, Look, Out, PhoneCheck, State, Talk};
use crate::log;
use crate::wire::{self, In, LOOK_FRAMES, PEEK_BYTES, PhoneEvent, PlayedHow, TALK_MAX_BYTES, TALK_RATE};
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

pub(crate) async fn run(door: Door, socket: WebSocket, phone: String) {
    let first = wire::snapshot(&(door.deps().snapshot)());
    let (tx, mut rx) = unbounded_channel();
    let _ = tx.send(Out::Text(first));
    let no = door.add_conn(&phone, tx);
    let (mut sink, mut stream) = socket.split();
    let writer = async {
        while let Some(out) = rx.recv().await {
            let msg = match out {
                Out::Text(text) => Message::Text(text.into()),
                Out::Binary(bytes) => Message::Binary(bytes.into()),
                Out::Close(code, reason) => {
                    let _ = sink.send(Message::Close(Some(CloseFrame { code, reason: reason.into() }))).await;
                    break;
                }
                Out::Drop => break,
            };
            if sink.send(msg).await.is_err() {
                break;
            }
        }
    };
    let reader = async {
        while let Some(Ok(msg)) = stream.next().await {
            match msg {
                Message::Text(text) => door.on_text(no, text.as_str()),
                Message::Binary(bytes) => door.on_binary(no, &bytes),
                Message::Close(_) => break,
                _ => door.touch(no),
            }
        }
    };
    tokio::select! {
        _ = writer => {}
        _ = reader => {}
    }
    door.remove_conn(no);
}

impl Door {
    pub(crate) fn add_conn(&self, phone: &str, tx: UnboundedSender<Out>) -> u64 {
        let now = self.now();
        let no = {
            let mut st = self.lock();
            let no = st.next_conn;
            st.next_conn += 1;
            if !st.phones.iter().any(|p| p.id == phone) {
                let _ = tx.send(Out::Close(4001, "removed"));
                log("phone", &format!("{phone} was removed before its socket opened"));
                return no;
            }
            let older: Vec<u64> = st.conns.iter().filter(|(_, c)| c.phone == phone).map(|(n, _)| *n).collect();
            for old in older {
                if let Some(conn) = st.conns.get(&old) {
                    conn.send(Out::Close(4002, "replaced"));
                }
                log("phone", &format!("{phone} opened a new socket, the older one is closed"));
            }
            st.conns.insert(no, Conn { phone: phone.to_string(), camera: false, last: now, voice: false, text_only: false, talk: None, tx });
            if let Some(record) = st.phones.iter_mut().find(|p| p.id == phone) {
                record.last_seen = Some(now);
            }
            log("phone", &format!("{phone} connected, {} open", st.conns.len()));
            no
        };
        self.changed();
        no
    }

    pub(crate) fn remove_conn(&self, no: u64) {
        let now = self.now();
        {
            let mut st = self.lock();
            let Some(conn) = st.conns.remove(&no) else {
                return;
            };
            st.drop_from_lines(no);
            if st.looks.get(&conn.phone).is_some_and(|l| l.conn == no) {
                self.fail_look(&mut st, &conn.phone, "error");
            }
            if conn.talk.is_some() {
                self.emit(PhoneEvent::TalkCancel { phone: conn.phone.clone() });
            }
            if let Some(record) = st.phones.iter_mut().find(|p| p.id == conn.phone) {
                record.last_seen = Some(now);
            }
            self.save(&st);
            log("phone", &format!("{} left, {} open", conn.phone, st.conns.len()));
        }
        self.changed();
    }

    pub(crate) fn touch(&self, no: u64) {
        let now = self.now();
        if let Some(conn) = self.lock().conns.get_mut(&no) {
            conn.last = now;
        }
    }

    pub(crate) fn on_binary(&self, no: u64, bytes: &[u8]) {
        let now = self.now();
        let mut st = self.lock();
        let Some(conn) = st.conns.get_mut(&no) else {
            return;
        };
        conn.last = now;
        let phone = conn.phone.clone();
        let camera = conn.camera;
        if wire::is_frame(bytes) && st.looks.get(&phone).is_some_and(|l| l.conn == no) {
            self.take_frame(&mut st, &phone, bytes);
            return;
        }
        if wire::is_frame(bytes) && camera {
            return;
        }
        let Some(talk) = st.conns.get_mut(&no).and_then(|c| c.talk.as_mut()) else {
            return;
        };
        if talk.parts.len() + bytes.len() > TALK_MAX_BYTES {
            return;
        }
        talk.parts.extend_from_slice(bytes);
        if talk.auto && !talk.peeked && talk.parts.len() >= PEEK_BYTES {
            talk.peeked = true;
            let pcm = wire::to_i16(&talk.parts);
            self.emit(PhoneEvent::TalkPeek { phone, pcm });
        }
    }

    pub(crate) fn on_text(&self, no: u64, text: &str) {
        let now = self.now();
        let parsed = wire::parse_in(text);
        let mut st = self.lock();
        let Some(conn) = st.conns.get_mut(&no) else {
            return;
        };
        conn.last = now;
        let phone = conn.phone.clone();
        let Some(msg) = parsed else {
            return;
        };
        match msg {
            In::Here(t) => conn.send(Out::Text(wire::here(&t))),
            In::TextOnly(on) => {
                if conn.text_only == on {
                    return;
                }
                conn.text_only = on;
                if on {
                    st.drop_from_lines(no);
                }
                log("phone", &format!("{phone} text only {}", if on { "on" } else { "off" }));
                self.changed();
            }
            In::Voice(ready) => {
                if conn.voice == ready {
                    return;
                }
                conn.voice = ready;
                let text_only = conn.text_only;
                if !ready {
                    st.drop_from_lines(no);
                }
                log("phone", &format!("{phone} sound {}", if ready { "on" } else { "off" }));
                self.changed();
                if !ready || !text_only {
                    self.emit(PhoneEvent::Voice { phone, ready });
                }
            }
            In::Command(command) => {
                log("phone", &format!("{phone} sent a command"));
                self.emit(PhoneEvent::Command { phone, command });
            }
            In::TalkStart(auto) => {
                conn.talk = Some(Talk { parts: Vec::new(), at: now, auto, peeked: false });
                log("phone", &format!("{phone} talk start, auto {auto}"));
                self.emit(PhoneEvent::TalkStart { phone, auto });
            }
            In::TalkEnd | In::TalkCancel => {
                let Some(talk) = conn.talk.take() else {
                    return;
                };
                let pcm = wire::to_i16(&talk.parts);
                let ms = pcm.len() * 1000 / TALK_RATE;
                if msg == In::TalkEnd {
                    log("phone", &format!("{phone} talk end, {ms} ms"));
                    self.emit(PhoneEvent::Talk { phone, pcm, started_at: talk.at, auto: talk.auto });
                } else {
                    log("phone", &format!("{phone} talk cancelled, {ms} ms"));
                    self.emit(PhoneEvent::TalkCancel { phone });
                }
            }
            In::Played(line) => {
                let Some(rec) = st.lines.get_mut(&line) else {
                    return;
                };
                if !rec.targets.remove(&no) {
                    return;
                }
                rec.any_played = true;
                if rec.targets.is_empty() {
                    st.finish_line(line, PlayedHow::Played);
                }
            }
            In::Check { name, ok, detail } => {
                log("phone", &format!("{phone} check {name} ok {ok} {detail}"));
                st.checks.entry(phone).or_default().insert(name, PhoneCheck { ok, detail, at: now });
                self.changed();
            }
            In::Caps { camera } => {
                conn.camera = camera;
                if !camera {
                    self.fail_look(&mut st, &phone, "denied");
                }
                log("phone", &format!("{phone} camera caps {camera}"));
                self.emit(PhoneEvent::Caps { phone, camera });
            }
            In::LookEnd { id, frames } => {
                if !st.looks.get(&phone).is_some_and(|l| l.id == id && l.conn == no) {
                    return;
                }
                let full = st.looks.get(&phone).is_some_and(|l| frames > 0 && usize::from(frames) == l.frames.len());
                if !full {
                    self.fail_look(&mut st, &phone, "error");
                    return;
                }
                let Some(Look { mut frames, .. }) = st.looks.remove(&phone) else {
                    return;
                };
                frames.sort_by_key(|f| f.index);
                log("phone", &format!("{phone} look {id} done, {} frames", frames.len()));
                self.emit(PhoneEvent::LookFrames { phone, id, frames });
            }
            In::LookFail { id, why } => {
                if !st.looks.get(&phone).is_some_and(|l| l.id == id && l.conn == no) {
                    return;
                }
                st.looks.remove(&phone);
                log("phone", &format!("{phone} look {id} failed on the phone: {why}"));
                self.emit(PhoneEvent::LookFail { phone, id, why });
            }
        }
    }

    fn take_frame(&self, st: &mut State, phone: &str, bytes: &[u8]) {
        let Some(look) = st.looks.get_mut(phone) else {
            return;
        };
        let taken = match wire::parse_frame(bytes) {
            Some((id, frame)) if id == look.id && look.frames.len() < usize::from(LOOK_FRAMES) && !look.frames.iter().any(|f| f.index == frame.index) => {
                look.frames.push(frame);
                true
            }
            _ => false,
        };
        if !taken {
            log("phone", &format!("{phone} sent a bad look frame"));
            self.fail_look(st, phone, "error");
        }
    }

    pub(crate) fn fail_look(&self, st: &mut State, phone: &str, why: &str) {
        let Some(look) = st.looks.remove(phone) else {
            return;
        };
        if let Some(conn) = st.conns.get(&look.conn) {
            conn.send(Out::Text(wire::look_cancel(look.id)));
        }
        log("phone", &format!("{phone} look {} failed: {why}", look.id));
        self.emit(PhoneEvent::LookFail { phone: phone.to_string(), id: look.id, why: why.to_string() });
    }
}

#[cfg(test)]
mod tests {
    use crate::cert::DoorNames;
    use crate::door::{Deps, Door, Out};
    use crate::files::PhoneRecord;
    use crate::wire::{FRAME_TAG, Lens, PhoneEvent};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    struct Rig {
        door: Door,
        events: UnboundedReceiver<PhoneEvent>,
        _clock: Arc<AtomicU64>,
        _dir: tempfile::TempDir,
    }

    fn rig() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(AtomicU64::new(1_000_000));
        let mut deps = Deps::new(dir.path(), Arc::new(|| json!({})));
        deps.port = 0;
        deps.setup_port = 0;
        deps.names = Arc::new(|address| DoorNames { ips: vec![address.to_string()], dns: Vec::new() });
        let at = clock.clone();
        deps.now = Arc::new(move || at.load(Ordering::SeqCst));
        let (door, events) = Door::new(deps).unwrap();
        door.lock().phones.push(PhoneRecord { id: "p1".into(), name: "Android phone".into(), key_hash: String::new(), paired_at: 0, last_seen: None });
        Rig { door, events, _clock: clock, _dir: dir }
    }

    fn link(rig: &Rig) -> (u64, UnboundedReceiver<Out>) {
        let (tx, rx) = unbounded_channel();
        (rig.door.add_conn("p1", tx), rx)
    }

    fn texts(rx: &mut UnboundedReceiver<Out>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(msg) = rx.try_recv() {
            if let Out::Text(text) = msg {
                out.push(serde_json::from_str(&text).unwrap());
            }
        }
        out
    }

    fn camera_events(rx: &mut UnboundedReceiver<PhoneEvent>) -> Vec<PhoneEvent> {
        let mut out = Vec::new();
        while let Ok(event) = rx.try_recv() {
            if matches!(event, PhoneEvent::Caps { .. } | PhoneEvent::LookFrames { .. } | PhoneEvent::LookFail { .. } | PhoneEvent::TalkPeek { .. } | PhoneEvent::Talk { .. }) {
                out.push(event);
            }
        }
        out
    }

    fn frame(id: u32, index: u8) -> Vec<u8> {
        let mut out = FRAME_TAG.to_vec();
        out.extend_from_slice(&id.to_le_bytes());
        out.push(index);
        out.extend_from_slice(&5u64.to_le_bytes());
        out.extend_from_slice(&[0xFF, 0xD8, index]);
        out
    }

    fn camera_on(rig: &mut Rig, no: u64, rx: &mut UnboundedReceiver<Out>) {
        rig.door.on_text(no, r#"{"type":"caps","camera":true}"#);
        texts(rx);
        camera_events(&mut rig.events);
    }

    #[test]
    fn a_connected_camera_phone_gives_one_photo_with_no_session() {
        let mut rig = rig();
        let (no, mut rx) = link(&rig);
        assert!(!rig.door.look_request("p1", 1, Lens::Rear));
        rig.door.on_text(no, r#"{"type":"caps","camera":true}"#);
        assert!(texts(&mut rx).is_empty());
        assert_eq!(rig.door.camera_phones().len(), 1);
        assert!(rig.door.look_request("p1", 7, Lens::Front));
        assert!(!rig.door.look_request("p1", 8, Lens::Rear));
        let sent = texts(&mut rx);
        assert_eq!((sent[0]["camera"].as_str(), sent[0]["frames"].as_u64()), (Some("front"), Some(1)));
        camera_events(&mut rig.events);
        rig.door.on_binary(no, &frame(7, 0));
        rig.door.on_text(no, r#"{"type":"look_end","id":7,"frames":1}"#);
        match camera_events(&mut rig.events).as_slice() {
            [PhoneEvent::LookFrames { phone, id: 7, frames }] => {
                assert_eq!(phone, "p1");
                assert_eq!(frames.len(), 1);
            }
            other => panic!("{other:?}"),
        }
        rig.door.on_text(no, r#"{"type":"caps","camera":false}"#);
        assert!(rig.door.camera_phones().is_empty());
    }

    #[test]
    fn bad_frames_fail_the_look_and_talk_audio_still_works() {
        let bad: [Vec<Vec<u8>>; 5] = [
            vec![frame(9, 0)],
            vec![frame(7, 0), frame(7, 0)],
            vec![{
                let mut f = frame(7, 0);
                f[17] = 0x89;
                f
            }],
            vec![{
                let mut f = frame(7, 0);
                f.resize(60_001, 0);
                f
            }],
            vec![frame(7, 3)],
        ];
        for frames in bad {
            let mut rig = rig();
            let (no, mut rx) = link(&rig);
            camera_on(&mut rig, no, &mut rx);
            assert!(rig.door.look_request("p1", 7, Lens::Rear));
            for f in &frames {
                rig.door.on_binary(no, f);
            }
            assert_eq!(camera_events(&mut rig.events), vec![PhoneEvent::LookFail { phone: "p1".into(), id: 7, why: "error".into() }]);
            assert!(texts(&mut rx).contains(&json!({"type": "look_cancel", "id": 7})));
        }
        let mut rig = rig();
        let (no, mut rx) = link(&rig);
        camera_on(&mut rig, no, &mut rx);
        assert!(rig.door.look_request("p1", 7, Lens::Rear));
        rig.door.on_binary(no, &frame(7, 0));
        rig.door.on_text(no, r#"{"type":"look_end","id":7,"frames":3}"#);
        assert_eq!(camera_events(&mut rig.events), vec![PhoneEvent::LookFail { phone: "p1".into(), id: 7, why: "error".into() }]);
        rig.door.on_text(no, r#"{"type":"talk_start","auto":false}"#);
        rig.door.on_binary(no, &frame(7, 1));
        rig.door.on_binary(no, &[1, 0, 2, 0]);
        rig.door.on_text(no, r#"{"type":"talk_end"}"#);
        match camera_events(&mut rig.events).as_slice() {
            [PhoneEvent::Talk { pcm, .. }] => assert_eq!(pcm.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_phone_can_fail_a_look_and_old_apps_get_none() {
        let mut rig = rig();
        let (no, mut rx) = link(&rig);
        assert!(rig.door.camera_phones().is_empty());
        assert!(!rig.door.look_request("p1", 1, Lens::Rear));
        camera_on(&mut rig, no, &mut rx);
        assert!(rig.door.look_request("p1", 2, Lens::Rear));
        rig.door.on_text(no, r#"{"type":"look_fail","id":2,"why":"denied"}"#);
        assert_eq!(camera_events(&mut rig.events), vec![PhoneEvent::LookFail { phone: "p1".into(), id: 2, why: "denied".into() }]);
        assert!(rig.door.look_request("p1", 3, Lens::Rear));
        rig.door.look_cancel("p1", 3);
        assert!(texts(&mut rx).contains(&json!({"type": "look_cancel", "id": 3})));
        assert!(rig.door.look_request("p1", 4, Lens::Rear));
    }

    #[test]
    fn forget_and_link_drop_fail_an_open_photo() {
        let mut rig = rig_with_camera();
        assert!(rig.0.door.look_request("p1", 5, Lens::Rear));
        assert!(rig.0.door.remove("p1"));
        assert!(camera_events(&mut rig.0.events).contains(&PhoneEvent::LookFail { phone: "p1".into(), id: 5, why: "error".into() }));

        let mut rig = rig_with_camera();
        assert!(rig.0.door.look_request("p1", 6, Lens::Rear));
        rig.0.door.remove_conn(rig.1);
        assert!(camera_events(&mut rig.0.events).contains(&PhoneEvent::LookFail { phone: "p1".into(), id: 6, why: "error".into() }));
        assert!(rig.0.door.camera_phones().is_empty());
    }

    #[test]
    fn frames_with_no_open_look_are_dropped_and_a_fourth_frame_fails_the_look() {
        let mut rig = rig();
        let (no, mut rx) = link(&rig);
        camera_on(&mut rig, no, &mut rx);
        rig.door.on_binary(no, &frame(9, 0));
        assert!(camera_events(&mut rig.events).is_empty());
        assert!(rig.door.look_request("p1", 7, Lens::Rear));
        for index in 0..3u8 {
            rig.door.on_binary(no, &frame(7, index));
        }
        rig.door.on_binary(no, &frame(7, 0));
        assert_eq!(
            camera_events(&mut rig.events),
            vec![PhoneEvent::LookFail { phone: "p1".into(), id: 7, why: "error".into() }]
        );
    }

    fn rig_with_camera() -> (Rig, u64, UnboundedReceiver<Out>) {
        let mut rig = rig();
        let (no, mut rx) = link(&rig);
        camera_on(&mut rig, no, &mut rx);
        (rig, no, rx)
    }
}
