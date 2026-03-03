use crw_core::config::*;

#[test]
fn server_config_default_values() {
    let config = ServerConfig::default();
    assert_eq!(config.host, "0.0.0.0");
    assert_eq!(config.port, 3000);
    assert_eq!(config.request_timeout_secs, 60);
}

#[test]
fn renderer_config_default_values() {
    let config = RendererConfig::default();
    assert_eq!(config.mode, "auto");
    assert_eq!(config.page_timeout_ms, 30000);
    assert_eq!(config.pool_size, 4);
    assert!(config.lightpanda.is_none());
    assert!(config.playwright.is_none());
    assert!(config.chrome.is_none());
}

#[test]
fn crawler_config_default_values() {
    let config = CrawlerConfig::default();
    assert_eq!(config.max_concurrency, 10);
    assert!((config.requests_per_second - 10.0).abs() < f64::EPSILON);
    assert!(config.respect_robots_txt);
    assert_eq!(config.user_agent, "CRW/0.1");
    assert_eq!(config.default_max_depth, 2);
    assert_eq!(config.default_max_pages, 100);
    assert!(config.proxy.is_none());
    assert_eq!(config.job_ttl_secs, 3600);
}

#[test]
fn extraction_config_default_values() {
    let config = ExtractionConfig::default();
    assert_eq!(config.default_format, "markdown");
    assert!(config.only_main_content);
    assert!(config.llm.is_none());
}

#[test]
fn auth_config_default_empty() {
    let config = AuthConfig::default();
    assert!(config.api_keys.is_empty());
}

#[test]
fn server_config_deserialize_partial() {
    let toml_str = r#"
        port = 8080
    "#;
    let config: ServerConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.port, 8080);
    // host should fallback to default
    assert_eq!(config.host, "0.0.0.0");
    assert_eq!(config.request_timeout_secs, 60);
}

#[test]
fn crawler_config_deserialize_partial() {
    let toml_str = r#"
        max_concurrency = 20
        requests_per_second = 5.0
    "#;
    let config: CrawlerConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.max_concurrency, 20);
    assert!((config.requests_per_second - 5.0).abs() < f64::EPSILON);
    // defaults for the rest
    assert!(config.respect_robots_txt);
    assert_eq!(config.user_agent, "CRW/0.1");
}

#[test]
fn app_config_deserialize_empty() {
    let toml_str = "";
    let config: AppConfig = toml::from_str(toml_str).unwrap();
    assert_eq!(config.server.port, 3000);
    assert_eq!(config.renderer.mode, "auto");
    assert_eq!(config.crawler.max_concurrency, 10);
}
