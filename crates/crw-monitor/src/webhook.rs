//! HMAC-SHA256 signed local webhook delivery.
//!
//! Signature scheme (matches the SaaS, §4.7): the header
//! `X-CRW-Signature: t=<unix>,v1=<hex>` carries an HMAC-SHA256 over the string
//! `"<t>.<body>"` keyed by the monitor's webhook secret. Receivers recompute
//! the MAC over the raw body to verify authenticity and freshness.
//!
//! Self-host is single-tenant and operator-owned, so the SSRF allow/deny-list
//! the SaaS enforces is out of scope here (the operator controls both the
//! monitor config and the receiver). A `note` to that effect is left for
//! anyone hardening a multi-tenant self-host deployment.

use crate::types::{CheckResult, WebhookConfig};
use crate::{MonitorError, MonitorResult};

/// Compute the `v1` HMAC-SHA256 hex signature over `"<t>.<body>"`.
#[cfg(feature = "webhook")]
pub fn sign(secret: &str, t: i64, body: &str) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    type HmacSha256 = Hmac<Sha256>;

    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");
    mac.update(format!("{t}.").as_bytes());
    mac.update(body.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Build the `X-CRW-Signature` header value for `body` at time `t`.
#[cfg(feature = "webhook")]
pub fn signature_header(secret: &str, t: i64, body: &str) -> String {
    format!("t={t},v1={}", sign(secret, t, body))
}

/// Deliver a check result to a monitor's webhook, signed with HMAC-SHA256.
/// Best-effort: returns an error on transport failure or a non-2xx response.
#[cfg(feature = "webhook")]
pub async fn deliver(
    client: &reqwest::Client,
    webhook: &WebhookConfig,
    result: &CheckResult,
) -> MonitorResult<()> {
    let body = serde_json::to_string(result)
        .map_err(|e| MonitorError::Webhook(format!("serialize check result: {e}")))?;
    let t = now_unix();
    let sig = signature_header(&webhook.secret, t, &body);

    let resp = client
        .post(&webhook.url)
        .header("Content-Type", "application/json")
        .header("X-CRW-Signature", sig)
        .body(body)
        .send()
        .await
        .map_err(|e| MonitorError::Webhook(format!("send: {e}")))?;

    if !resp.status().is_success() {
        return Err(MonitorError::Webhook(format!(
            "non-2xx response: {}",
            resp.status()
        )));
    }
    Ok(())
}

#[cfg(not(feature = "webhook"))]
pub async fn deliver(
    _client: &reqwest::Client,
    _webhook: &WebhookConfig,
    _result: &CheckResult,
) -> MonitorResult<()> {
    Err(MonitorError::Webhook(
        "webhook feature disabled at compile time".into(),
    ))
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// SMTP email delivery is **deferred** (documented stub).
///
/// TODO(monitor): wire SMTP/SES email notifications. This balloons scope (TLS,
/// SMTP AUTH, MIME multipart, bounce/suppression handling, double-opt-in
/// confirm tokens) and is intentionally out of the M6 core. Self-host operators
/// who want email today should point the HMAC webhook at a small relay that
/// turns the JSON payload into an email.
pub struct EmailStub;

impl EmailStub {
    /// Always returns an unimplemented error; present so the call site exists.
    pub fn send(_to: &str, _result: &CheckResult) -> MonitorResult<()> {
        Err(MonitorError::Webhook(
            "SMTP email delivery is not implemented (deferred); use the HMAC webhook".into(),
        ))
    }
}

#[cfg(all(test, feature = "webhook"))]
mod tests {
    use super::*;

    #[test]
    fn signature_is_stable_and_keyed() {
        let body = r#"{"hello":"world"}"#;
        let a = sign("secret-a", 1000, body);
        let b = sign("secret-b", 1000, body);
        // Deterministic for a fixed (secret, t, body).
        assert_eq!(a, sign("secret-a", 1000, body));
        // Different key → different signature.
        assert_ne!(a, b);
        // Header shape.
        let h = signature_header("secret-a", 1000, body);
        assert!(h.starts_with("t=1000,v1="));
        assert!(h.contains(&a));
    }

    #[test]
    fn known_vector() {
        // HMAC-SHA256("k", "1.x") — recomputed to lock the wire format.
        let got = sign("k", 1, "x");
        // length of a sha256 hex digest
        assert_eq!(got.len(), 64);
        // recompute independently
        use hmac::{Hmac, KeyInit, Mac};
        use sha2::Sha256;
        let mut mac = Hmac::<Sha256>::new_from_slice(b"k").unwrap();
        mac.update(b"1.x");
        assert_eq!(got, hex::encode(mac.finalize().into_bytes()));
    }
}
