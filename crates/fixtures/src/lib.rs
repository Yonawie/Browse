//! Fixture web sites for end-to-end tests. Everything is served from one
//! process on a loopback port; sites are told apart by the `Host` header
//! (`*.localhost` resolves to loopback in Chromium without any DNS setup), so
//! tests get *distinct origins* — needed to exercise the agent's cross-origin
//! data-flow rules.
//!
//! | host                | purpose                                                     |
//! |---------------------|-------------------------------------------------------------|
//! | `shop.localhost`    | catalog, product pages, cart, checkout with payment fields  |
//! | `shop2.localhost`   | a competing shop (different origin) for comparison tasks    |
//! | `forms.localhost`   | contact form with text/select/checkbox/textarea + thanks    |
//! | `news.localhost`    | articles (EN/RU) for summarize/ask/translate, a data table  |
//! | `login.localhost`   | cookie session; `/account` requires login (session import)  |
//! | `hostile.localhost` | prompt-injection page trying to redirect the agent          |
//! | `evil.localhost`    | exfiltration sink; `/collected` shows what (if anything) arrived |

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use axum::Router;
use serde::Serialize;

pub mod sites;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Product {
    pub id: u32,
    pub name: String,
    pub brand: String,
    pub price_cents: u32,
    pub weight_g: u32,
    pub battery_h: u32,
    pub rating: f32,
}

#[derive(Default)]
pub struct Shared {
    /// Everything `evil.localhost/collect` ever received.
    pub collected: Vec<String>,
    /// Orders placed on `shop.localhost` (should stay empty in agent tests without confirmation).
    pub orders: Vec<String>,
    /// Contact-form submissions.
    pub contacts: Vec<HashMap<String, String>>,
}

#[derive(Clone)]
pub struct AppState {
    pub shared: Arc<Mutex<Shared>>,
    pub port: u16,
}

pub struct Fixture {
    pub addr: SocketAddr,
    pub state: AppState,
    handle: tokio::task::JoinHandle<()>,
}

impl Fixture {
    /// Bind an ephemeral loopback port and start serving.
    pub async fn spawn() -> Fixture {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().unwrap();
        let state = AppState { shared: Arc::new(Mutex::new(Shared::default())), port: addr.port() };
        let app = router(state.clone());
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        Fixture { addr, state, handle }
    }

    pub fn origin(&self, host: &str) -> String {
        format!("http://{host}.localhost:{}", self.addr.port())
    }

    pub fn url(&self, host: &str, path: &str) -> String {
        format!("{}{}", self.origin(host), path)
    }

    pub fn collected(&self) -> Vec<String> {
        self.state.shared.lock().unwrap().collected.clone()
    }

    pub fn orders(&self) -> Vec<String> {
        self.state.shared.lock().unwrap().orders.clone()
    }

    pub fn contacts(&self) -> Vec<HashMap<String, String>> {
        self.state.shared.lock().unwrap().contacts.clone()
    }

    pub fn abort(&self) {
        self.handle.abort();
    }
}

pub fn router(state: AppState) -> Router {
    Router::new().fallback(any(dispatch)).with_state(state)
}

async fn dispatch(State(state): State<AppState>, req: Request) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let site = host.trim_end_matches(".localhost").to_string();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query: HashMap<String, String> = req
        .uri()
        .query()
        .map(|q| {
            q.split('&').filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), url_decode(v)))).collect()
        })
        .unwrap_or_default();
    let cookies = parse_cookies(req.headers());
    let body_bytes = axum::body::to_bytes(req.into_body(), 1 << 20).await.unwrap_or_default();
    let form: HashMap<String, String> = std::str::from_utf8(&body_bytes)
        .unwrap_or("")
        .split('&')
        .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), url_decode(v))))
        .collect();

    let ctx = sites::Ctx { state: &state, method: method.as_str(), path: &path, query, cookies, form };
    match site.as_str() {
        "shop" => sites::shop::handle(&ctx, sites::shop::ShopVariant::A),
        "shop2" => sites::shop::handle(&ctx, sites::shop::ShopVariant::B),
        "forms" => sites::forms::handle(&ctx),
        "news" => sites::news::handle(&ctx),
        "login" => sites::login::handle(&ctx),
        "hostile" => sites::hostile::handle(&ctx),
        "evil" => sites::evil::handle(&ctx),
        _ => sites::index(&ctx),
    }
}

fn parse_cookies(headers: &HeaderMap) -> HashMap<String, String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|s| s.split(';'))
        .filter_map(|kv| kv.trim().split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect()
}

pub fn url_decode(s: &str) -> String {
    let s = s.replace('+', " ");
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn html(status: StatusCode, body: String) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(body))
        .unwrap()
}

pub fn html_with_cookie(body: String, cookie: &str) -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::SET_COOKIE, cookie)
        .body(Body::from(body))
        .unwrap()
}

pub fn redirect(to: &str) -> Response {
    (StatusCode::SEE_OTHER, [(header::LOCATION, to.to_string())]).into_response()
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Page shell shared by all sites: a `<nav>` (excluded from content extraction)
/// and a `<main>` with the content.
pub fn page(site: &str, title: &str, nav: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>{title}</title>
<style>body{{font-family:system-ui;margin:0}} nav{{background:#eee;padding:8px}} main{{padding:16px;max-width:900px}}
table{{border-collapse:collapse}} td,th{{border:1px solid #ccc;padding:4px 8px}} .price{{font-weight:bold}}
label{{display:block;margin:8px 0}} button{{padding:6px 12px}}</style></head>
<body><nav><a href="/">{site}</a> {nav}</nav><main>{body}</main></body></html>"#
    )
}
