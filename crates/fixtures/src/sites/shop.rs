use axum::http::StatusCode;
use axum::response::Response;

use super::{not_found, Ctx};
use crate::{escape, html, html_with_cookie, page, redirect, Product};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ShopVariant {
    A,
    B,
}

fn p(id: u32, name: &str, brand: &str, price_cents: u32, weight_g: u32, battery_h: u32, rating: f32) -> Product {
    Product { id, name: name.into(), brand: brand.into(), price_cents, weight_g, battery_h, rating }
}

pub fn catalog(v: ShopVariant) -> Vec<Product> {
    match v {
        ShopVariant::A => vec![
            p(1, "Aero X1 Headphones", "Aero", 12_900, 250, 30, 4.5),
            p(2, "Aero X2 Headphones", "Aero", 19_900, 240, 40, 4.7),
            p(3, "Pulse Mini Speaker", "Pulse", 4_900, 300, 12, 4.1),
            p(4, "Pulse Max Speaker", "Pulse", 14_900, 1200, 20, 4.4),
            p(5, "Nimbus Keyboard", "Nimbus", 8_900, 700, 0, 4.2),
            p(6, "Nimbus Mouse", "Nimbus", 3_900, 90, 0, 4.0),
            p(7, "Orbit Webcam 4K", "Orbit", 9_900, 150, 0, 3.9),
            p(8, "Orbit Ring Light", "Orbit", 2_900, 400, 0, 4.3),
        ],
        ShopVariant::B => vec![
            p(1, "Aero X1 Headphones", "Aero", 11_900, 250, 30, 4.4),
            p(2, "Aero X2 Headphones", "Aero", 21_900, 240, 40, 4.6),
            p(3, "Pulse Mini Speaker", "Pulse", 5_400, 300, 12, 4.0),
            p(9, "Echo Buds", "Echo", 7_900, 50, 8, 4.1),
            p(5, "Nimbus Keyboard", "Nimbus", 7_900, 700, 0, 4.3),
            p(10, "Vortex Charger 65W", "Vortex", 3_400, 120, 0, 4.6),
        ],
    }
}

fn money(cents: u32) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}

fn site_name(v: ShopVariant) -> &'static str {
    match v {
        ShopVariant::A => "Shop",
        ShopVariant::B => "Shop2",
    }
}

fn nav() -> &'static str {
    r#"<a href="/search">Search</a> <a href="/cart">Cart</a>"#
}

fn cart_ids(ctx: &Ctx<'_>) -> Vec<u32> {
    ctx.cookies.get("cart").map(|c| c.split(',').filter_map(|s| s.parse().ok()).collect()).unwrap_or_default()
}

fn cart_cookie(ids: &[u32]) -> String {
    format!("cart={}; Path=/", ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(","))
}

fn product_card(pr: &Product) -> String {
    format!(
        r#"<article class="product"><h2><a href="/p/{id}">{name}</a></h2>
<p>Brand: {brand}. Rating: {rating}/5.</p><p class="price">{price}</p></article>"#,
        id = pr.id,
        name = escape(&pr.name),
        brand = pr.brand,
        rating = pr.rating,
        price = money(pr.price_cents)
    )
}

pub fn handle(ctx: &Ctx<'_>, v: ShopVariant) -> Response {
    let site = site_name(v);
    let items = catalog(v);
    let segs: Vec<&str> = ctx.path.trim_matches('/').split('/').collect();
    match (ctx.method, segs.as_slice()) {
        ("GET", [""]) => {
            let cards: String = items.iter().map(product_card).collect();
            html(StatusCode::OK, page(site, &format!("{site} — catalog"), nav(), &format!("<h1>Catalog</h1>{cards}")))
        }
        ("GET", ["search"]) => {
            let q = ctx.query.get("q").cloned().unwrap_or_default();
            let hits: Vec<&Product> = items
                .iter()
                .filter(|pr| {
                    q.is_empty()
                        || pr.name.to_lowercase().contains(&q.to_lowercase())
                        || pr.brand.to_lowercase().contains(&q.to_lowercase())
                })
                .collect();
            let cards: String = hits.iter().map(|pr| product_card(pr)).collect();
            let body = format!(
                r#"<h1>Search</h1><form method="get" action="/search"><label>Search products <input type="search" name="q" value="{q}"></label><button type="submit">Search</button></form>
<p>{n} result(s) for “{q}”.</p>{cards}"#,
                q = escape(&q),
                n = hits.len()
            );
            html(StatusCode::OK, page(site, &format!("Search results for {q}"), nav(), &body))
        }
        ("GET", ["p", id]) => {
            let Some(pr) = id.parse::<u32>().ok().and_then(|i| items.into_iter().find(|p| p.id == i)) else {
                return not_found();
            };
            let body = format!(
                r#"<h1>{name}</h1><p class="price">Price: {price}</p>
<table><caption>Specifications</caption>
<tr><th>Brand</th><td>{brand}</td></tr>
<tr><th>Weight</th><td>{weight} g</td></tr>
<tr><th>Battery life</th><td>{battery} h</td></tr>
<tr><th>Rating</th><td>{rating} / 5</td></tr></table>
<form method="post" action="/cart/add/{id}"><button type="submit" name="add">Add to cart</button></form>
<p><a href="/">Back to catalog</a></p>"#,
                name = escape(&pr.name),
                price = money(pr.price_cents),
                brand = pr.brand,
                weight = pr.weight_g,
                battery = pr.battery_h,
                rating = pr.rating,
                id = pr.id
            );
            html(StatusCode::OK, page(site, &pr.name, nav(), &body))
        }
        ("POST", ["cart", "add", id]) => {
            let mut ids = cart_ids(ctx);
            if let Ok(i) = id.parse() {
                ids.push(i);
            }
            let body =
                page(site, "Added to cart", nav(), r#"<h1>Added to cart</h1><p><a href="/cart">View cart</a></p>"#);
            html_with_cookie(body, &cart_cookie(&ids))
        }
        ("GET", ["cart"]) => {
            let ids = cart_ids(ctx);
            let rows: String = ids
                .iter()
                .filter_map(|i| items.iter().find(|p| p.id == *i))
                .map(|pr| format!("<tr><td>{}</td><td>{}</td></tr>", escape(&pr.name), money(pr.price_cents)))
                .collect();
            let total: u32 = ids.iter().filter_map(|i| items.iter().find(|p| p.id == *i)).map(|p| p.price_cents).sum();
            let body = format!(
                r#"<h1>Your cart</h1><table><tr><th>Item</th><th>Price</th></tr>{rows}</table>
<p>Total: <span class="price">{total}</span></p>
<p><a href="/checkout">Proceed to checkout</a></p>"#,
                total = money(total)
            );
            html(StatusCode::OK, page(site, "Cart", nav(), &body))
        }
        ("GET", ["checkout"]) => {
            let body = r#"<h1>Checkout</h1>
<form method="post" action="/checkout" id="checkout">
<label>Full name <input name="name" autocomplete="name" required></label>
<label>Email <input name="email" type="email" autocomplete="email" required></label>
<label>Card number <input name="card" autocomplete="cc-number" inputmode="numeric" required></label>
<label>CVC <input name="cvc" autocomplete="cc-csc" required></label>
<button type="submit">Place order</button></form>"#;
            html(StatusCode::OK, page(site, "Checkout", nav(), body))
        }
        ("POST", ["checkout"]) => {
            ctx.state.shared.lock().unwrap().orders.push(format!("{:?}", ctx.form));
            html(StatusCode::OK, page(site, "Order placed", nav(), "<h1>Order placed</h1><p>Thank you!</p>"))
        }
        ("GET", ["cart", "add", _]) => redirect("/cart"),
        _ => not_found(),
    }
}
