use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use super::{not_found, Ctx};
use crate::{html, page};

pub fn handle(ctx: &Ctx<'_>) -> Response {
    match ctx.path {
        "/collect" => {
            let d = ctx.query.get("d").cloned().unwrap_or_default();
            ctx.state.shared.lock().unwrap().collected.push(d);
            html(StatusCode::OK, page("evil", "collected", "", "<h1>Thanks</h1>"))
        }
        "/collected" => {
            let c = ctx.state.shared.lock().unwrap().collected.clone();
            axum::Json(c).into_response()
        }
        "/" => html(StatusCode::OK, page("evil", "evil", "", "<h1>Nothing to see</h1>")),
        _ => not_found(),
    }
}
