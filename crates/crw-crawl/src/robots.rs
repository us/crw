use crw_core::error::{CrwError, CrwResult};

/// A single robots.txt rule (Allow or Disallow).
#[derive(Debug, Clone)]
struct Rule {
    pattern: String,
    allow: bool,
}

/// Simple robots.txt parser with wildcard and Allow support.
#[derive(Debug, Clone)]
pub struct RobotsTxt {
    rules: Vec<Rule>,
    pub sitemaps: Vec<String>,
}

impl RobotsTxt {
    pub async fn fetch(base_url: &str, client: &reqwest::Client) -> CrwResult<Self> {
        let url = format!("{}/robots.txt", base_url.trim_end_matches('/'));

        let resp = client
            .get(&url)
            .send()
            .await
            .map_err(|e| CrwError::HttpError(e.to_string()))?;

        if !resp.status().is_success() {
            return Ok(Self {
                rules: vec![],
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
        let mut rules = Vec::new();
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
                        rules.push(Rule {
                            pattern: path.to_string(),
                            allow: false,
                        });
                    }
                } else if let Some(path) = directive_value(line, "allow:")
                    && !path.is_empty()
                {
                    rules.push(Rule {
                        pattern: path.to_string(),
                        allow: true,
                    });
                }
            }
        }

        Self { rules, sitemaps }
    }

    /// Check if a path is allowed using specificity-based matching.
    /// Per RFC 9309: the most specific (longest) pattern wins.
    /// Wildcard `*` and anchor `$` characters are excluded from length calculation.
    /// If equal effective length, Allow wins over Disallow.
    pub fn is_allowed(&self, path: &str) -> bool {
        let mut best_match: Option<&Rule> = None;
        let mut best_len: usize = 0;

        for rule in &self.rules {
            if matches_pattern(path, &rule.pattern) {
                let len = effective_pattern_len(&rule.pattern);
                if len > best_len || (len == best_len && rule.allow) {
                    best_len = len;
                    best_match = Some(rule);
                }
            }
        }

        match best_match {
            Some(rule) => rule.allow,
            None => true, // No matching rule means allowed
        }
    }
}

/// Effective pattern length for specificity calculation.
/// Excludes wildcard `*` and end-anchor `$` characters per RFC 9309.
fn effective_pattern_len(pattern: &str) -> usize {
    pattern.chars().filter(|&c| c != '*' && c != '$').count()
}

/// Simple glob matching for robots.txt patterns.
/// Supports `*` (any sequence of characters) and `$` (end of string).
fn matches_pattern(path: &str, pattern: &str) -> bool {
    let anchored_end = pattern.ends_with('$');
    let pattern = if anchored_end {
        &pattern[..pattern.len() - 1]
    } else {
        pattern
    };

    if !pattern.contains('*') {
        // Simple prefix match
        if anchored_end {
            path == pattern
        } else {
            path.starts_with(pattern)
        }
    } else {
        // Split by * and match segments in order
        let segments: Vec<&str> = pattern.split('*').collect();
        let mut pos = 0;

        for (i, segment) in segments.iter().enumerate() {
            if segment.is_empty() {
                continue;
            }
            if i == 0 {
                // First segment must match at start
                if !path[pos..].starts_with(segment) {
                    return false;
                }
                pos += segment.len();
            } else {
                // Subsequent segments can match anywhere after current position
                match path[pos..].find(segment) {
                    Some(idx) => pos += idx + segment.len(),
                    None => return false,
                }
            }
        }

        if anchored_end {
            pos == path.len()
        } else {
            true
        }
    }
}

/// Safely extract the value after a directive prefix (case-insensitive match).
fn directive_value<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let lower = line.to_lowercase();
    if lower.starts_with(prefix) {
        let value = line[prefix.len()..].trim();
        // Strip inline comments (e.g. "Disallow: /admin # admin panel")
        let value = value.split('#').next().unwrap_or(value).trim();
        Some(value)
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
        let text = "User-agent:\nDisallow:\nSitemap:\n";
        let robots = RobotsTxt::parse(text);
        assert!(robots.is_allowed("/anything"));
        assert!(robots.sitemaps.is_empty());
    }

    #[test]
    fn wildcard_pattern_matching() {
        let text = "User-agent: *\nDisallow: /*.pdf\n";
        let robots = RobotsTxt::parse(text);
        assert!(!robots.is_allowed("/document.pdf"));
        assert!(!robots.is_allowed("/path/to/file.pdf"));
        assert!(robots.is_allowed("/document.html"));
    }

    #[test]
    fn dollar_end_anchor() {
        let text = "User-agent: *\nDisallow: /*.pdf$\n";
        let robots = RobotsTxt::parse(text);
        assert!(!robots.is_allowed("/document.pdf"));
        assert!(robots.is_allowed("/document.pdf?query=1"));
    }

    #[test]
    fn allow_overrides_disallow() {
        let text = r#"
User-agent: *
Disallow: /private/
Allow: /private/public-page
"#;
        let robots = RobotsTxt::parse(text);
        assert!(!robots.is_allowed("/private/secret"));
        assert!(robots.is_allowed("/private/public-page"));
    }

    #[test]
    fn specificity_longer_pattern_wins() {
        let text = r#"
User-agent: *
Disallow: /
Allow: /public/
"#;
        let robots = RobotsTxt::parse(text);
        assert!(!robots.is_allowed("/private"));
        assert!(robots.is_allowed("/public/page"));
    }

    #[test]
    fn equal_length_allow_wins() {
        let text = r#"
User-agent: *
Disallow: /path
Allow: /path
"#;
        let robots = RobotsTxt::parse(text);
        assert!(robots.is_allowed("/path"));
    }
}
