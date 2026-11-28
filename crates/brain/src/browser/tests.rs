use super::gate::{StepArgs, Tab, check_step, check_tab, fold, other_site, page_of, tabs_of};
use super::notice::company_of;
use super::proxy::{Ask, parse_head};
use crate::web::safety::{public_address, public_text, public_url};
use super::step::{input_of, launch_args, should_stop};
use super::*;
use crate::app::App;
use std::net::IpAddr;

const PAGE: &str = "- Page URL: https://example.com/start\n- Page Title: Start\n- generic [ref=e1]:\n  - link \"More docs\" [ref=e2]:\n    - /url: /docs\n  - button \"Submit order\" [ref=e3]\n  - textbox \"Search\" [ref=e4]\n  - link \"Billing\" [ref=e5]:\n    - /url: https://pay.example.org/checkout\n";

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn args(target: Option<&str>, text: Option<&str>, url: Option<&str>) -> StepArgs {
    StepArgs { target: target.map(str::to_string), text: text.map(str::to_string), url: url.map(str::to_string), ..StepArgs::default() }
}

#[test]
fn public_text_blocks_private_words_and_long_text() {
    assert!(public_text("weather in a city").is_ok());
    assert!(public_text("").is_err());
    assert!(public_text("my password").is_err());
    assert!(public_text("open /Users/someone/file").is_err());
    assert!(public_text("card 4111 1111 1111 1111").is_err());
    assert!(public_text(&"a".repeat(241)).is_err());
    assert!(public_text("line\nbreak").is_err());
    assert!(public_text("write to someone@example.com").is_err());
}

#[test]
fn addresses_and_urls_must_be_public() {
    for ip in ["10.0.0.1", "127.0.0.1", "192.168.1.2", "172.20.0.1", "169.254.1.1", "100.64.0.1", "0.0.0.0", "::1", "fe80::1", "2001:db8::1", "::ffff:10.0.0.1"] {
        assert!(!public_address(ip.parse::<IpAddr>().unwrap()), "{ip}");
    }
    for ip in ["93.184.216.34", "8.8.8.8", "2606:4700::1111"] {
        assert!(public_address(ip.parse::<IpAddr>().unwrap()), "{ip}");
    }
    assert!(public_url("https://example.com/a?q=rust").is_ok());
    assert!(public_url("https://example.com:8443/").is_err());
    assert!(public_url("ftp://example.com/").is_err());
    assert!(public_url("https://user:pw@example.com/").is_err());
    assert!(public_url("http://printer.local/").is_err());
    assert!(public_url("http://localhost/").is_err());
    assert!(public_url("http://10.1.2.3/").is_err());
    assert!(public_url("https://example.com/my%20secret").is_err());
}

#[test]
fn page_snapshot_gives_url_refs_and_stamp() {
    let page = page_of(PAGE);
    assert_eq!(page.url, "https://example.com/start");
    assert!(page.refs["e2"].contains("/url: /docs"));
    assert!(page.refs["e1"].contains("Submit order"));
    assert!(!page.refs["e4"].contains("Billing"));
    assert_eq!(page.stamp.len(), 64);
    assert_ne!(page.stamp, page_of(&format!("{PAGE} ")).stamp);
    assert_eq!(fold("ｓｅｎｄ\u{200B}it"), "sendit");
}

#[test]
fn tabs_list_is_read() {
    let tabs = tabs_of("- 0: [Start](https://example.com/)\n- 1: (current) [Docs [v2]](https://example.com/docs)\nnoise");
    assert_eq!(tabs.len(), 2);
    assert_eq!(tabs[&1].title, "Docs [v2]");
    assert_eq!(tabs[&0].url, "https://example.com/");
}

#[test]
fn the_gate_checks_refs_and_lets_every_step_run() {
    let page = page_of(PAGE);
    let link = check_step(&page, "click", &args(Some("e2"), None, None)).unwrap();
    assert_eq!(link.url, "https://example.com/docs");
    assert!(check_step(&page, "click", &args(Some("e3"), None, None)).is_ok());
    assert!(check_step(&page, "click", &args(Some("e99"), None, None)).unwrap_err().contains("exact element ref"));
    assert!(check_step(&page, "click", &args(Some("#id"), None, None)).is_err());
    assert!(check_step(&page, "click", &args(None, None, None)).unwrap_err().contains("element ref"));
    assert!(check_step(&page, "type", &args(Some("e4"), Some("send it now"), None)).is_ok());
    assert!(check_step(&page, "key", &args(None, Some("Enter"), None)).is_ok());
    assert!(check_step(&page, "open", &args(None, None, Some("https://www.paypal.com/"))).is_ok());
    assert!(check_step(&page, "open", &args(None, None, Some("file:///etc/passwd"))).is_err());
    let login = page_of("- Page URL: https://example.com/\n- textbox \"Password\" [ref=e1]");
    assert!(check_step(&login, "click", &args(Some("e1"), None, None)).is_ok());
}

#[test]
fn tab_steps_check_the_tab_not_public_text() {
    let tab = |title: &str, url: &str| Tab { title: title.into(), url: url.into() };
    let blank = check_tab(&tab("New Tab", "about:blank")).unwrap();
    assert_eq!(blank.url, "about:blank");
    assert_eq!(blank.element, "New Tab");
    assert!(check_tab(&tab("My Docs", "https://example.com/docs")).is_ok());
    assert!(check_tab(&tab(&"long title ".repeat(40), "https://example.com/")).is_ok());
}

#[test]
fn card_and_inputs_match_the_step() {
    let warn = other_site("https://other.example.org/", "https://example.com/start");
    assert!(warn);
    assert!(!other_site("https://example.com/docs", "https://example.com/start"));
    let typed = input_of("type", &args(Some("e4"), Some("rust"), None), "textbox").unwrap();
    assert_eq!(typed["submit"], json!(false));
    assert_eq!(typed["target"], json!("e4"));
    assert!(input_of("open", &args(None, None, None), "").is_err());
    let tab = input_of("tab_close", &StepArgs { index: Some(2), ..StepArgs::default() }, "").unwrap();
    assert_eq!(tab, json!({"action": "close", "index": 2}));
    assert!(input_of("tab_switch", &StepArgs { index: Some(-1), ..StepArgs::default() }, "").is_err());
    assert_eq!(input_of("dialog", &StepArgs::default(), "").unwrap(), json!({"accept": false}));
}

#[test]
fn launch_args_keep_the_browser_closed_down() {
    let dir = std::path::Path::new("/tmp/x");
    let own = launch_args(Which::Own, dir, Some("http://127.0.0.1:9"));
    assert_eq!(own[0], "--isolated");
    assert!(own.windows(2).any(|w| w[0] == "--proxy-server" && w[1] == "http://127.0.0.1:9"));
    assert!(own.contains(&"<-loopback>".to_string()));
    let mine = launch_args(Which::Mine, dir, None);
    assert_eq!(mine[0], "--extension");
    assert!(!mine.contains(&"--proxy-server".to_string()));
    assert!(mine.contains(&"--profile-dir-name".to_string()));
}

#[test]
fn the_notice_names_the_receiving_company() {
    assert_eq!(company_of("codex", "", None), "OpenAI");
    assert_eq!(company_of("", "", Some("https://api.anthropic.com/v1")), "Anthropic (api.anthropic.com)");
    assert_eq!(company_of("", "", Some("http://127.0.0.1:11434/v1")), "the local brain on 127.0.0.1");
    assert_eq!(company_of("", "", Some("https://llm.example.net/v1")), "the brain service at llm.example.net");
    assert_eq!(company_of("opencode", "deepseek/chat", None), "DeepSeek (via OpenCode)");
    assert_eq!(company_of("opencode", "", None), "OpenCode's configured brain provider");
}

#[test]
fn the_proxy_checks_each_request() {
    assert_eq!(parse_head("CONNECT example.com:443 HTTP/1.1\r\nHost: example.com:443").unwrap(), Ask::Tunnel { host: "example.com".into() });
    assert!(parse_head("CONNECT example.com:22 HTTP/1.1").is_err());
    assert!(parse_head("CONNECT 10.0.0.1:443 HTTP/1.1").is_err());
    assert!(parse_head("CONNECT router.lan:443 HTTP/1.1").is_err());
    assert!(parse_head("GET http://127.0.0.1/ HTTP/1.1").is_err());
    assert!(parse_head("GET http://example.com/a-private-path HTTP/1.1").is_ok());
    assert!(parse_head("GET http://example.com:8080/ HTTP/1.1").is_err());
    let Ask::Plain { host, head } = parse_head("GET http://example.com/a?b=1 HTTP/1.1\r\nHost: evil.example\r\nProxy-Connection: keep-alive\r\nAccept: */*").unwrap() else { panic!("plain") };
    let head = String::from_utf8(head).unwrap();
    assert_eq!(host, "example.com");
    assert!(head.starts_with("GET /a?b=1 HTTP/1.1\r\nHost: example.com\r\n"));
    assert!(head.contains("Accept: */*"));
    assert!(!head.to_lowercase().contains("proxy-connection"));
    assert!(head.ends_with("Connection: close\r\n\r\n"));
}

#[tokio::test]
async fn the_proxy_refuses_private_targets_over_the_wire() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let guard = proxy::start().await.unwrap();
    let addr = guard.url.trim_start_matches("http://").to_string();
    let mut s = tokio::net::TcpStream::connect(&addr).await.unwrap();
    s.write_all(b"CONNECT 127.0.0.1:443 HTTP/1.1\r\n\r\n").await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 403"));
    proxy::close(&guard);
}

#[tokio::test]
async fn the_proxy_drops_a_client_that_never_sends_a_head() {
    use tokio::io::AsyncReadExt;
    let limits = proxy::Limits { head: std::time::Duration::from_millis(150), idle: std::time::Duration::from_secs(5) };
    let guard = proxy::start_with(limits).await.unwrap();
    let addr = guard.url.trim_start_matches("http://").to_string();
    let mut s = tokio::net::TcpStream::connect(&addr).await.unwrap();
    let mut out = Vec::new();
    let read = tokio::time::timeout(std::time::Duration::from_secs(3), s.read_to_end(&mut out)).await;
    assert!(read.is_ok(), "the proxy kept a silent client open");
    assert!(out.is_empty());
    proxy::close(&guard);
}

async fn pair() -> (tokio::net::TcpStream, tokio::net::TcpStream) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (near, far) = tokio::join!(tokio::net::TcpStream::connect(addr), listener.accept());
    (near.unwrap(), far.unwrap().0)
}

#[tokio::test]
async fn the_proxy_pump_carries_both_ways_then_ends_when_quiet() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut browser, proxy_side) = pair().await;
    let (upstream_side, mut site) = pair().await;
    let idle = std::time::Duration::from_millis(300);
    let pumping = tokio::spawn(proxy::pump(proxy_side, upstream_side, idle));
    browser.write_all(b"hello").await.unwrap();
    let mut got = [0u8; 5];
    site.read_exact(&mut got).await.unwrap();
    assert_eq!(&got, b"hello");
    for _ in 0..4 {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        site.write_all(b"x").await.unwrap();
        let mut one = [0u8; 1];
        browser.read_exact(&mut one).await.unwrap();
        assert_eq!(&one, b"x");
    }
    let mut rest = Vec::new();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(3), browser.read_to_end(&mut rest)).await;
    assert!(ended.is_ok(), "a quiet tunnel stayed open");
    assert!(rest.is_empty());
    assert!(tokio::time::timeout(std::time::Duration::from_secs(3), pumping).await.is_ok());
}

#[tokio::test]
async fn the_proxy_pump_keeps_the_other_way_open_after_one_side_ends() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut browser, proxy_side) = pair().await;
    let (upstream_side, mut site) = pair().await;
    let pumping = tokio::spawn(proxy::pump(proxy_side, upstream_side, std::time::Duration::from_secs(5)));
    browser.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
    browser.shutdown().await.unwrap();
    let mut asked = Vec::new();
    site.read_to_end(&mut asked).await.unwrap();
    assert_eq!(asked, b"GET / HTTP/1.1\r\n\r\n");
    site.write_all(b"HTTP/1.1 200 OK\r\n\r\nbody").await.unwrap();
    drop(site);
    let mut answer = Vec::new();
    browser.read_to_end(&mut answer).await.unwrap();
    assert_eq!(answer, b"HTTP/1.1 200 OK\r\n\r\nbody");
    assert!(tokio::time::timeout(std::time::Duration::from_secs(3), pumping).await.is_ok());
}

#[tokio::test]
async fn tools_are_off_until_the_user_turns_them_on() {
    let (_dir, app) = app();
    // Enabled by default in automatic mode; off by default in manual mode.
    assert!(!enabled(&app));
    set(&app, &json!({"on": false})).unwrap();
    let mut a = Args::new();
    a.insert("browser".into(), json!("own"));
    a.insert("what".into(), json!("look"));
    assert_eq!(call(&app, READ, &a).await.unwrap().unwrap_err(), OFF_TEXT);
    assert!(call(&app, "other", &a).await.is_none());
    set(&app, &json!({"on": true})).unwrap();
    assert!(enabled(&app));
    let blocked = call(&app, READ, &a).await.unwrap().unwrap_err();
    assert!(blocked.contains("Private and confidential"), "{blocked}");
    a.insert("what".into(), json!("click"));
    assert!(call(&app, READ, &a).await.unwrap().unwrap_err().contains("browser_step"));
    a.insert("browser".into(), json!("other"));
    assert!(call(&app, READ, &a).await.unwrap().is_err());
    let calls = view(&app)["calls"].as_array().unwrap().len();
    assert_eq!(calls, 1);
    assert!(set(&app, &json!({"command": "bad\nline"})).is_err());
    set(&app, &json!({"command": "/opt/tools/playwright-mcp"})).unwrap();
    if std::env::var(COMMAND_ENV).is_err() {
        assert_eq!(command_text(&app), "/opt/tools/playwright-mcp");
    }
}

#[tokio::test]
async fn steps_are_refused_with_bad_args() {
    let (_dir, app) = app();
    set(&app, &json!({"on": true})).unwrap();
    crate::settings_api::set_privacy(&app, false).unwrap();
    let mut a = Args::new();
    a.insert("browser".into(), json!("own"));
    a.insert("action".into(), json!("click"));
    a.insert("selector".into(), json!("#buy"));
    let out = call(&app, STEP, &a).await.unwrap().unwrap_err();
    assert!(out.contains("Unsupported browser argument"), "{out}");
}

const FAKE: &str = r#"import sys, json
url = "https://example.com/start"
for line in sys.stdin:
    m = json.loads(line)
    if "id" not in m:
        continue
    if m.get("method") == "initialize":
        r = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}}, "serverInfo": {"name": "fake", "version": "1"}}
    elif m.get("method") == "tools/call":
        n = m["params"]["name"]
        a = m["params"]["arguments"]
        if n == "browser_snapshot":
            t = "- Page URL: " + url + "\n- link \"More docs\" [ref=e2]:\n  - /url: /docs\n- textbox \"Search\" [ref=e4]\n"
        elif n == "browser_tabs" and a.get("action") == "list":
            t = "- 0: (current) [Start](" + url + ")\n- 1: [New Tab](about:blank)"
        elif n == "browser_tabs":
            t = "tab " + a.get("action", "") + " " + str(a.get("index"))
        elif n == "browser_click":
            url = "https://example.com/docs"
            t = "clicked " + a.get("target", "")
        else:
            t = n
        r = {"content": [{"type": "text", "text": t}]}
    else:
        r = {}
    print(json.dumps({"jsonrpc": "2.0", "id": m["id"], "result": r}), flush=True)
"#;

#[tokio::test]
async fn a_full_step_reads_and_runs_against_a_fake_browser_tool() {
    if super::mcp::find_program("python3").is_none() || std::env::var(COMMAND_ENV).is_ok() {
        return;
    }
    let (dir, app) = app();
    let script = dir.path().join("fake.py");
    std::fs::write(&script, FAKE).unwrap();
    set(&app, &json!({"on": true, "command": format!("python3 {}", script.display())})).unwrap();
    crate::settings_api::set_privacy(&app, false).unwrap();
    let mut a = Args::new();
    a.insert("browser".into(), json!("own"));
    a.insert("what".into(), json!("look"));
    let look = call(&app, READ, &a).await.unwrap().unwrap();
    assert!(look.starts_with(super::step::RESULT_HEAD), "{look}");
    assert!(look.contains("More docs"));
    let mut s = Args::new();
    s.insert("browser".into(), json!("own"));
    s.insert("action".into(), json!("click"));
    s.insert("ref".into(), json!("e2"));
    let held = call(&app, STEP, &s).await.unwrap().unwrap_err();
    assert!(held.contains("Held for the user's yes"), "{held}");
    assert!(!lock(&app.browser.info).calls.iter().any(|c| c.action == "click" && c.approved));
    let id = lock(&app.desktop.held).as_ref().map(|h| h.id.clone()).unwrap();
    crate::desktop::gate::answer(&app, &id, true).unwrap();
    let done = call(&app, STEP, &s).await.unwrap().unwrap();
    assert!(done.contains("clicked e2"), "{done}");
    assert_eq!(lock(&app.browser.info).histories[&Which::Own].last().map(String::as_str), Some("https://example.com/docs"));
    let calls: Vec<String> = lock(&app.browser.info).calls.iter().map(|c| format!("{} {}", c.action, c.verdict)).collect();
    assert!(calls.contains(&"click ran".to_string()), "{calls:?}");
    assert!(lock(&app.browser.info).calls.iter().any(|c| c.action == "click" && c.approved));
    let mut t = Args::new();
    t.insert("browser".into(), json!("own"));
    t.insert("action".into(), json!("tab_close"));
    t.insert("index".into(), json!(1));
    assert!(call(&app, STEP, &t).await.unwrap().unwrap_err().contains("Held for the user's yes"));
    let id = lock(&app.desktop.held).as_ref().map(|h| h.id.clone()).unwrap();
    crate::desktop::gate::answer(&app, &id, true).unwrap();
    let closed = call(&app, STEP, &t).await.unwrap().unwrap();
    assert!(closed.contains("tab close 1"), "{closed}");
    t.insert("url".into(), json!("https://example.com/"));
    assert!(call(&app, STEP, &t).await.unwrap().unwrap_err().contains("only the tab index"));
    let dirs: Vec<std::path::PathBuf> = app.browser.sessions.lock().await.values().map(|o| o.session.dir.clone()).collect();
    assert_eq!(dirs.len(), 1);
    super::step::close_all(&app).await;
    stop(&app);
    assert!(app.browser.sessions.try_lock().is_ok_and(|s| s.is_empty()));
    assert!(dirs.iter().all(|d| !d.exists()));
}

#[test]
fn the_watch_stops_idle_or_blocked_sessions() {
    let (_dir, app) = app();
    assert!(should_stop(&app, now()));
    set(&app, &json!({"on": true})).unwrap();
    lock(&app.browser.info).last_used = now();
    assert!(should_stop(&app, now()));
}

fn now() -> u64 {
    crate::util::now_ms()
}
