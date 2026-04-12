//! [`WebsiteSource`] — crawl a website and convert pages to Markdown.
//!
//! The crawler performs a breadth-first crawl starting from `root_url`,
//! follows links within the same host, and converts each HTML page to
//! Markdown via [`htmd`].  The resulting `.md` files are written to a
//! temporary directory that is returned as a [`PreparedSource`].

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;

use lore_core::LoreError;
use reqwest::Url;
use scraper::{Html, Selector};
use tracing::{debug, warn};

use super::{PreparedSource, Source};

/// The default maximum number of pages to crawl.
const DEFAULT_MAX_PAGES: usize = 500;

/// A documentation source that crawls a live website.
///
/// Pages are fetched, converted from HTML to Markdown, and stored in a
/// temporary directory.  Only pages within the same HTTP host as `root_url`
/// are followed.
pub struct WebsiteSource {
    /// The starting URL for the crawl.
    pub root_url: String,
    /// Maximum number of pages to crawl. Defaults to [`DEFAULT_MAX_PAGES`].
    pub max_pages: usize,
}

impl WebsiteSource {
    /// Create a [`WebsiteSource`] starting from `root_url`.
    pub fn new(root_url: impl Into<String>) -> Self {
        Self { root_url: root_url.into(), max_pages: DEFAULT_MAX_PAGES }
    }

    /// Set the maximum number of pages to crawl.
    #[must_use]
    pub const fn with_max_pages(mut self, max_pages: usize) -> Self {
        self.max_pages = max_pages;
        self
    }
}

impl Source for WebsiteSource {
    async fn prepare(&self) -> Result<PreparedSource, LoreError> {
        let root: Url = self
            .root_url
            .parse()
            .map_err(|e| LoreError::Registry(format!("invalid URL '{}': {e}", self.root_url)))?;

        let temp = tempfile::TempDir::new().map_err(LoreError::Io)?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("lore-crawler/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| LoreError::Registry(e.to_string()))?;

        crawl(&http, root, self.max_pages, temp.path().to_path_buf()).await?;
        Ok(PreparedSource::from_temp(temp, None))
    }
}

// ── Private crawler ───────────────────────────────────────────────────────────

async fn crawl(
    http: &reqwest::Client,
    root: Url,
    max_pages: usize,
    out_dir: PathBuf,
) -> Result<(), LoreError> {
    let link_selector = Selector::parse("a[href]").expect("valid selector");
    let host = root.host_str().unwrap_or("").to_owned();

    let mut queue: VecDeque<Url> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();

    queue.push_back(root);

    while let Some(url) = queue.pop_front() {
        if visited.len() >= max_pages {
            break;
        }
        let canonical = canonical_url(&url);
        if !visited.insert(canonical) {
            continue;
        }

        debug!(url = %url, "crawling page");
        let html = match fetch_html(http, &url).await {
            Ok(h) => h,
            Err(e) => {
                warn!(url = %url, error = %e, "skipping page");
                continue;
            }
        };

        // Convert and save the page.
        let markdown = html_to_markdown(&html);
        let path = url_to_path(&url, &out_dir);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(LoreError::Io)?;
        }
        tokio::fs::write(&path, markdown.as_bytes()).await.map_err(LoreError::Io)?;

        // Enqueue links on the same host.
        let doc = Html::parse_document(&html);
        for link_el in doc.select(&link_selector) {
            if let Some(href) = link_el.value().attr("href") {
                if let Ok(abs) = url.join(href) {
                    if abs.host_str() == Some(host.as_str()) && should_crawl(&abs) {
                        let canon = canonical_url(&abs);
                        if !visited.contains(&canon) {
                            queue.push_back(abs);
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

async fn fetch_html(http: &reqwest::Client, url: &Url) -> Result<String, LoreError> {
    let resp = http
        .get(url.as_str())
        .send()
        .await
        .map_err(|e| LoreError::Registry(e.to_string()))?
        .error_for_status()
        .map_err(|e| LoreError::Registry(e.to_string()))?;

    resp.text().await.map_err(|e| LoreError::Registry(e.to_string()))
}

fn html_to_markdown(html: &str) -> String {
    htmd::convert(html).unwrap_or_else(|_| html.to_owned())
}

/// Strip fragment and query from a URL so we don't visit the same page twice.
fn canonical_url(url: &Url) -> String {
    let mut u = url.clone();
    u.set_fragment(None);
    u.set_query(None);
    u.to_string()
}

/// Returns `false` for non-HTML resource extensions we don't want to crawl.
fn should_crawl(url: &Url) -> bool {
    let path = url.path();
    let skip_exts = [
        ".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".pdf", ".zip", ".tar", ".gz", ".woff",
        ".woff2", ".ttf", ".eot", ".css", ".js", ".json", ".xml",
    ];
    !skip_exts.iter().any(|ext| path.ends_with(ext))
}

/// Map a URL to a `.md` file path inside `out_dir`.
fn url_to_path(url: &Url, out_dir: &std::path::Path) -> PathBuf {
    // Use the URL path segments as directory structure.
    let path = url.path().trim_start_matches('/');
    let stem = if path.is_empty() || path == "/" {
        "index".to_owned()
    } else {
        path.replace('/', "__")
    };
    // Ensure we don't overwrite non-.md files accidentally.
    let stem = if std::path::Path::new(&stem)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
    {
        stem[..stem.rfind('.').unwrap_or(stem.len())].to_owned()
    } else {
        stem
    };
    out_dir.join(format!("{stem}.md"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn website_source_builder() {
        let src = WebsiteSource::new("https://docs.example.com").with_max_pages(100);
        assert_eq!(src.root_url, "https://docs.example.com");
        assert_eq!(src.max_pages, 100);
    }

    #[test]
    fn canonical_url_strips_fragment_and_query() {
        let url: Url = "https://docs.example.com/guide?ref=nav#section"
            .parse()
            .unwrap();
        assert_eq!(canonical_url(&url), "https://docs.example.com/guide");
    }

    #[test]
    fn should_not_crawl_image() {
        let url: Url = "https://docs.example.com/logo.png".parse().unwrap();
        assert!(!should_crawl(&url));
    }

    #[test]
    fn url_to_path_index() {
        let url: Url = "https://docs.example.com/".parse().unwrap();
        let path = url_to_path(&url, std::path::Path::new("/tmp"));
        assert_eq!(path, std::path::PathBuf::from("/tmp/index.md"));
    }

    #[test]
    fn url_to_path_nested() {
        let url: Url = "https://docs.example.com/guide/install".parse().unwrap();
        let path = url_to_path(&url, std::path::Path::new("/tmp"));
        assert_eq!(path, std::path::PathBuf::from("/tmp/guide__install.md"));
    }
}
