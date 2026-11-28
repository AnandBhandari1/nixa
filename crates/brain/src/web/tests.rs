use super::*;
use crate::app::App;
use axum::routing::post;
use std::net::IpAddr;

fn args(value: Value) -> Args {
    value.as_object().cloned().unwrap_or_default()
}

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

async fn serve(router: Router) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    port
}

async fn free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

#[test]
fn only_short_public_text_goes_out() {
    assert!(safety::public_text("rust 1.90 release date").is_ok());
    assert!(safety::public_text("weather in a big city today").is_ok());
    for bad in ["", "   ", "my password reset", "line\nbreak", "see /Users/someone/x", "{\"a\":1}", "card 4111 1111 1111 1111", "mail a.b@example.com", "the api key for x", "our client list", "use `ls`", "file:///etc/hosts"] {
        assert_eq!(safety::public_text(bad), Err(safety::PUBLIC_ONLY.to_string()), "{bad}");
    }
    assert!(safety::public_text(&"a".repeat(240)).is_ok());
    assert!(safety::public_text(&"a".repeat(241)).is_err());
    assert!(safety::public_text("tokens per second of a model").is_ok());
}

#[test]
fn only_public_addresses_pass() {
    for good in ["8.8.8.8", "1.1.1.1", "2606:4700::1111", "2a00:1450::1"] {
        assert!(safety::public_address(ip(good)), "{good}");
    }
    for bad in ["0.0.0.0", "10.1.2.3", "127.0.0.1", "100.64.0.1", "169.254.1.1", "172.16.0.1", "172.31.9.9", "192.168.1.1", "192.0.2.1", "192.88.99.1", "198.18.0.1", "198.51.100.1", "203.0.113.1", "224.0.0.1", "255.255.255.255", "::1", "::", "fe80::1", "fd00::1", "::ffff:8.8.8.8", "2001:db8::1", "2001:0::1", "2002::1", "3fff::1", "64:ff9b::808:808"] {
        assert!(!safety::public_address(ip(bad)), "{bad}");
    }
    assert!(safety::public_address(ip("172.15.0.1")));
    for good in ["192.0.43.8", "192.0.1.1", "192.0.3.1", "192.88.98.1", "192.88.100.1", "198.17.255.1", "198.20.0.1", "198.51.99.1", "198.51.101.1", "203.0.112.1", "203.0.114.1", "192.169.0.1", "172.32.0.1"] {
        assert!(safety::public_address(ip(good)), "{good}");
    }
    for bad in ["192.0.0.1", "192.0.0.255", "192.0.2.0", "192.0.2.255", "192.88.99.0", "192.88.99.255", "198.18.0.0", "198.19.255.255", "198.51.100.0", "198.51.100.255", "203.0.113.0", "203.0.113.255"] {
        assert!(!safety::public_address(ip(bad)), "{bad}");
    }
    assert!(safety::public_url("https://192.0.43.8/").is_ok());
    assert!(safety::public_address(ip("2001:4860::8888")));
}

#[test]
fn only_public_pages_on_standard_ports() {
    assert_eq!(safety::public_url("https://example.com/docs/page").unwrap().as_str(), "https://example.com/docs/page");
    assert!(safety::public_url("https://example.com:443/a").is_ok());
    assert!(safety::public_url("http://example.com:80/a").is_ok());
    assert!(safety::public_url("https://8.8.8.8/").is_ok());
    for bad in ["ftp://example.com/", "https://example.com:8080/", "https://u:p@example.com/", "https://intranet/", "http://127.0.0.1/", "http://10.0.0.1/", "http://[::1]/", "file:///etc/hosts", "not a url", "https://example.com/Users/x", "https://example.com/?q=%2FUsers%2Fsomeone", "https://example.com/my/page"] {
        assert!(safety::public_url(bad).is_err(), "{bad}");
    }
    for blocked in ["https://printer.lan/", "https://box.local./", "https://localhost./", "https://app.internal/x", "https://a.test/"] {
        assert_eq!(safety::public_url(blocked).unwrap_err(), safety::PRIVATE_ADDRESS, "{blocked}");
    }
    assert_eq!(safety::percent_decode("a%20b%2Fc%zz%4"), "a b/c%zz%4");
}

#[tokio::test]
async fn lookups_to_local_addresses_are_blocked() {
    assert_eq!(safety::public_lookup("localhost", 80).await.unwrap_err(), safety::PRIVATE_ADDRESS);
    assert_eq!(safety::public_lookup("127.0.0.1", 443).await.unwrap_err(), safety::PRIVATE_ADDRESS);
    assert!(safety::public_lookup("8.8.8.8", 443).await.is_ok());
}

#[test]
fn exa_body_and_hits() {
    let body = exa::body("q", 50);
    assert_eq!(body["numResults"], 10);
    assert_eq!(exa::body("q", 0)["numResults"], 1);
    assert_eq!(body["type"], "auto");
    assert_eq!(body["contents"]["highlights"]["maxCharacters"], 600);
    let data = json!({"results": [
        {"title": "A", "url": "https://a.example", "publishedDate": "2026-01-02T03:04:05Z", "highlights": ["one  two", "three\nfour"]},
        {"url": "https://b.example", "text": "x".repeat(900)},
        {"url": "http:///go/v1/abc", "title": "no host"},
        {}
    ]});
    let hits = exa::hits(&data);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0], Hit { title: "A".into(), url: "https://a.example".into(), date: "2026-01-02".into(), text: "one two … three four".into() });
    assert_eq!(hits[1].title, "https://b.example");
    assert_eq!(hits[1].text.chars().count(), 700);
    assert!(exa::hits(&json!({})).is_empty());
    assert_eq!(exa::status_error(401), "Exa refused the key.");
    assert_eq!(exa::status_error(500), "Exa search failed (500).");
}

#[tokio::test]
async fn exa_search_sends_the_key_and_reads_hits() {
    let router = Router::new().route(
        "/search",
        post(|headers: HeaderMap, axum::Json(body): axum::Json<Value>| async move {
            if headers.get("x-api-key").and_then(|v| v.to_str().ok()) != Some("good") {
                return (StatusCode::UNAUTHORIZED, axum::Json(json!({}))).into_response();
            }
            axum::Json(json!({"results": [{"title": body["query"], "url": "https://a.example", "highlights": ["hi"]}]})).into_response()
        }),
    );
    let port = serve(router).await;
    let url = format!("http://127.0.0.1:{port}/search");
    let http = reqwest::Client::new();
    let hits = exa::search(&http, &url, "good", "rust news", 3).await.unwrap();
    assert_eq!(hits[0].title, "rust news");
    assert_eq!(exa::search(&http, &url, "bad", "rust news", 3).await.unwrap_err(), "Exa refused the key.");
    let gone = free_port().await;
    assert_eq!(exa::search(&http, &format!("http://127.0.0.1:{gone}/search"), "good", "x", 1).await.unwrap_err(), "Exa could not be reached.");
}

#[test]
fn searxng_hits_skip_empty_urls_and_keep_count() {
    let data = json!({"results": [
        {"title": "", "url": "https://a.example", "content": "a  b", "publishedDate": null},
        {"title": "no url"},
        {"title": "no host", "url": "http:///go/v1/abc"},
        {"title": "not web", "url": "javascript:alert(1)"},
        {"title": "C", "url": "https://c.example", "content": "c", "publishedDate": "2025-05-06T00:00:00"},
        {"title": "D", "url": "https://d.example"}
    ]});
    let hits = searxng::hits(&data, 2);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0], Hit { title: "https://a.example".into(), url: "https://a.example".into(), date: String::new(), text: "a b".into() });
    assert_eq!(hits[1].date, "2025-05-06");
}

fn fake_searxng(json_on: bool) -> Router {
    Router::new().route("/healthz", get(|| async { "OK" })).route(
        "/search",
        get(move |axum::extract::Query(q): axum::extract::Query<std::collections::HashMap<String, String>>| async move {
            if !json_on || q.get("format").map(String::as_str) != Some("json") {
                return (StatusCode::FORBIDDEN, "no").into_response();
            }
            let query = q.get("q").cloned().unwrap_or_default();
            axum::Json(json!({"results": [{"title": query, "url": "https://a.example", "content": "write to help@example.com"}]})).into_response()
        }),
    )
}

#[tokio::test]
async fn searxng_search_reads_json_and_explains_json_off() {
    let port = serve(fake_searxng(true)).await;
    let client = searxng::local_client().unwrap();
    assert!(searxng::healthy(&client, port).await);
    let hits = searxng::search(&client, port, "rust release", 5).await.unwrap();
    assert_eq!(hits[0].title, "rust release");
    let off = serve(fake_searxng(false)).await;
    assert_eq!(searxng::search(&client, off, "rust", 5).await.unwrap_err(), searxng::JSON_OFF);
    assert!(!searxng::healthy(&client, free_port().await).await);
}

fn setup_at(folder: &std::path::Path, port: u16) -> setup::Setup {
    setup::Setup {
        on: true,
        provider: Provider::Searxng,
        folder: folder.join("src"),
        settings: folder.join("settings.yml"),
        port,
        folder_set: true,
        settings_set: true,
    }
}

#[tokio::test]
async fn ensure_uses_a_running_search_and_does_not_own_it() {
    let dir = tempfile::tempdir().unwrap();
    let port = serve(fake_searxng(true)).await;
    let keeper = searxng::Keeper::default();
    searxng::ensure(&keeper, &setup_at(dir.path(), port), "s").await.unwrap();
    assert!(!searxng::running(&keeper));
    assert!(*crate::app::lock(&keeper.used_at) > 0);
}

#[tokio::test]
async fn ensure_says_when_searxng_is_missing_or_fails() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port().await;
    let keeper = searxng::Keeper::default();
    let setup = setup_at(dir.path(), port);
    assert_eq!(searxng::ensure(&keeper, &setup, "s").await.unwrap_err(), searxng::NOT_INSTALLED);
    let command = searxng::command(&setup.folder);
    std::fs::create_dir_all(command.parent().unwrap()).unwrap();
    std::fs::write(&command, "#!/bin/sh\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(&setup.settings, "x").unwrap();
    assert!(searxng::installed(&setup));
    let started = std::time::Instant::now();
    assert_eq!(searxng::ensure(&keeper, &setup, "s").await.unwrap_err(), searxng::NOT_HEALTHY);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(!searxng::running(&keeper));
}

#[tokio::test]
async fn idle_searxng_is_stopped_with_its_group() {
    let keeper = searxng::Keeper::default();
    let child = tokio::process::Command::new("/bin/sleep").arg("30").process_group(0).kill_on_drop(true).spawn().unwrap();
    let pid = child.id().unwrap();
    *crate::app::lock(&keeper.running) = Some(searxng::Running { child, port: 1 });
    searxng::touch(&keeper);
    let now = crate::util::now_ms();
    assert!(!searxng::idle_stop(&keeper, now));
    assert!(searxng::running(&keeper));
    assert!(searxng::idle_stop(&keeper, now + searxng::IDLE_MS + 1));
    assert!(!searxng::running(&keeper));
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let alive = std::process::Command::new("/bin/kill").args(["-0", &pid.to_string()]).stderr(std::process::Stdio::null()).status().unwrap().success();
    assert!(!alive);
}

fn sleeper() -> (tokio::process::Child, u32) {
    let child = tokio::process::Command::new("/bin/sleep").arg("30").process_group(0).kill_on_drop(true).spawn().unwrap();
    let pid = child.id().unwrap();
    (child, pid)
}

#[test]
fn searxng_restarts_when_its_setup_changes() {
    let dir = tempfile::tempdir().unwrap();
    let before = setup_at(dir.path(), 8888);
    assert!(!searxng::needs_restart(&before, &before.clone()));
    let changes = [
        setup::Setup { on: false, ..before.clone() },
        setup::Setup { provider: Provider::Exa, ..before.clone() },
        setup::Setup { port: 8899, ..before.clone() },
        setup::Setup { folder: dir.path().join("other"), ..before.clone() },
        setup::Setup { settings: dir.path().join("other.yml"), ..before.clone() },
    ];
    for after in changes {
        assert!(searxng::needs_restart(&before, &after), "{after:?}");
    }
}

#[tokio::test]
async fn set_web_stops_an_owned_searxng_on_a_new_setup_only() {
    let (_dir, app) = app();
    setup::change(&app, &json!({"on": true, "provider": "searxng"})).unwrap();
    let (child, _) = sleeper();
    *crate::app::lock(&app.web.searxng.running) = Some(searxng::Running { child, port: 8888 });
    assert!(ws(&app, "set_web", &json!({"on": true})).unwrap().is_ok());
    assert!(searxng::running(&app.web.searxng));
    assert!(ws(&app, "set_web", &json!({"settings": "/opt/other/settings.yml"})).unwrap().is_ok());
    assert!(!searxng::running(&app.web.searxng));
    let (child, _) = sleeper();
    *crate::app::lock(&app.web.searxng.running) = Some(searxng::Running { child, port: 8888 });
    assert!(act(&app, "set_web", &json!({"folder": "/opt/other/src"})).await.unwrap().is_ok());
    assert!(!searxng::running(&app.web.searxng));
    let (child, _) = sleeper();
    *crate::app::lock(&app.web.searxng.running) = Some(searxng::Running { child, port: 8888 });
    assert!(act(&app, "set_web", &json!({"port": 8899})).await.unwrap().is_ok());
    assert!(!searxng::running(&app.web.searxng));
}

#[test]
fn safe_clients_are_built() {
    assert!(searxng::local_client().is_ok());
    assert!(read::client_for("example.com", "93.184.215.14:443".parse().unwrap()).is_ok());
    assert!(read::client_for("8.8.8.8", "8.8.8.8:443".parse().unwrap()).is_ok());
}

#[test]
fn the_searxng_secret_is_private_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let first = searxng::secret(dir.path()).unwrap();
    assert_eq!(first.len(), 64);
    assert_eq!(searxng::secret(dir.path()).unwrap(), first);
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(dir.path().join(searxng::SECRET_FILE)).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn html_becomes_plain_text() {
    let html = "<!doctype html><html><head><title>The &amp; Page</title><style>x{}</style></head><body><!-- note --><script>alert(1)</script><h1>Hello</h1><p>One&nbsp;two &lt;three&gt; &#65;&#x42; &bogus;</p><ul><li>a</li><li>b</li></ul><noscript>no</noscript></body></html>";
    assert_eq!(page::title(html).as_deref(), Some("The & Page"));
    assert_eq!(page::html_text(html), "Hello\n\nOne two <three> AB &bogus;\n\na\n\nb");
    assert!(page::is_html("text/html; charset=utf-8", ""));
    assert!(page::is_html("", "<!DOCTYPE html><p>"));
    assert!(!page::is_html("text/plain", "<html>"));
    assert!(page::readable("application/json"));
    assert!(!page::readable("image/png"));
    assert_eq!(page::window("abcdef", 2, 3), ("cde".to_string(), Some(5)));
    assert_eq!(page::window("abcdef", 4, 9), ("ef".to_string(), None));
    assert_eq!(page::window("ab", 9, 3), (String::new(), None));
}

#[test]
fn hits_and_pages_are_formatted() {
    assert_eq!(format_hits(&[]), "No web results.");
    let hits = vec![Hit { title: "A".into(), url: "https://a.example".into(), date: "2026-01-02".into(), text: "t".into() }, Hit { title: "B".into(), url: "https://b.example".into(), date: String::new(), text: "u".into() }];
    assert_eq!(format_hits(&hits), "1. A (2026-01-02)\nhttps://a.example\nt\n\n2. B\nhttps://b.example\nu");
    let found = read::Page { url: "https://a.example/".into(), title: Some("T".into()), text: "abcdef".into() };
    assert_eq!(page_text(&found, 0, 4), "URL: https://a.example/\nTitle: T\n\nabcd\n\n[More text: read again with start_char 4.]");
    assert_eq!(page_text(&found, 9, 4), "URL: https://a.example/\nTitle: T\n\n(no text)");
    let wrapped = wrap(SEARCH_HEAD, &format!("x {}", "y".repeat(RESULT_CHARS)));
    assert!(wrapped.starts_with(SEARCH_HEAD));
    assert_eq!(wrapped.chars().count(), RESULT_CHARS);
}

#[tokio::test]
async fn one_page_step_follows_moves_and_reads_text() {
    let router = Router::new()
        .route("/old", get(|| async { (StatusCode::FOUND, [("location", "/new?x=1")], "") }))
        .route("/new", get(|| async { ([("content-type", "text/html")], "<title>New</title><p>Body text</p>") }))
        .route("/plain", get(|| async { ([("content-type", "text/plain")], "  a  \n\n\n\n b ") }))
        .route("/img", get(|| async { ([("content-type", "image/png")], "x") }))
        .route("/gone", get(|| async { StatusCode::NOT_FOUND }))
        .route("/big", get(|| async { ([("content-type", "text/plain")], "z".repeat(read::MAX_BYTES + 100)) }));
    let port = serve(router).await;
    let base = format!("http://127.0.0.1:{port}");
    let client = searxng::local_client().unwrap();
    let url = |p: &str| reqwest::Url::parse(&format!("{base}{p}")).unwrap();
    match read::get_once(&client, &url("/old")).await.unwrap() {
        read::Step::Moved(next) => assert_eq!(next.as_str(), format!("{base}/new?x=1")),
        other => panic!("{other:?}"),
    }
    match read::get_once(&client, &url("/new")).await.unwrap() {
        read::Step::Done(found) => {
            assert_eq!(found.title.as_deref(), Some("New"));
            assert_eq!(found.text, "New\nBody text");
        }
        other => panic!("{other:?}"),
    }
    match read::get_once(&client, &url("/plain")).await.unwrap() {
        read::Step::Done(found) => assert_eq!(found.text, "a\n\nb"),
        other => panic!("{other:?}"),
    }
    match read::get_once(&client, &url("/big")).await.unwrap() {
        read::Step::Done(found) => assert_eq!(found.text.len(), read::MAX_BYTES),
        other => panic!("{other:?}"),
    }
    assert_eq!(read::get_once(&client, &url("/img")).await.unwrap_err(), "The page is not text (image/png).");
    assert_eq!(read::get_once(&client, &url("/gone")).await.unwrap_err(), "The page answered 404.");
    assert_eq!(read::fetch(&format!("{base}/new")).await.unwrap_err(), safety::PUBLIC_PAGE);
}

#[test]
fn settings_change_and_load() {
    let (dir, app) = app();
    let home = std::path::Path::new("/Users/someone");
    let conn = nixa_store::open(&app.db_file).unwrap();
    let first = setup::load_with(&conn, home);
    assert!(!first.on);
    assert!(!setup::load_with_defaults(home).on);
    assert_eq!(first.provider, Provider::Searxng);
    assert_eq!(first.folder, home.join(setup::DEFAULT_FOLDER));
    assert_eq!(first.settings, home.join(setup::DEFAULT_SETTINGS));
    assert_eq!(first.port, setup::DEFAULT_PORT);
    setup::change_with(&conn, &json!({"on": true, "provider": "exa", "port": 9999, "folder": "~/sx", "settings": "/opt/sx/settings.yml"})).unwrap();
    let next = setup::load_with(&conn, home);
    assert!(next.on);
    assert_eq!(next.provider, Provider::Exa);
    assert_eq!(next.port, 9999);
    assert_eq!(next.folder, home.join("sx"));
    assert_eq!(next.settings, std::path::PathBuf::from("/opt/sx/settings.yml"));
    for bad in [json!({"on": "yes"}), json!({"provider": "other"}), json!({"port": 80}), json!({"port": 7421}), json!({"port": 70000}), json!({"folder": "rel/path"}), json!({"folder": "/a/../b"}), json!({"settings": 5})] {
        assert!(setup::change_with(&conn, &bad).is_err(), "{bad}");
    }
    assert_eq!(setup::load_with(&conn, home), next);
    setup::change_with(&conn, &json!({"folder": ""})).unwrap();
    assert_eq!(setup::load_with(&conn, home).folder, home.join(setup::DEFAULT_FOLDER));
    let view = setup::view_of(&next, false, false, false, home);
    assert_eq!(view["searxng"]["settings"], "/opt/sx/settings.yml");
    assert_eq!(setup::view_of(&first, true, true, false, home)["searxng"]["folder"], format!("~/{}", setup::DEFAULT_FOLDER));
    drop(dir);
}

#[tokio::test]
async fn explicit_opt_out_disables_both_web_tools() {
    let (_dir, app) = app();
    setup::change(&app, &json!({"on": false})).unwrap();
    assert_eq!(call(&app, SEARCH, &args(json!({"query": "rust news"}))).await.unwrap().unwrap_err(), OFF);
    assert_eq!(call(&app, READ, &args(json!({"url": "https://example.com/"}))).await.unwrap().unwrap_err(), OFF);
    assert!(call(&app, "other", &args(json!({}))).await.is_none());
    assert_eq!(specs().iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), NAMES.to_vec());
}

#[tokio::test]
async fn search_goes_to_local_searxng_and_hides_private_bits() {
    let (_dir, app) = app();
    let port = serve(fake_searxng(true)).await;
    setup::change(&app, &json!({"port": port, "on": true})).unwrap();
    crate::settings_api::set_privacy(&app, false).unwrap();
    assert_eq!(search(&app, &args(json!({"query": "/Users/x/repo"}))).await.unwrap_err(), safety::PUBLIC_ONLY);
    assert_eq!(search(&app, &args(json!({"query": "rust Vec<String> token"}))).await.unwrap_err(), safety::PUBLIC_ONLY);
    assert_eq!(search(&app, &args(json!({"query": format!("sk-ant-{}", "x".repeat(20))}))).await.unwrap_err(), safety::PUBLIC_ONLY);
    let local = read_page(&app, &args(json!({"url": format!("http://127.0.0.1:{port}/healthz")}))).await.unwrap_err();
    assert_eq!(local, safety::PUBLIC_PAGE);
    let out = search(&app, &args(json!({"query": "rust release", "count": 3}))).await.unwrap();
    assert!(out.starts_with(SEARCH_HEAD), "{out}");
    assert!(out.contains("Provider: local SearXNG\n1. rust release\nhttps://a.example\nwrite to <email>"), "{out}");
}

#[tokio::test]
#[ignore]
async fn live_local_searxng_and_page_read() {
    let setup = setup::Setup { on: true, port: 8898, ..setup::load_with_defaults(&setup::home()) };
    let keeper = searxng::Keeper::default();
    let dir = tempfile::tempdir().unwrap();
    let secret = searxng::secret(dir.path()).unwrap();
    let started = std::time::Instant::now();
    let ready = searxng::ensure(&keeper, &setup, &secret).await;
    eprintln!("ensure: {ready:?} after {:?}, owned: {}", started.elapsed(), searxng::running(&keeper));
    let hits = searxng::search(&searxng::local_client().unwrap(), setup.port, "rust programming language", 3).await;
    eprintln!("{}", hits.as_ref().map(|h| format_hits(h)).unwrap_or_else(|e| e.clone()));
    let page = read::fetch("https://example.com/").await;
    eprintln!("{}", page.as_ref().map(|p| page_text(p, 0, 300)).unwrap_or_else(|e| e.clone()));
    assert!(searxng::stop(&keeper));
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(!searxng::healthy(&searxng::local_client().unwrap(), setup.port).await);
    ready.unwrap();
    assert!(!hits.unwrap().is_empty());
    assert_eq!(page.unwrap().title.as_deref(), Some("Example Domain"));
}

#[tokio::test]
async fn window_messages_and_routes() {
    let (_dir, app) = app();
    assert!(ws(&app, "say", &json!({})).is_none());
    assert!(ws(&app, "set_web", &json!({"provider": "nope"})).unwrap().is_err());
    assert!(ws(&app, "set_web", &json!({"on": true})).unwrap().is_ok());
    assert!(setup::load(&app).on);
    assert!(ws(&app, "web_key", &json!({"key": "a b"})).unwrap().is_err());
    let port = serve(routes().with_state(app.clone())).await;
    let http = reqwest::Client::new();
    let base = format!("http://127.0.0.1:{port}/web");
    assert_eq!(http.get(&base).send().await.unwrap().status(), 401);
    assert_eq!(http.post(&base).json(&json!({"type": "set_web", "on": false})).send().await.unwrap().status(), 401);
    let token = crate::window_key::HEADER;
    let bad = http.post(&base).header(token, &app.window_token).json(&json!({"type": "nope"})).send().await.unwrap();
    assert_eq!(bad.status(), 400);
    let done: Value = http.post(&base).header(token, &app.window_token).json(&json!({"type": "set_web", "on": false, "provider": "exa"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(done["ok"], true);
    assert_eq!(done["web"]["on"], false);
    assert_eq!(done["web"]["provider"], "exa");
}

#[tokio::test]
async fn web_tools_are_off_on_a_fresh_app_and_safe_when_on() {
    let (_dir, app) = app();
    assert!(!setup::load(&app).on);
    assert_eq!(read_page(&app, &args(json!({"url": "https://example.com/"}))).await.unwrap_err(), OFF);
    setup::change(&app, &json!({"on": true})).unwrap();
    crate::settings_api::set_privacy(&app, true).unwrap();
    assert_eq!(search(&app, &args(json!({"query": "rust news"}))).await.unwrap_err(), PRIVATE);
    assert_eq!(read_page(&app, &args(json!({"url": "https://example.com/"}))).await.unwrap_err(), PRIVATE);
    crate::settings_api::set_privacy(&app, false).unwrap();
    for url in ["http://127.0.0.1:7421/health", "http://127.0.0.1:7421/settings", "http://192.168.1.20/", "http://localhost/"] {
        let refused = read_page(&app, &args(json!({"url": url}))).await.unwrap_err();
        assert!(refused == safety::PUBLIC_PAGE || refused == safety::PRIVATE_ADDRESS, "{url}: {refused}");
    }
    assert_eq!(search(&app, &args(json!({"query": "/Users/x/repo"}))).await.unwrap_err(), safety::PUBLIC_ONLY);
}
