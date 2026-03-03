use crw_core::types::*;
use serde_json::json;

#[test]
fn output_format_serde_roundtrip() {
    let variants = vec![
        (OutputFormat::Markdown, "\"markdown\""),
        (OutputFormat::Html, "\"html\""),
        (OutputFormat::RawHtml, "\"rawHtml\""),
        (OutputFormat::PlainText, "\"plainText\""),
        (OutputFormat::Links, "\"links\""),
        (OutputFormat::Json, "\"json\""),
    ];

    for (variant, expected_json) in variants {
        let serialized = serde_json::to_string(&variant).unwrap();
        assert_eq!(
            serialized, expected_json,
            "Serialize failed for {variant:?}"
        );

        let deserialized: OutputFormat = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, variant, "Deserialize failed for {variant:?}");
    }
}

#[test]
fn scrape_request_default_formats() {
    let req: ScrapeRequest = serde_json::from_str(r#"{"url":"https://example.com"}"#).unwrap();
    assert_eq!(req.url, "https://example.com");
    assert_eq!(req.formats, vec![OutputFormat::Markdown]);
    assert!(req.only_main_content);
    assert!(req.render_js.is_none());
    assert!(req.wait_for.is_none());
    assert!(req.include_tags.is_empty());
    assert!(req.exclude_tags.is_empty());
    assert!(req.json_schema.is_none());
    assert!(req.headers.is_empty());
}

#[test]
fn api_response_ok_serialization() {
    let resp = ApiResponse::ok("hello");
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["success"], true);
    assert_eq!(json["data"], "hello");
    assert!(json.get("error").is_none());
}

#[test]
fn api_response_err_serialization() {
    let resp = ApiResponse::<()>::err("something went wrong");
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["success"], false);
    assert!(json.get("data").is_none());
    assert_eq!(json["error"], "something went wrong");
}

#[test]
fn crawl_status_serde_rename() {
    let cases = vec![
        (CrawlStatus::InProgress, "\"scraping\""),
        (CrawlStatus::Completed, "\"completed\""),
        (CrawlStatus::Failed, "\"failed\""),
    ];

    for (status, expected) in cases {
        let serialized = serde_json::to_string(&status).unwrap();
        assert_eq!(serialized, expected);
        let deserialized: CrawlStatus = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, status);
    }
}

#[test]
fn page_metadata_skip_serializing_none() {
    let meta = PageMetadata {
        title: Some("Test".into()),
        description: None,
        og_title: None,
        og_description: None,
        og_image: None,
        canonical_url: None,
        source_url: "https://example.com".into(),
        language: None,
        status_code: 200,
        rendered_with: None,
        elapsed_ms: 100,
    };

    let json = serde_json::to_value(&meta).unwrap();
    assert_eq!(json["title"], "Test");
    // None fields with skip_serializing_if should not be present
    assert!(json.get("ogTitle").is_none());
    assert!(json.get("ogDescription").is_none());
    assert!(json.get("ogImage").is_none());
    assert!(json.get("canonicalUrl").is_none());
    assert!(json.get("language").is_none());
    assert!(json.get("renderedWith").is_none());
    // Required fields always present
    assert_eq!(json["sourceURL"], "https://example.com");
    assert_eq!(json["statusCode"], 200);
}

#[test]
fn crawl_request_default_formats() {
    let req: CrawlRequest = serde_json::from_str(r#"{"url":"https://example.com"}"#).unwrap();
    assert_eq!(req.formats, vec![OutputFormat::Markdown]);
    assert!(req.only_main_content);
    assert!(req.max_depth.is_none());
    assert!(req.max_pages.is_none());
}

#[test]
fn crawl_start_response_serialization() {
    let resp = CrawlStartResponse {
        success: true,
        id: "abc-123".into(),
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["success"], true);
    assert_eq!(json["id"], "abc-123");
}

#[test]
fn map_request_defaults() {
    let req: MapRequest = serde_json::from_str(r#"{"url":"https://example.com"}"#).unwrap();
    assert!(req.use_sitemap);
    assert!(req.max_depth.is_none());
}

#[test]
fn scrape_data_skip_serializing_none() {
    let data = ScrapeData {
        markdown: Some("# Hello".into()),
        html: None,
        raw_html: None,
        plain_text: None,
        links: None,
        json: None,
        metadata: PageMetadata {
            title: None,
            description: None,
            og_title: None,
            og_description: None,
            og_image: None,
            canonical_url: None,
            source_url: "https://example.com".into(),
            language: None,
            status_code: 200,
            rendered_with: None,
            elapsed_ms: 50,
        },
    };

    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["markdown"], "# Hello");
    assert!(json.get("html").is_none());
    assert!(json.get("rawHtml").is_none());
    assert!(json.get("plainText").is_none());
    assert!(json.get("links").is_none());
    assert!(json.get("json").is_none());
}

#[test]
fn scrape_request_with_all_fields() {
    let input = json!({
        "url": "https://example.com",
        "formats": ["markdown", "html", "links"],
        "onlyMainContent": false,
        "renderJs": true,
        "waitFor": 2000,
        "includeTags": ["article"],
        "excludeTags": ["nav"],
        "jsonSchema": {"type": "object"},
        "headers": {"X-Custom": "value"}
    });

    let req: ScrapeRequest = serde_json::from_value(input).unwrap();
    assert_eq!(req.formats.len(), 3);
    assert!(!req.only_main_content);
    assert_eq!(req.render_js, Some(true));
    assert_eq!(req.wait_for, Some(2000));
    assert_eq!(req.include_tags, vec!["article"]);
    assert_eq!(req.exclude_tags, vec!["nav"]);
    assert!(req.json_schema.is_some());
    assert_eq!(req.headers.get("X-Custom").unwrap(), "value");
}
