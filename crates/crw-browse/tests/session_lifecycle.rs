//! Session registry lifecycle test (no real browser needed).
//!
//! Exercises the session bookkeeping paths: insert → primary & secondary
//! indexes agree, close_session drops both, TTL sweep removes stale entries.
//!
//! Because creating a real `CdpConnection` requires a CDP WebSocket peer, this
//! test spins up a minimal echo WebSocket server on a loopback port using
//! `tokio-tungstenite` and points the connection at it. The server replies
//! nothing — the tests only exercise the registry's bookkeeping, not any CDP
//! round-trip.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crw_browse::session::SessionRegistry;
use crw_renderer::cdp_conn::CdpConnection;
use futures::SinkExt;
use futures::StreamExt;
use tokio::net::TcpListener;

async fn spawn_mock_ws() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                if let Ok(ws) = tokio_tungstenite::accept_async(stream).await {
                    let (mut write, mut read) = ws.split();
                    // Consume client messages forever; don't reply.
                    while let Some(msg) = read.next().await {
                        if msg.is_err() {
                            break;
                        }
                    }
                    let _ = write.close().await;
                }
            });
        }
    });
    format!("ws://{addr}")
}

#[tokio::test]
async fn insert_and_close_keeps_indexes_consistent() {
    let ws_url = spawn_mock_ws().await;
    let conn = Arc::new(
        CdpConnection::connect(&ws_url, Duration::from_secs(3))
            .await
            .expect("connect"),
    );
    let registry = SessionRegistry::new();
    let session = registry.insert(conn).expect("insert");

    assert_eq!(registry.len(), 1);
    assert!(registry.get(&session.id).is_some());
    assert!(registry.get_by_short(&session.short_id).is_some());

    let sid = session.id;
    drop(session);
    registry.close_session(sid).await;

    assert!(registry.is_empty());
}

#[tokio::test]
async fn unique_short_ids_across_many_inserts() {
    // Allocate a batch of sessions and ensure all short_ids are unique.
    let ws_url = spawn_mock_ws().await;
    let registry = SessionRegistry::new();
    let mut short_ids = std::collections::HashSet::new();

    for _ in 0..20 {
        let conn = Arc::new(
            CdpConnection::connect(&ws_url, Duration::from_secs(3))
                .await
                .expect("connect"),
        );
        let session = registry.insert(conn).expect("insert");
        assert!(
            short_ids.insert(session.short_id.clone()),
            "duplicate short_id {:?}",
            session.short_id
        );
    }
}

#[tokio::test]
async fn ttl_sweep_removes_only_expired_sessions() {
    // Deterministic: no wall-clock sleep. Both sessions share a 60-second TTL
    // so neither "naturally" expires during the test. We forcibly age session
    // A's `last_used` by an hour; B stays fresh. A sweep must take A and
    // leave B. This exercises the actual TTL threshold (not just "all or
    // nothing"), which a broken `> 0`-style comparison wouldn't survive.
    let ws_url = spawn_mock_ws().await;
    let registry = Arc::new(SessionRegistry::with_ttl(Duration::from_secs(60)));

    let conn_a = Arc::new(
        CdpConnection::connect(&ws_url, Duration::from_secs(3))
            .await
            .expect("connect A"),
    );
    let session_a = registry.insert(conn_a).expect("insert A");
    let a_id = session_a.id;

    let conn_b = Arc::new(
        CdpConnection::connect(&ws_url, Duration::from_secs(3))
            .await
            .expect("connect B"),
    );
    let session_b = registry.insert(conn_b).expect("insert B");
    let b_id = session_b.id;

    // Age A deterministically — bypass any wall clock.
    {
        let mut last = session_a.last_used.write().await;
        *last = Instant::now() - Duration::from_secs(3600);
    }

    assert_eq!(registry.len(), 2, "both sessions present pre-sweep");

    registry.sweep_expired().await;

    assert!(
        registry.get(&a_id).is_none(),
        "A should be evicted (last_used aged past ttl)"
    );
    assert!(
        registry.get(&b_id).is_some(),
        "B should survive (last_used within ttl)"
    );
    assert_eq!(registry.len(), 1, "exactly one session remaining");
}
