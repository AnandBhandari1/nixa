use crate::cert::{self, DoorNames, RENEW_MS};
use crate::files::{self, PhoneRecord};
use crate::secret::{hex, is_hex, is_key, random_id, random_key, same, sha256, sha256_bytes, unhex};
use crate::wire::{self, Frame, Lens, PhoneEvent, PlayedHow};
use crate::{log, net, ports, qr, routes, secret, tls};
use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub type SnapshotFn = Arc<dyn Fn() -> Value + Send + Sync>;
pub type AddressFn = Arc<dyn Fn() -> Option<String> + Send + Sync>;
pub type NamesFn = Arc<dyn Fn(&str) -> DoorNames + Send + Sync>;
pub type NowFn = Arc<dyn Fn() -> u64 + Send + Sync>;

pub const GONE_MS: u64 = 5000;
pub const WATCH_MS: u64 = 30_000;
pub const SWEEP_MS: u64 = 1000;
pub const COOKIE: &str = "nixa_phone";
pub const KEEP_SECONDS: u64 = 365 * 24 * 60 * 60;
const STOP_WAIT: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct Deps {
    pub data_dir: PathBuf,
    pub port: u16,
    pub setup_port: u16,
    pub snapshot: SnapshotFn,
    pub address: AddressFn,
    pub names: NamesFn,
    pub now: NowFn,
    pub gone_ms: u64,
    pub watch_ms: u64,
}

impl Deps {
    pub fn new(data_dir: &Path, snapshot: SnapshotFn) -> Deps {
        Deps {
            data_dir: data_dir.to_path_buf(),
            port: ports::PHONE_PORT,
            setup_port: ports::SETUP_PORT,
            snapshot,
            address: Arc::new(net::wifi_address),
            names: Arc::new(net::door_names),
            now: Arc::new(crate::now_ms),
            gone_ms: GONE_MS,
            watch_ms: WATCH_MS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PhoneCheck {
    pub ok: bool,
    pub detail: String,
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PhoneRow {
    pub id: String,
    pub name: String,
    pub paired_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<u64>,
    pub connected: bool,
    pub voice: bool,
    pub checks: BTreeMap<String, PhoneCheck>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CameraPhone {
    pub id: String,
    pub name: String,
    pub until: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PhoneInfo {
    pub on: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub door_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pair_qr: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_pin: Option<String>,
    pub phones: Vec<PhoneRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub(crate) enum Out {
    Text(String),
    Binary(Vec<u8>),
    Close(u16, &'static str),
    Drop,
}

pub(crate) struct Talk {
    pub parts: Vec<u8>,
    pub at: u64,
    pub auto: bool,
    pub peeked: bool,
}

pub(crate) struct Look {
    pub id: u32,
    pub conn: u64,
    pub frames: Vec<Frame>,
}

pub(crate) struct Conn {
    pub phone: String,
    pub camera: bool,
    pub last: u64,
    pub voice: bool,
    pub text_only: bool,
    pub talk: Option<Talk>,
    pub tx: UnboundedSender<Out>,
}

impl Conn {
    pub fn send(&self, out: Out) {
        let _ = self.tx.send(out);
    }

    pub fn open(&self) -> bool {
        !self.tx.is_closed()
    }
}

pub(crate) struct Line {
    pub targets: HashSet<u64>,
    pub any_played: bool,
    pub done: Option<PlayedHow>,
    pub waiter: Option<oneshot::Sender<PlayedHow>>,
}

pub(crate) struct Open {
    pub address: String,
    pub port: u16,
    pub setup_port: u16,
    pub hosts: HashSet<String>,
    pub ca_der: Vec<u8>,
    pub root_url: String,
    pub cert_until: u64,
    stop: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
}

#[derive(Default)]
pub(crate) struct State {
    pub on: bool,
    pub open: Option<Open>,
    pub pair_qr: Option<Vec<String>>,
    pub error: Option<String>,
    pub door_key: Option<String>,
    pub phones: Vec<PhoneRecord>,
    pub checks: HashMap<String, BTreeMap<String, PhoneCheck>>,
    pub conns: HashMap<u64, Conn>,
    pub next_conn: u64,
    pub lines: HashMap<u32, Line>,
    pub watch: Option<CancellationToken>,
    pub looks: HashMap<String, Look>,
}

struct Inner {
    deps: Deps,
    state: Mutex<State>,
    events: UnboundedSender<PhoneEvent>,
    queue: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
pub struct Door {
    inner: Arc<Inner>,
}

pub(crate) enum Pair {
    Wrong,
    Already,
    New(String),
}

impl Door {
    pub fn new(deps: Deps) -> Result<(Door, UnboundedReceiver<PhoneEvent>)> {
        ports::check_ports(deps.port, deps.setup_port)?;
        let phones = files::read_phones(&files::phones_path(&deps.data_dir));
        let (events, rx) = unbounded_channel();
        let state = State { phones, ..State::default() };
        let inner = Inner { deps, state: Mutex::new(state), events, queue: tokio::sync::Mutex::new(()) };
        Ok((Door { inner: Arc::new(inner) }, rx))
    }

    pub(crate) fn deps(&self) -> &Deps {
        &self.inner.deps
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn now(&self) -> u64 {
        (self.inner.deps.now)()
    }

    pub(crate) fn emit(&self, event: PhoneEvent) {
        let _ = self.inner.events.send(event);
    }

    pub(crate) fn changed(&self) {
        self.emit(PhoneEvent::Changed);
    }

    pub(crate) fn save(&self, st: &State) {
        if let Err(e) = files::write_phones(&files::phones_path(&self.inner.deps.data_dir), &st.phones) {
            log("phone", &format!("could not save phones: {e:#}"));
        }
    }

    fn door_key(&self, st: &mut State) -> Result<String> {
        if let Some(key) = &st.door_key {
            return Ok(key.clone());
        }
        let key = files::read_door_key(&files::key_path(&self.inner.deps.data_dir))?;
        st.door_key = Some(key.clone());
        Ok(key)
    }

    pub async fn start(&self) {
        let _queue = self.inner.queue.lock().await;
        {
            let mut st = self.lock();
            if st.on && st.open.is_some() {
                return;
            }
            st.on = true;
        }
        self.open().await;
        self.ensure_watch();
        self.changed();
    }

    pub async fn stop(&self) {
        let _queue = self.inner.queue.lock().await;
        let watch = {
            let mut st = self.lock();
            st.on = false;
            st.error = None;
            st.watch.take()
        };
        if let Some(watch) = watch {
            watch.cancel();
        }
        self.shut_down().await;
        log("phone", "door closed");
        self.changed();
    }

    pub async fn refresh(&self) {
        let _queue = self.inner.queue.lock().await;
        if !self.lock().on {
            return;
        }
        self.shut_down().await;
        self.open().await;
        self.ensure_watch();
        self.changed();
    }

    pub async fn new_key(&self) -> Result<()> {
        let _queue = self.inner.queue.lock().await;
        let key = files::write_door_key(&files::key_path(&self.inner.deps.data_dir))?;
        let removed = {
            let mut st = self.lock();
            st.door_key = Some(key);
            let removed = st.phones.len();
            st.phones.clear();
            st.checks.clear();
            let phones: Vec<String> = st.looks.keys().cloned().collect();
            for phone in phones {
                self.fail_look(&mut st, &phone, "error");
            }
            self.save(&st);
            for conn in st.conns.values() {
                conn.send(Out::Close(4001, "removed"));
            }
            removed
        };
        self.make_pair_qr();
        log("phone", &format!("new door key, {removed} phones removed"));
        self.changed();
        Ok(())
    }

    pub fn remove(&self, id: &str) -> bool {
        {
            let mut st = self.lock();
            if !st.phones.iter().any(|p| p.id == id) {
                return false;
            }
            st.phones.retain(|p| p.id != id);
            st.checks.remove(id);
            self.fail_look(&mut st, id, "error");
            self.save(&st);
            for conn in st.conns.values().filter(|c| c.phone == id) {
                conn.send(Out::Close(4001, "removed"));
            }
        }
        log("phone", &format!("removed {id}"));
        self.changed();
        true
    }

    pub fn relay(&self, msg: &Value) {
        let st = self.lock();
        if st.conns.is_empty() {
            return;
        }
        let Some(text) = wire::relayed(msg) else {
            return;
        };
        for conn in st.conns.values().filter(|c| c.open()) {
            conn.send(Out::Text(text.clone()));
        }
    }

    pub fn info(&self) -> PhoneInfo {
        let st = self.lock();
        let live: HashSet<&str> = st.conns.values().map(|c| c.phone.as_str()).collect();
        let phones = st
            .phones
            .iter()
            .map(|p| PhoneRow {
                id: p.id.clone(),
                name: p.name.clone(),
                paired_at: p.paired_at,
                last_seen: p.last_seen,
                connected: live.contains(p.id.as_str()),
                voice: st.conns.values().any(|c| c.phone == p.id && c.voice && !c.text_only),
                checks: st.checks.get(&p.id).cloned().unwrap_or_default(),
            })
            .collect();
        let open = st.open.as_ref();
        PhoneInfo {
            on: st.on,
            address: open.map(|o| o.address.clone()),
            door_url: open.map(|o| format!("https://{}:{}/", o.address, o.port)),
            pair_qr: st.pair_qr.clone(),
            root_url: open.map(|o| o.root_url.clone()),
            root_sha256: open.map(|o| hex(&sha256_bytes(&o.ca_der))),
            root_pin: open.map(|o| qr::root_pin(&o.ca_der)),
            phones,
            error: st.error.clone(),
        }
    }

    pub fn look_request(&self, phone: &str, id: u32, lens: Lens) -> bool {
        let mut guard = self.lock();
        let st = &mut *guard;
        if st.looks.contains_key(phone) {
            return false;
        }
        let Some((&no, conn)) = st.conns.iter().find(|(_, c)| c.phone == phone && c.camera && c.open()) else {
            return false;
        };
        conn.send(Out::Text(wire::look_request(id, lens)));
        st.looks.insert(phone.to_string(), Look { id, conn: no, frames: Vec::new() });
        log("phone", &format!("{phone} look {id} asked, {lens:?} camera"));
        true
    }

    pub fn look_cancel(&self, phone: &str, id: u32) {
        let mut st = self.lock();
        if !st.looks.get(phone).is_some_and(|l| l.id == id) {
            return;
        }
        if let Some(look) = st.looks.remove(phone) {
            if let Some(conn) = st.conns.get(&look.conn) {
                conn.send(Out::Text(wire::look_cancel(id)));
            }
            log("phone", &format!("{phone} look {id} cancelled"));
        }
    }

    pub fn camera_phones(&self) -> Vec<CameraPhone> {
        let st = self.lock();
        let mut out: Vec<CameraPhone> = st
            .phones
            .iter()
            .filter(|p| st.conns.values().any(|c| c.phone == p.id && c.camera && c.open()))
            .map(|p| CameraPhone { id: p.id.clone(), name: p.name.clone(), until: 0 })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub fn is_on(&self) -> bool {
        self.lock().on
    }

    pub fn clients(&self) -> usize {
        self.lock().conns.len()
    }

    pub fn talking(&self) -> bool {
        self.lock().conns.values().any(|c| c.talk.is_some())
    }

    pub fn text_only(&self) -> bool {
        self.lock().conns.values().any(|c| c.text_only && c.open())
    }

    pub fn ports(&self) -> Option<(u16, u16)> {
        self.lock().open.as_ref().map(|o| (o.port, o.setup_port))
    }

    pub(crate) fn has_host(&self, host: &str) -> bool {
        self.lock().open.as_ref().is_some_and(|o| o.hosts.contains(host))
    }

    pub(crate) fn has_setup_host(&self, host: &str) -> bool {
        self.lock().open.as_ref().is_some_and(|o| {
            o.hosts.iter().filter_map(|h| h.rsplit_once(':')).any(|(name, _)| format!("{name}:{}", o.setup_port) == host)
        })
    }

    pub(crate) fn setup_view(&self) -> Option<Vec<u8>> {
        self.lock().open.as_ref().map(|o| o.ca_der.clone())
    }

    pub(crate) fn phone_of(&self, cookies: &str) -> Option<String> {
        let value = secret::cookie_value(cookies, COOKIE)?;
        let (id, key) = value.split_once('.')?;
        if !is_hex(id, 8) || !is_key(key) {
            return None;
        }
        let st = self.lock();
        let phone = st.phones.iter().find(|p| p.id == id)?;
        let want = unhex(&phone.key_hash)?;
        same(&want, &sha256(key)).then(|| phone.id.clone())
    }

    pub(crate) fn pair(&self, key: &str, cookies: &str, agent: &str) -> Pair {
        let already = self.phone_of(cookies).is_some();
        let made = {
            let mut st = self.lock();
            let Ok(door_key) = self.door_key(&mut st) else {
                return Pair::Wrong;
            };
            if !same(&sha256(key), &sha256(&door_key)) {
                drop(st);
                log("phone", "wrong door key");
                return Pair::Wrong;
            }
            if already {
                return Pair::Already;
            }
            let (Ok(id), Ok(cookie_key)) = (random_id(), random_key()) else {
                return Pair::Wrong;
            };
            let record = PhoneRecord {
                id: id.clone(),
                name: wire::phone_name(agent).to_string(),
                key_hash: secret::hex(&sha256(&cookie_key)),
                paired_at: self.now(),
                last_seen: None,
            };
            log("phone", &format!("paired {id} ({})", record.name));
            st.phones.push(record);
            self.save(&st);
            st.door_key = Some(match files::write_door_key(&files::key_path(&self.inner.deps.data_dir)) {
                Ok(next) => next,
                Err(e) => {
                    let _ = std::fs::remove_file(files::key_path(&self.inner.deps.data_dir));
                    log("phone", &format!("the used door key could not be replaced, so no new phone can pair until a new key is made: {e:#}"));
                    String::new()
                }
            });
            format!("{COOKIE}={id}.{cookie_key}; Path=/; Max-Age={KEEP_SECONDS}; Secure; HttpOnly; SameSite=Lax")
        };
        self.make_pair_qr();
        self.changed();
        Pair::New(made)
    }

    fn make_pair_qr(&self) {
        let url = {
            let mut st = self.lock();
            let Some((address, port, root)) = st.open.as_ref().map(|o| (o.address.clone(), o.port, o.ca_der.clone())) else {
                return;
            };
            match self.door_key(&mut st) {
                Ok(key) if key.is_empty() => {
                    st.pair_qr = None;
                    return;
                }
                Ok(key) => qr::pair_text(&address, port, &key, &root),
                Err(e) => {
                    log("phone", &format!("no door key: {e:#}"));
                    return;
                }
            }
        };
        let rows = qr::qr_rows(&url).map_err(|e| log("phone", &format!("pairing QR failed: {e:#}"))).ok();
        self.lock().pair_qr = rows;
    }

    async fn open(&self) {
        let Some(address) = (self.inner.deps.address)() else {
            self.lock().error = Some("No Wi-Fi address. Join a Wi-Fi network.".into());
            log("phone", "no wifi address");
            return;
        };
        match self.bind(&address).await {
            Ok(open) => {
                log("phone", &format!("door open https://{}:{} root {}", open.address, open.port, open.root_url));
                let mut st = self.lock();
                st.open = Some(open);
                st.error = None;
            }
            Err(e) => {
                let message = format!("{e:#}");
                log("phone", &format!("door did not open: {message}"));
                self.lock().error = Some(message);
                return;
            }
        }
        self.make_pair_qr();
    }

    async fn bind(&self, address: &str) -> Result<Open> {
        let deps = &self.inner.deps;
        let ip: IpAddr = address.parse().context("bad Wi-Fi address")?;
        let names_fn = deps.names.clone();
        let dir = files::cert_dir(&deps.data_dir);
        let now = self.now();
        let addr = address.to_string();
        let (names, cert) = tokio::task::spawn_blocking(move || -> Result<(DoorNames, cert::DoorCert)> {
            let names = names_fn(&addr);
            let cert = cert::door_cert(&dir, &names, now)?;
            Ok((names, cert))
        })
        .await
        .context("the certificate job stopped")??;
        let config = cert::tls_config(&cert)?;
        let https = TcpListener::bind((ip, deps.port))
            .await
            .with_context(|| format!("the phone door could not use port {}", deps.port))?;
        let http = TcpListener::bind((ip, deps.setup_port))
            .await
            .with_context(|| format!("the setup page could not use port {}", deps.setup_port))?;
        let port = https.local_addr()?.port();
        let setup_port = http.local_addr()?.port();
        let mut hosts = HashSet::from([format!("{address}:{port}")]);
        hosts.extend(names.dns.iter().map(|name| format!("{name}:{port}")));
        let root_url = format!("http://{address}:{setup_port}{}", routes::ROOT_PATH);
        let stop = CancellationToken::new();
        let mut tasks = tls::serve_tls(https, config, routes::door_router(self.clone()), stop.clone());
        tasks.extend(tls::serve_plain(http, routes::setup_router(self.clone()), stop.clone()));
        tasks.push(self.sweeper(stop.clone()));
        Ok(Open {
            address: address.to_string(),
            port,
            setup_port,
            hosts,
            ca_der: cert.ca_der,
            root_url,
            cert_until: cert.until,
            stop,
            tasks,
        })
    }

    async fn shut_down(&self) {
        let open = {
            let mut st = self.lock();
            for conn in st.conns.values() {
                conn.send(Out::Drop);
            }
            st.pair_qr = None;
            st.open.take()
        };
        let Some(open) = open else {
            return;
        };
        open.stop.cancel();
        for task in open.tasks {
            let abort = task.abort_handle();
            if tokio::time::timeout(STOP_WAIT, task).await.is_err() {
                abort.abort();
            }
        }
    }

    fn sweeper(&self, stop: CancellationToken) -> JoinHandle<()> {
        let door = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = stop.cancelled() => break,
                    _ = tokio::time::sleep(Duration::from_millis(SWEEP_MS)) => door.sweep(),
                }
            }
        })
    }

    pub(crate) fn sweep(&self) {
        let now = self.now();
        let gone = self.inner.deps.gone_ms;
        let st = self.lock();
        for conn in st.conns.values().filter(|c| now.saturating_sub(c.last) > gone) {
            log("phone", &format!("{} gone after {} ms", conn.phone, now.saturating_sub(conn.last)));
            conn.send(Out::Drop);
        }
    }

    fn ensure_watch(&self) {
        let token = {
            let mut st = self.lock();
            if st.watch.is_some() {
                return;
            }
            let token = CancellationToken::new();
            st.watch = Some(token.clone());
            token
        };
        let door = self.clone();
        let every = Duration::from_millis(self.inner.deps.watch_ms.max(1));
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = token.cancelled() => break,
                    _ = tokio::time::sleep(every) => door.watch_tick().await,
                }
            }
        });
    }

    async fn watch_tick(&self) {
        let found = (self.inner.deps.address)();
        let now = self.now();
        let due = {
            let st = self.lock();
            if !st.on {
                return;
            }
            let current = st.open.as_ref().map(|o| o.address.clone());
            let renew = st.open.as_ref().is_some_and(|o| o.cert_until.saturating_sub(now) <= RENEW_MS);
            found != current || renew
        };
        if due {
            log("phone", "wifi address or certificate changed, opening the door again");
            self.refresh().await;
        }
    }
}
