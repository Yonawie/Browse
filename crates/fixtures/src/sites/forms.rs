use axum::http::StatusCode;
use axum::response::Response;

use super::{not_found, Ctx};
use crate::{escape, html, page};

pub fn handle(ctx: &Ctx<'_>) -> Response {
    match (ctx.method, ctx.path) {
        ("GET", "/") | ("GET", "/contact") => {
            let body = r#"<h1>Contact us</h1>
<form method="post" action="/contact" id="contact">
<label>Your name <input name="name" autocomplete="name" required></label>
<label>Email address <input name="email" type="email" autocomplete="email" required></label>
<label>Topic <select name="topic"><option value="">Choose…</option><option value="billing">Billing</option><option value="support">Technical support</option><option value="sales">Sales</option></select></label>
<label>Message <textarea name="message" rows="4"></textarea></label>
<label><input type="checkbox" name="newsletter" value="yes"> Subscribe to the newsletter</label>
<button type="submit">Send message</button>
</form>"#;
            html(StatusCode::OK, page("Forms", "Contact us", "", body))
        }
        ("POST", "/contact") => {
            ctx.state.shared.lock().unwrap().contacts.push(ctx.form.clone());
            let rows: String = ["name", "email", "topic", "message", "newsletter"]
                .iter()
                .map(|k| {
                    format!(
                        "<tr><th>{k}</th><td id=\"echo-{k}\">{}</td></tr>",
                        escape(ctx.form.get(*k).map(String::as_str).unwrap_or(""))
                    )
                })
                .collect();
            let body = format!("<h1>Thanks, we got your message</h1><table>{rows}</table>");
            html(StatusCode::OK, page("Forms", "Message sent", "", &body))
        }
        _ => not_found(),
    }
}
