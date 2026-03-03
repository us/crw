use crw_core::error::CrwError;

#[test]
fn error_display_all_variants() {
    let cases: Vec<(CrwError, &str)> = vec![
        (
            CrwError::HttpError("conn refused".into()),
            "HTTP request failed: conn refused",
        ),
        (
            CrwError::InvalidRequest("bad url".into()),
            "Invalid request: bad url",
        ),
        (
            CrwError::RendererError("cdp fail".into()),
            "Renderer error: cdp fail",
        ),
        (
            CrwError::ExtractionError("parse fail".into()),
            "Extraction error: parse fail",
        ),
        (
            CrwError::CrawlError("depth exceeded".into()),
            "Crawl error: depth exceeded",
        ),
        (CrwError::Timeout(5000), "Timeout after 5000ms"),
        (
            CrwError::ConfigError("missing key".into()),
            "Config error: missing key",
        ),
        (CrwError::NotFound("page 404".into()), "Not found: page 404"),
        (CrwError::Internal("oops".into()), "oops"),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected, "Failed for variant: {error:?}");
    }
}

#[test]
fn error_from_url_parse_error() {
    let url_err = url::Url::parse("not a url").unwrap_err();
    let crw_err: CrwError = url_err.into();
    let msg = crw_err.to_string();
    assert!(msg.contains("URL parse error"), "got: {msg}");
}

#[test]
fn error_debug_impl() {
    let err = CrwError::HttpError("test".into());
    let debug = format!("{err:?}");
    assert!(debug.contains("HttpError"));
}
