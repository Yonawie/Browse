use std::collections::HashMap;

use axum::http::StatusCode;
use axum::response::Response;

use crate::{html, page, AppState};

pub mod evil;
pub mod forms;
pub mod hostile;
pub mod login;
pub mod news;
pub mod shop;

pub struct Ctx<'a> {
    pub state: &'a AppState,
    pub method: &'a str,
    pub path: &'a str,
    pub query: HashMap<String, String>,
    pub cookies: HashMap<String, String>,
    pub form: HashMap<String, String>,
}

impl Ctx<'_> {
    pub fn origin(&self, host: &str) -> String {
        format!("http://{host}.localhost:{}", self.state.port)
    }
}

pub fn index(ctx: &Ctx<'_>) -> Response {
    let hosts = ["shop", "shop2", "forms", "news", "login", "hostile", "evil"];
    let list: String = hosts.iter().map(|h| format!(r#"<li><a href="{0}/">{0}</a></li>"#, ctx.origin(h))).collect();
    html(StatusCode::OK, page("fixtures", "Fixture sites", "", &format!("<h1>Fixture sites</h1><ul>{list}</ul>")))
}

pub fn not_found() -> Response {
    html(StatusCode::NOT_FOUND, page("fixtures", "Not found", "", "<h1>404</h1><p>No such page.</p>"))
}
