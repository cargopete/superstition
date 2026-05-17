//! Superstition public feed server.
//!
//! Routes:
//!   GET /              → HTML feed
//!   GET /feed.rss      → RSS 2.0
//!   GET /api/patterns  → JSON array
//!   GET /wasm/:id      → raw .wasm bytes (Content-Type: application/wasm)
//!
//! Configuration (env vars):
//!   FEED_PATH  — path to feed.json (default: feed.json)
//!   PORT       — listen port (default: 8080)

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use superstition_feed::{FeedStore, Pattern};

#[derive(Clone)]
struct AppState {
    feed: Arc<FeedStore>,
}

#[tokio::main]
async fn main() {
    let feed_path = std::env::var("FEED_PATH").unwrap_or_else(|_| "feed.json".into());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);

    let state = AppState { feed: Arc::new(FeedStore::open(&feed_path)) };

    let app = Router::new()
        .route("/", get(index))
        .route("/feed.rss", get(rss))
        .route("/api/patterns", get(api_patterns))
        .route("/wasm/:id", get(wasm_download))
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    println!("Superstition feed → http://localhost:{port}");
    println!("RSS               → http://localhost:{port}/feed.rss");
    println!("JSON API          → http://localhost:{port}/api/patterns");
    println!("feed path         : {feed_path}");

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

// ── handlers ─────────────────────────────────────────────────────────────────

async fn index(State(s): State<AppState>) -> Html<String> {
    let patterns = s.feed.patterns().unwrap_or_default();
    Html(render_html(&patterns))
}

async fn rss(State(s): State<AppState>) -> impl IntoResponse {
    let patterns = s.feed.patterns().unwrap_or_default();
    (
        [(header::CONTENT_TYPE, "application/rss+xml; charset=utf-8")],
        render_rss(&patterns),
    )
}

async fn api_patterns(State(s): State<AppState>) -> impl IntoResponse {
    let patterns = s.feed.patterns().unwrap_or_default();
    axum::Json(patterns)
}

async fn wasm_download(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match s.feed.get(&id) {
        Ok(Some(p)) => match p.wasm_bytes() {
            Ok(bytes) => (
                [
                    (header::CONTENT_TYPE, "application/wasm"),
                    (
                        header::CONTENT_DISPOSITION,
                        &format!("attachment; filename=\"{id}.wasm\"") as &str,
                    ),
                ],
                bytes,
            )
                .into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

// ── HTML rendering ────────────────────────────────────────────────────────────

fn render_html(patterns: &[Pattern]) -> String {
    let rows = if patterns.is_empty() {
        "<tr><td colspan=\"6\" style=\"text-align:center;color:#8b949e;padding:2rem\">\
         No significant patterns published yet.</td></tr>"
            .to_string()
    } else {
        patterns
            .iter()
            .map(|p| {
                format!(
                    r#"<tr>
  <td><span class="id" title="{id}">{id_short}…</span></td>
  <td>{desc}</td>
  <td><span class="family">{family}</span></td>
  <td class="mono">{q:.2e}</td>
  <td class="mono">{effect:.3}</td>
  <td class="mono verify">superstition-verify {id}</td>
</tr>"#,
                    id = p.id,
                    id_short = &p.id[..8],
                    desc = html_escape(&p.description),
                    family = html_escape(&p.family),
                    q = p.q_value,
                    effect = p.effect_size,
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Superstition — Verifiable On-Chain Patterns</title>
<style>
  :root {{
    --bg: #0d1117; --surface: #161b22; --border: #30363d;
    --text: #e6edf3; --muted: #8b949e; --accent: #58a6ff;
    --green: #3fb950; --purple: #d2a8ff;
  }}
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{ background: var(--bg); color: var(--text); font-family: -apple-system,BlinkMacSystemFont,"Segoe UI",monospace; line-height: 1.6; }}
  header {{ border-bottom: 1px solid var(--border); padding: 1.5rem 2rem; display: flex; align-items: baseline; gap: 1rem; }}
  header h1 {{ font-size: 1.4rem; color: var(--accent); }}
  header p {{ color: var(--muted); font-style: italic; font-size: 0.9rem; }}
  .links {{ margin-left: auto; font-size: 0.8rem; }}
  .links a {{ color: var(--muted); text-decoration: none; margin-left: 1rem; }}
  .links a:hover {{ color: var(--text); }}
  main {{ padding: 2rem; max-width: 1100px; margin: 0 auto; }}
  .tagline {{ color: var(--muted); font-size: 0.85rem; margin-bottom: 1.5rem; }}
  .tagline code {{ background: var(--surface); padding: 0.1em 0.4em; border-radius: 3px; color: var(--purple); }}
  table {{ width: 100%; border-collapse: collapse; font-size: 0.85rem; }}
  th {{ text-align: left; color: var(--muted); font-weight: 600; padding: 0.5rem 1rem; border-bottom: 1px solid var(--border); }}
  td {{ padding: 0.6rem 1rem; border-bottom: 1px solid var(--border); vertical-align: top; }}
  tr:hover td {{ background: var(--surface); }}
  .id {{ font-family: monospace; color: var(--muted); font-size: 0.78rem; cursor: help; }}
  .family {{ background: var(--surface); border: 1px solid var(--border); padding: 0.1em 0.5em; border-radius: 3px; font-size: 0.78rem; color: var(--purple); white-space: nowrap; }}
  .mono {{ font-family: monospace; color: var(--green); }}
  .verify {{ color: var(--muted); font-size: 0.78rem; user-select: all; }}
  .verify:hover {{ color: var(--text); }}
  footer {{ border-top: 1px solid var(--border); padding: 1rem 2rem; color: var(--muted); font-size: 0.78rem; }}
</style>
</head>
<body>
<header>
  <h1>✦ Superstition</h1>
  <p>Verifiable astrology for on-chain data.</p>
  <nav class="links">
    <a href="/feed.rss">RSS</a>
    <a href="/api/patterns">JSON</a>
  </nav>
</header>
<main>
  <p class="tagline">
    Every pattern carries a <code>wasm_hash</code>, a <code>corpus_id</code>, and a corrected
    <code>q-value</code>. Reproduce any result: paste the <em>verify</em> command below.
  </p>
  <table>
    <thead>
      <tr>
        <th>ID</th><th>Description</th><th>Family</th>
        <th>q-value</th><th>Effect</th><th>Reproduce</th>
      </tr>
    </thead>
    <tbody>
{rows}
    </tbody>
  </table>
</main>
<footer>
  <em>"The stars don't lie. They just don't always tell the truth."</em>
</footer>
</body>
</html>"#
    )
}

// ── RSS rendering ─────────────────────────────────────────────────────────────

fn render_rss(patterns: &[Pattern]) -> String {
    let items = patterns
        .iter()
        .map(|p| {
            format!(
                r#"    <item>
      <title>{title}</title>
      <description>q={q:.2e} · effect={effect:.3} · corpus={corpus} | verify: {cmd}</description>
      <pubDate>{date}</pubDate>
      <guid isPermaLink="false">superstition:{id}</guid>
    </item>"#,
                title = xml_escape(&p.description),
                q = p.q_value,
                effect = p.effect_size,
                corpus = p.corpus_id,
                cmd = p.verify_cmd(),
                date = p.published_at,
                id = p.id,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Superstition — Verifiable On-Chain Patterns</title>
    <link>http://localhost:8080</link>
    <description>Statistically significant patterns in EVM chain data, reproducible from bytecode alone.</description>
    <language>en</language>
{items}
  </channel>
</rss>"#
    )
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn xml_escape(s: &str) -> String {
    html_escape(s).replace('"', "&quot;").replace('\'', "&apos;")
}
