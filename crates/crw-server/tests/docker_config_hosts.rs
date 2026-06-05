//! Regression guard for issue #90: every host referenced in `config.docker.toml`
//! must resolve to a service that the reference `docker-compose.yml` actually
//! defines. Issue #90 shipped a SaaS-only hostname (`searxng-internal`) in the
//! opencore default; that name has no service/alias on the single-bridge compose
//! network, so search was permanently broken out of the box.
//!
//! This test parses the TOML (we have the `toml` crate as a dev-dep) and checks
//! each renderer/search host against the known compose service names. We do NOT
//! parse the compose YAML — there's no YAML parser in-tree and the service list
//! is small and stable. If you add a compose service that a config host points
//! at, add it to `COMPOSE_SERVICE_NAMES` below.

use std::path::PathBuf;

/// Service names defined in `docker-compose.yml` / `docker-compose.stealth.yml`
/// that a config host is allowed to point at. `crw` itself is the app, not a
/// target host, so it's intentionally excluded.
///
/// MAINTENANCE CONTRACT: if you add a compose service that `config.docker.toml`
/// points a renderer/search URL at, add its service name here.
const COMPOSE_SERVICE_NAMES: &[&str] = &["searxng", "lightpanda", "chrome", "chrome-stealth"];

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is <repo>/crates/crw-server; go up two levels.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("crw-server should live at <repo>/crates/crw-server")
        .to_path_buf()
}

/// Extract the host from a `scheme://host:port/...` URL string.
fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .unwrap_or_else(|e| panic!("config.docker.toml URL `{url}` did not parse: {e}"))
        .host_str()
        .unwrap_or_else(|| panic!("config.docker.toml URL `{url}` has no host"))
        .to_string()
}

#[test]
fn docker_config_hosts_match_compose_services() {
    let config_path = repo_root().join("config.docker.toml");
    assert!(
        config_path.exists(),
        "expected config.docker.toml at {} — did the crate move relative to the repo root?",
        config_path.display()
    );

    let raw = std::fs::read_to_string(&config_path).expect("read config.docker.toml");
    let doc: toml::Value = toml::from_str(&raw).expect("parse config.docker.toml");

    // (config key path, the URL string) for every host-bearing field we ship.
    let mut hosts: Vec<(&str, String)> = Vec::new();

    let get = |table: &str, sub: &str, key: &str| -> Option<String> {
        doc.get(table)?
            .get(sub)?
            .get(key)?
            .as_str()
            .map(str::to_string)
    };

    if let Some(u) = get("renderer", "lightpanda", "ws_url") {
        hosts.push(("renderer.lightpanda.ws_url", u));
    }
    if let Some(u) = get("renderer", "chrome", "ws_url") {
        hosts.push(("renderer.chrome.ws_url", u));
    }
    if let Some(u) = doc
        .get("search")
        .and_then(|s| s.get("searxng_url"))
        .and_then(|v| v.as_str())
    {
        hosts.push(("search.searxng_url", u.to_string()));
    }

    assert!(
        !hosts.is_empty(),
        "no renderer/search host URLs found in config.docker.toml — did the schema change?"
    );

    for (field, url) in &hosts {
        let host = host_of(url);
        assert!(
            COMPOSE_SERVICE_NAMES.contains(&host.as_str()),
            "config.docker.toml `{field}` host '{host}' is not a docker-compose service name \
             (known: {COMPOSE_SERVICE_NAMES:?}). A SaaS-only or typo'd host leaked into the \
             opencore default — see issue #90. Either fix the host or, if you added a new compose \
             service, extend COMPOSE_SERVICE_NAMES in this test."
        );
    }
}
