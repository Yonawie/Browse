use axum::http::StatusCode;
use axum::response::Response;

use super::{not_found, Ctx};
use crate::{html, page};

pub struct Article {
    pub slug: &'static str,
    pub title: &'static str,
    pub lang: &'static str,
    pub body: &'static str,
}

pub const ARTICLES: &[Article] = &[
    Article {
        slug: "solar-roofs",
        title: "City council approves solar roofs for all new schools",
        lang: "en",
        body: r#"<p>The city council voted 9–2 on Tuesday to require solar panels on every new school building starting in 2027. The measure, proposed by council member Dana Ruiz, is expected to cut the district's electricity bill by roughly 18 percent within five years.</p>
<h2>What the rule requires</h2>
<p>New schools must cover at least 60 percent of usable roof area with photovoltaic panels and include battery storage sized for four hours of emergency lighting. Renovations that replace a roof must add panels if the structure can bear the load.</p>
<h2>Cost and funding</h2>
<p>The district estimates an upfront cost of 3.2 million dollars for the first three schools, offset by a state grant covering 40 percent and by energy savings of about 210,000 dollars per year. Critics argued the money would be better spent on teacher salaries.</p>
<h2>Timeline</h2>
<p>Design guidelines will be published in March 2026. The first school built under the rule, Northgate Elementary, breaks ground in the summer of 2027 and opens in 2029.</p>"#,
    },
    Article {
        slug: "battery-recycling",
        title: "Battery recycling plant opens with capacity for 50,000 tonnes a year",
        lang: "en",
        body: r#"<p>A new lithium-ion battery recycling facility opened outside Rotterdam on Monday. Operator NordCycle says the plant can process 50,000 tonnes of spent batteries per year and recover 95 percent of the lithium, nickel and cobalt they contain.</p>
<h2>How it works</h2>
<p>Batteries are discharged, shredded under nitrogen and separated into copper, aluminium and a powder known as black mass. The black mass is dissolved in acid and the metals are precipitated one by one.</p>
<h2>Why it matters</h2>
<p>Europe's battery regulation requires recycled content in new cells from 2031. NordCycle expects to supply material for about 400,000 electric-car batteries a year at full capacity and employs 180 people.</p>"#,
    },
    Article {
        slug: "gorod-park",
        title: "Город открыл новый парк на месте бывшего завода",
        lang: "ru",
        body: r#"<p>В субботу в Заречном районе открылся парк площадью 12 гектаров, разбитый на месте закрытого в 2015 году механического завода. Проект обошёлся городу в 480 миллионов рублей, треть из которых выделил областной бюджет.</p>
<h2>Что есть в парке</h2>
<p>В парке проложено шесть километров дорожек, построены две детские площадки, скейт-парк и амфитеатр на 600 мест. Старый заводской цех переоборудован под крытый рынок и мастерские.</p>
<h2>Планы</h2>
<p>Ко второй очереди, запланированной на 2027 год, обещают построить пешеходный мост через реку и пруд с очистными сооружениями. Жители жалуются на нехватку парковок: рядом с парком всего 120 мест.</p>"#,
    },
];

pub fn handle(ctx: &Ctx<'_>) -> Response {
    match (ctx.method, ctx.path) {
        ("GET", "/") => {
            let list: String =
                ARTICLES.iter().map(|a| format!(r#"<li><a href="/a/{}">{}</a></li>"#, a.slug, a.title)).collect();
            html(
                StatusCode::OK,
                page("News", "News", r#"<a href="/table">Data table</a>"#, &format!("<h1>Latest</h1><ul>{list}</ul>")),
            )
        }
        ("GET", "/table") => {
            let body = r#"<h1>Largest cities by population</h1>
<table id="cities"><thead><tr><th>City</th><th>Country</th><th>Population (millions)</th><th>Area (km²)</th></tr></thead>
<tbody>
<tr><td>Tokyo</td><td>Japan</td><td>37.4</td><td>2194</td></tr>
<tr><td>Delhi</td><td>India</td><td>32.9</td><td>1484</td></tr>
<tr><td>Shanghai</td><td>China</td><td>29.2</td><td>6341</td></tr>
<tr><td>São Paulo</td><td>Brazil</td><td>22.6</td><td>1521</td></tr>
<tr><td>Cairo</td><td>Egypt</td><td>22.2</td><td>3085</td></tr>
</tbody></table>"#;
            html(StatusCode::OK, page("News", "Largest cities", "", body))
        }
        ("GET", p) if p.starts_with("/a/") => {
            let slug = &p[3..];
            let Some(a) = ARTICLES.iter().find(|a| a.slug == slug) else { return not_found() };
            let body = format!(r#"<article><h1>{}</h1>{}</article>"#, a.title, a.body);
            let doc = page("News", a.title, "", &body).replacen("lang=\"en\"", &format!("lang=\"{}\"", a.lang), 1);
            html(StatusCode::OK, doc)
        }
        _ => not_found(),
    }
}
