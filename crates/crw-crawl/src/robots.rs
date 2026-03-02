use crw_core::error::{CrwError, CrwResult};

/// Simple robots.txt parser.
#[derive(Debug, Clone)]
pub struct RobotsTxt {
    disallowed: Vec<String>,
    pub sitemaps: Vec<String>,
}

impl RobotsTxt {
    pub async fn fetch(base_url: &str, client: &reqwest::Client) -> CrwResult<Self> {
        let url = format!(
            "{}/robots.txt",
            base_url.trim_end_matches('/')
        );

        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| CrwError::HttpError(e.to_string()))?;

        if !resp.status().is_success() {
            return Ok(Self {
                disallowed: vec![],
                sitemaps: vec![],
            });
        }

        let text = resp
            .text()
            .await
            .map_err(|e| CrwError::HttpError(e.to_string()))?;

        Ok(Self::parse(&text))
    }

    pub fn parse(text: &str) -> Self {
        let mut disallowed = Vec::new();
        let mut sitemaps = Vec::new();
        let mut in_our_section = false;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let lower = line.to_lowercase();

            if let Some(agent) = directive_value(&lower, "user-agent:") {
                in_our_section = agent == "*" || agent.contains("crw");
                continue;
            }

            if let Some(url) = directive_value(line, "sitemap:") {
                if !url.is_empty() {
                    sitemaps.push(url.to_string());
                }
                continue;
            }

            if in_our_section {
                if let Some(path) = directive_value(line, "disallow:") {
                    if !path.is_empty() {
                        disallowed.push(path.to_string());
                    }
                }
            }
        }

        Self {
            disallowed,
            sitemaps,
        }
    }

    /// Check if a path is allowed.
    pub fn is_allowed(&self, path: &str) -> bool {
        !self.disallowed.iter().any(|d| path.starts_with(d))
    }
}

/// Safely extract the value after a directive prefix (case-insensitive match).
fn directive_value<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let lower = line.to_lowercase();
    if lower.starts_with(prefix) {
        Some(line[prefix.len()..].trim())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_robots_txt() {
        let text = r#"
User-agent: *
Disallow: /admin/
Disallow: /private/

Sitemap: https://example.com/sitemap.xml
"#;
        let robots = RobotsTxt::parse(text);
        assert!(!robots.is_allowed("/admin/page"));
        assert!(robots.is_allowed("/public/page"));
        assert_eq!(robots.sitemaps, vec!["https://example.com/sitemap.xml"]);
    }

    #[test]
    fn handles_edge_cases() {
        // Empty lines, no content after directive
        let text = "User-agent:\nDisallow:\nSitemap:\n";
        let robots = RobotsTxt::parse(text);
        assert!(robots.is_allowed("/anything"));
        assert!(robots.sitemaps.is_empty());
    }
}
