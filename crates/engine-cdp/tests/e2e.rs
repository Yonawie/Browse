//! End-to-end tests: real Chromium (headless) driven over CDP against the local
//! fixture sites. No network access is required. Tests skip themselves when no
//! Chromium/Chrome/Edge binary is installed (set `BROWSE_CHROME` to point at one).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use core_types::{ElementRef, Observation, Origin};
use engine_adapter::{Action, EngineAdapter, EngineError, EngineEvent, ProfileKind, WebViewOptions};
use engine_cdp::launcher::{find_browser, LaunchOptions};
use engine_cdp::CdpEngine;
use fixtures::Fixture;

/// One browser and one fixture server are shared by all tests (each test gets
/// its own webview), which mirrors production use and keeps the suite fast.
/// The connection's reader/writer tasks must outlive individual tests, so they
/// run on a process-wide runtime instead of per-test runtimes.
struct Harness {
    engine: Arc<CdpEngine>,
    fx: Fixture,
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap())
}

fn harness() -> Option<&'static Harness> {
    static H: OnceLock<Option<Harness>> = OnceLock::new();
    H.get_or_init(|| {
        if find_browser().is_none() {
            eprintln!("skipping: no Chromium-based browser found (set BROWSE_CHROME)");
            return None;
        }
        runtime().block_on(async {
            let fx = Fixture::spawn().await;
            let engine = CdpEngine::launch(LaunchOptions::headless()).await.expect("launch chromium");
            Some(Harness { engine, fx })
        })
    })
    .as_ref()
}

/// Runs an async test body on the shared runtime, skipping when no browser is available.
macro_rules! e2e {
    ($name:ident, |$h:ident| $body:block) => {
        #[test]
        fn $name() {
            let Some($h) = harness() else { return };
            runtime().block_on(async move { $body });
        }
    };
}

fn user_opts() -> WebViewOptions {
    WebViewOptions { profile: ProfileKind::User, inject_sensor: true, observe_iframe_origins: vec![], headless: true }
}

fn find<'a>(obs: &'a Observation, role: &str, name_part: &str) -> &'a core_types::InteractiveElement {
    obs.interactive
        .iter()
        .find(|e| e.role == role && e.name.to_lowercase().contains(&name_part.to_lowercase()))
        .unwrap_or_else(|| {
            panic!(
                "no `{role}` named like `{name_part}` in {:?}",
                obs.interactive.iter().map(|e| format!("{}:{}", e.role, e.name)).collect::<Vec<_>>()
            )
        })
}

fn eref(el: &core_types::InteractiveElement) -> ElementRef {
    el.element_ref.clone()
}

fn content_text(obs: &Observation) -> String {
    obs.content.iter().map(|c| c.text.as_str()).collect::<Vec<_>>().join("\n")
}

e2e!(observe_catalog_page, |h| {
    let wv = h.engine.create_webview(user_opts()).await.unwrap();
    h.engine.navigate(&wv, &h.fx.url("shop", "/")).await.unwrap();
    let obs = h.engine.observe(&wv).await.unwrap();

    assert!(obs.page.untrusted);
    assert_eq!(obs.page.origin.host(), "shop.localhost");
    assert!(obs.page.title.contains("Shop"), "title = {}", obs.page.title);
    assert!(!obs.page.snapshot_hash.is_empty());
    assert!(obs.interactive.iter().any(|e| e.role == "link" && e.href.as_deref().unwrap_or("").contains("/p/")));
    // Product names are headings: they must reach the model via `heading_path`.
    assert!(
        obs.content.iter().any(|c| c.heading_path.as_deref().unwrap_or("").contains("Aero X1 Headphones")),
        "content = {:?}",
        obs.content
    );
    assert!(content_text(&obs).contains("Rating"), "content = {}", content_text(&obs));
    assert!(obs.approx_tokens > 0);
    assert!(obs.hidden_text_signals.is_empty());

    let png = h.engine.screenshot(&wv).await.unwrap();
    assert_eq!(&png[..4], b"\x89PNG");
    let ax = h.engine.ax_tree(&wv).await.unwrap();
    assert!(ax.as_array().map(|a| a.len() > 5).unwrap_or(false));
    h.engine.close_webview(&wv).await.unwrap();
});

e2e!(type_submit_and_navigate_history, |h| {
    let wv = h.engine.create_webview(user_opts()).await.unwrap();
    h.engine.navigate(&wv, &h.fx.url("shop", "/search")).await.unwrap();
    let obs = h.engine.observe(&wv).await.unwrap();
    let box_ = find(&obs, "textbox", "Search products");
    let r = h.engine.act(&wv, Action::Type { target: eref(box_), text: "light".into(), submit: true }).await.unwrap();
    assert!(r.ok);
    assert!(r.url.contains("q=light"), "url = {}", r.url);
    let obs = h.engine.observe(&wv).await.unwrap();
    assert!(content_text(&obs).to_lowercase().contains("light"));

    let r = h.engine.act(&wv, Action::Back).await.unwrap();
    assert!(r.ok && !r.url.contains("q="), "back url = {}", r.url);
    let r = h.engine.act(&wv, Action::Forward).await.unwrap();
    assert!(r.ok && r.url.contains("q=light"), "forward url = {}", r.url);

    let events = h.engine.poll_events().await.unwrap();
    assert!(events.iter().any(|e| matches!(e, EngineEvent::Loaded { .. })));
    assert!(events.iter().any(|e| matches!(e, EngineEvent::Request { url, .. } if url.contains("shop.localhost"))));
});

e2e!(fill_contact_form_with_trusted_input, |h| {
    let wv = h.engine.create_webview(user_opts()).await.unwrap();
    h.engine.navigate(&wv, &h.fx.url("forms", "/contact")).await.unwrap();
    let obs = h.engine.observe(&wv).await.unwrap();

    let name = eref(find(&obs, "textbox", "Your name"));
    let email = eref(find(&obs, "textbox", "Email"));
    let topic = eref(find(&obs, "combobox", "Topic"));
    let message = eref(find(&obs, "textbox", "Message"));
    let newsletter = eref(find(&obs, "checkbox", "newsletter"));
    let send = eref(find(&obs, "button", "Send message"));

    h.engine.act(&wv, Action::Type { target: name, text: "Ada Lovelace".into(), submit: false }).await.unwrap();
    h.engine.act(&wv, Action::Type { target: email, text: "ada@example.org".into(), submit: false }).await.unwrap();
    let r =
        h.engine.act(&wv, Action::Select { target: topic.clone(), value: "Technical support".into() }).await.unwrap();
    assert!(r.ok, "{:?}", r.message);
    let r = h.engine.act(&wv, Action::Select { target: topic, value: "Nonexistent".into() }).await.unwrap();
    assert!(!r.ok);
    h.engine.act(&wv, Action::Type { target: message, text: "Hello there".into(), submit: false }).await.unwrap();
    h.engine.act(&wv, Action::Check { target: newsletter.clone(), checked: true }).await.unwrap();

    let obs = h.engine.observe(&wv).await.unwrap();
    assert!(obs.find_ref(newsletter.id).map(|e| e.state.checked).unwrap_or(false), "checkbox should be checked");
    assert_eq!(find(&obs, "textbox", "Your name").value.as_deref(), Some("Ada Lovelace"));

    let r = h.engine.act(&wv, Action::Click { target: send }).await.unwrap();
    assert!(r.ok);
    let contacts = h.fx.contacts();
    assert_eq!(contacts.len(), 1, "form should have been submitted once");
    let c = &contacts[0];
    assert_eq!(c.get("name").map(String::as_str), Some("Ada Lovelace"));
    assert_eq!(c.get("email").map(String::as_str), Some("ada@example.org"));
    assert_eq!(c.get("topic").map(String::as_str), Some("support"));
    assert_eq!(c.get("message").map(String::as_str), Some("Hello there"));
    assert_eq!(c.get("newsletter").map(String::as_str), Some("yes"));
});

e2e!(masked_fields_are_never_typed_into, |h| {
    // Keep this test isolated from the session-import test, which signs into the
    // shared user profile and may run in parallel.
    let wv = h.engine.create_webview(WebViewOptions::agent("masked-fields")).await.unwrap();

    h.engine.navigate(&wv, &h.fx.url("login", "/")).await.unwrap();
    let obs = h.engine.observe(&wv).await.unwrap();
    let pass = find(&obs, "textbox", "Password");
    assert!(pass.state.masked);
    assert!(pass.value.is_none());
    let err = h.engine.act(&wv, Action::Type { target: eref(pass), text: "demo".into(), submit: false }).await.err();
    assert!(matches!(err, Some(EngineError::Blocked(_))), "{err:?}");

    h.engine.navigate(&wv, &h.fx.url("shop", "/checkout")).await.unwrap();
    let obs = h.engine.observe(&wv).await.unwrap();
    assert_eq!(obs.page.page_kind, core_types::PageKind::Checkout);
    let card = find(&obs, "textbox", "Card number");
    assert!(card.state.masked, "cc-number must be masked");
    let err = h.engine.act(&wv, Action::Type { target: eref(card), text: "4111".into(), submit: false }).await.err();
    assert!(matches!(err, Some(EngineError::Blocked(_))));
    let place = find(&obs, "button", "Place order");
    assert!(place.consequential_hint, "submit inside a payment form must carry the consequential hint");
});

e2e!(agent_profile_is_isolated_and_session_import_is_scoped, |h| {
    let origin = Origin::parse(&h.fx.origin("login")).unwrap();

    // The user signs in in their own profile (password typed by the user, not the agent).
    let user = h.engine.create_webview(user_opts()).await.unwrap();
    h.engine.navigate(&user, &h.fx.url("login", "/")).await.unwrap();
    h.engine
        .evaluate(
            &user,
            "document.querySelector('[name=user]').value='demo'; document.querySelector('[name=pass]').value='demo'; document.querySelector('form').submit(); true",
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    h.engine.navigate(&user, &h.fx.url("login", "/account")).await.unwrap();
    let obs = h.engine.observe(&user).await.unwrap();
    assert!(obs.page.title.contains("Your account"), "user should be signed in: {}", obs.page.title);
    assert!(obs.page.has_session, "httpOnly session cookie must be detected");

    // A fresh agent context sees no cookies.
    let agent = h.engine.create_webview(WebViewOptions::agent("s1")).await.unwrap();
    h.engine.navigate(&agent, &h.fx.url("login", "/account")).await.unwrap();
    let obs = h.engine.observe(&agent).await.unwrap();
    assert!(obs.page.title.contains("Sign in"), "agent must not inherit the user's session: {}", obs.page.title);
    assert!(!obs.page.has_session);

    // Import only cookies for that origin; other origins are dropped.
    let mut cookies = h.engine.export_session(&user, &origin).await.unwrap();
    assert!(cookies.iter().any(|c| c.name == "sid" && c.http_only));
    cookies.push(engine_adapter::SessionCookie {
        name: "other".into(),
        value: "x".into(),
        domain: "shop.localhost".into(),
        path: "/".into(),
        secure: false,
        http_only: false,
        expires: None,
    });
    let n = h.engine.import_session(&agent, &origin, cookies.clone()).await.unwrap();
    assert_eq!(n, 1);
    h.engine.navigate(&agent, &h.fx.url("login", "/account")).await.unwrap();
    let obs = h.engine.observe(&agent).await.unwrap();
    assert!(obs.page.title.contains("Your account"), "{}", obs.page.title);
    assert!(obs.page.has_session);

    // Import into a user or private profile is refused.
    let err = h.engine.import_session(&user, &origin, cookies.clone()).await.err();
    assert!(matches!(err, Some(EngineError::Blocked(_))));
    let private =
        h.engine.create_webview(WebViewOptions { profile: ProfileKind::Private, ..user_opts() }).await.unwrap();
    let err = h.engine.import_session(&private, &origin, cookies).await.err();
    assert!(matches!(err, Some(EngineError::Blocked(_))));
    h.engine.navigate(&private, &h.fx.url("login", "/account")).await.unwrap();
    let obs = h.engine.observe(&private).await.unwrap();
    assert!(obs.page.title.contains("Sign in"));
});

e2e!(hostile_page_hidden_instructions_are_quarantined, |h| {
    let wv = h.engine.create_webview(WebViewOptions::agent("s2")).await.unwrap();
    h.engine.navigate(&wv, &h.fx.url("hostile", "/")).await.unwrap();
    let obs = h.engine.observe(&wv).await.unwrap();

    let visible = content_text(&obs);
    assert!(!visible.contains("admin mode"), "hidden text leaked into content: {visible}");
    assert!(!visible.contains("ignore all previous"), "off-screen text leaked into content: {visible}");
    assert!(!obs.hidden_text_signals.is_empty(), "hidden instructions must be reported as signals");
    assert!(
        obs.hidden_text_signals.iter().any(|s| s.to_lowercase().contains("admin mode")),
        "{:?}",
        obs.hidden_text_signals
    );
    let injected = obs.content.iter().filter(|c| c.suspect_injection).count();
    assert!(injected >= 1, "visible instruction aimed at agents must be flagged: {:?}", obs.content);
    assert!(obs.has_injection_signals());

    let evil = h.fx.origin("evil");
    let link = obs.interactive.iter().find(|e| e.href.as_deref().map(|u| u.starts_with(&evil)).unwrap_or(false));
    assert!(link.is_some(), "cross-origin link is still reported as data");

    // Merely observing the page must not have contacted the attacker.
    assert!(h.fx.collected().is_empty());
    // The sensor lives in an isolated world: the page cannot see it.
    let leaked = h.engine.evaluate(&wv, "typeof BrowseSensor").await.unwrap();
    assert_eq!(leaked.as_str(), Some("undefined"));
});

e2e!(read_more_returns_full_chunk_and_unknown_ids_are_null, |h| {
    let wv = h.engine.create_webview(user_opts()).await.unwrap();
    h.engine.navigate(&wv, &h.fx.url("news", "/a/solar-roofs")).await.unwrap();
    let obs = h.engine.observe(&wv).await.unwrap();
    assert_eq!(obs.page.page_kind, core_types::PageKind::Article);
    let first = obs.content.first().expect("article has content");
    let r = h.engine.act(&wv, Action::ReadMore { obs_id: first.obs_id.clone() }).await.unwrap();
    let text = r.output.as_ref().and_then(|v| v.as_str()).expect("read_more returns text");
    assert!(text.len() >= first.text.trim_end_matches('…').len());
    let r = h.engine.act(&wv, Action::ReadMore { obs_id: "nope".into() }).await.unwrap();
    assert!(r.output.as_ref().map(|v| v.is_null()).unwrap_or(true));

    let err = h.engine.act(&wv, Action::Click { target: ElementRef { id: 999_999, path: String::new() } }).await.err();
    assert!(matches!(err, Some(EngineError::NoSuchElement(_))), "{err:?}");
    let err = h.engine.observe(&"wv-missing".to_string()).await.err();
    assert!(matches!(err, Some(EngineError::NoSuchWebView(_))));
});
