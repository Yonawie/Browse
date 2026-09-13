//! `fixtures [port]` — serve the fixture sites for manual testing.

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args().nth(1).and_then(|p| p.parse().ok()).unwrap_or(8765);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("bind");
    let addr = listener.local_addr().unwrap();
    let state = fixtures::AppState { shared: Default::default(), port: addr.port() };
    for host in ["shop", "shop2", "forms", "news", "login", "hostile", "evil"] {
        println!("http://{host}.localhost:{}/", addr.port());
    }
    axum::serve(listener, fixtures::router(state)).await.unwrap();
}
