use axum::http::StatusCode;
use axum::response::Response;

use super::{not_found, Ctx};
use crate::{html, html_with_cookie, page, redirect};

pub const SESSION_COOKIE: &str = "sid";
pub const SESSION_VALUE: &str = "demo-session-token";

pub fn handle(ctx: &Ctx<'_>) -> Response {
    let logged_in = ctx.cookies.get(SESSION_COOKIE).map(String::as_str) == Some(SESSION_VALUE);
    match (ctx.method, ctx.path) {
        ("GET", "/") => {
            let body = if logged_in {
                r#"<h1>Welcome back</h1><p>You are signed in. <a href="/account">Go to your account</a></p>"#
                    .to_string()
            } else {
                r#"<h1>Sign in</h1><form method="post" action="/login">
<label>Username <input name="user" autocomplete="username"></label>
<label>Password <input name="pass" type="password" autocomplete="current-password"></label>
<button type="submit">Sign in</button></form><p>Demo account: demo / demo</p>"#
                    .to_string()
            };
            html(StatusCode::OK, page("Login", "Sign in", r#"<a href="/account">Account</a>"#, &body))
        }
        ("POST", "/login") => {
            if ctx.form.get("user").map(String::as_str) == Some("demo")
                && ctx.form.get("pass").map(String::as_str) == Some("demo")
            {
                let body = page("Login", "Signed in", "", r#"<h1>Signed in</h1><p><a href="/account">Account</a></p>"#);
                html_with_cookie(body, &format!("{SESSION_COOKIE}={SESSION_VALUE}; Path=/; HttpOnly"))
            } else {
                html(StatusCode::UNAUTHORIZED, page("Login", "Wrong credentials", "", "<h1>Wrong credentials</h1>"))
            }
        }
        ("GET", "/account") => {
            if !logged_in {
                return redirect("/");
            }
            let body = r#"<h1>Your account</h1>
<table><tr><th>Name</th><td id="acct-name">Demo User</td></tr>
<tr><th>Plan</th><td id="acct-plan">Pro (annual)</td></tr>
<tr><th>Next invoice</th><td id="acct-invoice">2026-10-01, $120.00</td></tr></table>
<form method="post" action="/logout"><button type="submit">Sign out</button></form>"#;
            html(StatusCode::OK, page("Login", "Your account", "", body))
        }
        ("POST", "/logout") => html_with_cookie(
            page("Login", "Signed out", "", "<h1>Signed out</h1>"),
            &format!("{SESSION_COOKIE}=; Path=/; Max-Age=0"),
        ),
        _ => not_found(),
    }
}
