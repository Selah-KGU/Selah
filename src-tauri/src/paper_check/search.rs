//! Pluggable web-search backend for the similarity/plagiarism channel.
//!
//! Default is a keyless free provider (DuckDuckGo HTML endpoint) so the feature
//! works out of the box; it is best-effort and may be rate-limited. Users who
//! want reliability can drop a free-tier Brave or Google CSE key into the
//! PaperCheck config and the provider switches automatically.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::LazyLock;

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    /// "free" | "brave" | "google"
    pub provider: String,
    pub brave_api_key: String,
    pub google_api_key: String,
    pub google_cx: String,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            provider: "free".to_string(),
            brave_api_key: String::new(),
            google_api_key: String::new(),
            google_cx: String::new(),
        }
    }
}

fn config_path() -> PathBuf {
    crate::client::data_dir().join("paper_check_search.json")
}

pub fn load_config() -> SearchConfig {
    let path = config_path();
    if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or_default()
    } else {
        SearchConfig::default()
    }
}

pub fn save_config(config: &SearchConfig) -> Result<(), String> {
    let data = serde_json::to_string_pretty(config)
        .map_err(|e| format!("JSON serialization error: {e}"))?;
    std::fs::write(config_path(), data).map_err(|e| format!("保存に失敗しました: {e}"))
}

pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
        reqwest::Client::builder()
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/122.0 Safari/537.36",
            )
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default()
    });
    &CLIENT
}

/// Which provider is effectively active given the config + present keys.
pub fn active_provider_label(cfg: &SearchConfig) -> &'static str {
    match cfg.provider.as_str() {
        "brave" if !cfg.brave_api_key.is_empty() => "brave",
        "google" if !cfg.google_api_key.is_empty() && !cfg.google_cx.is_empty() => "google",
        _ => "free",
    }
}

/// Run one search, dispatching to whichever provider is configured/available.
pub async fn search(
    cfg: &SearchConfig,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, String> {
    match active_provider_label(cfg) {
        "brave" => brave_search(&cfg.brave_api_key, query, limit).await,
        "google" => google_search(&cfg.google_api_key, &cfg.google_cx, query, limit).await,
        _ => duckduckgo_search(query, limit).await,
    }
}

/// Keyless DuckDuckGo HTML endpoint. Fragile by nature (HTML scraping) — treat
/// failures as "no results", never as a hard error for the whole report.
async fn duckduckgo_search(query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );
    let resp = http_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("DuckDuckGo request failed: {e}"))?;
    let body = resp
        .text()
        .await
        .map_err(|e| format!("DuckDuckGo read failed: {e}"))?;

    // Parse in a block so the non-Send scraper::Html is dropped before returning.
    let results = {
        use scraper::{Html, Selector};
        let doc = Html::parse_document(&body);
        let link_sel = Selector::parse("a.result__a").unwrap();
        let snip_sel = Selector::parse(".result__snippet").unwrap();
        let snippets: Vec<String> = doc
            .select(&snip_sel)
            .map(|e| e.text().collect::<String>().trim().to_string())
            .collect();
        let mut out = Vec::new();
        for (idx, a) in doc.select(&link_sel).enumerate() {
            if out.len() >= limit {
                break;
            }
            let href = a.value().attr("href").unwrap_or("");
            let url = decode_ddg_href(href);
            if url.is_empty() {
                continue;
            }
            let title = a.text().collect::<String>().trim().to_string();
            let snippet = snippets.get(idx).cloned().unwrap_or_default();
            out.push(SearchResult {
                title,
                url,
                snippet,
            });
        }
        out
    };
    Ok(results)
}

/// DDG wraps result links as `//duckduckgo.com/l/?uddg=<urlencoded target>`.
fn decode_ddg_href(href: &str) -> String {
    if let Some(pos) = href.find("uddg=") {
        let rest = &href[pos + 5..];
        let enc = rest.split('&').next().unwrap_or(rest);
        if let Ok(decoded) = urlencoding::decode(enc) {
            return decoded.into_owned();
        }
    }
    if href.starts_with("http") {
        href.to_string()
    } else {
        String::new()
    }
}

async fn brave_search(key: &str, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding::encode(query),
        limit.min(20)
    );
    let resp = http_client()
        .get(&url)
        .header("X-Subscription-Token", key)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Brave request failed: {e}"))?;
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Brave parse failed: {e}"))?;
    let mut out = Vec::new();
    if let Some(items) = json["web"]["results"].as_array() {
        for it in items.iter().take(limit) {
            out.push(SearchResult {
                title: it["title"].as_str().unwrap_or("").to_string(),
                url: it["url"].as_str().unwrap_or("").to_string(),
                snippet: it["description"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    Ok(out)
}

async fn google_search(
    key: &str,
    cx: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchResult>, String> {
    let url = format!(
        "https://www.googleapis.com/customsearch/v1?key={}&cx={}&q={}&num={}",
        urlencoding::encode(key),
        urlencoding::encode(cx),
        urlencoding::encode(query),
        limit.min(10)
    );
    let resp = http_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Google CSE request failed: {e}"))?;
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Google CSE parse failed: {e}"))?;
    let mut out = Vec::new();
    if let Some(items) = json["items"].as_array() {
        for it in items.iter().take(limit) {
            out.push(SearchResult {
                title: it["title"].as_str().unwrap_or("").to_string(),
                url: it["link"].as_str().unwrap_or("").to_string(),
                snippet: it["snippet"].as_str().unwrap_or("").to_string(),
            });
        }
    }
    Ok(out)
}
