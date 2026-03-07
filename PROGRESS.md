# 📋 Progress Plan: Round 2 Review Fix'leri

> Created: 2026-03-04 | Status: ✅ Complete | Completed: 8/8

## 🎯 Objective
Multi-review Round 2 consensus bulgularını fix etmek.

## 📊 Progress Overview
- Total tasks: 8
- Completed: 8
- In Progress: 0
- Remaining: 0

---

## Tasks

### Phase 1: Critical Fixes

- [x] **Task 1.1**: Fix mutex altında async sleep — `RateLimiter::wait()` ikiye böl
  - Files: `crw-crawl/src/crawl.rs`
  - Details: `next_sleep()` → mutex bırak → `tokio::time::sleep()` dışarıda

- [x] **Task 1.2**: Fix `unwrap_or_else` fallback → `expect()` (SSRF korumasız client oluşmasını engelle)
  - Files: `crw-crawl/src/crawl.rs`

- [x] **Task 1.3**: Sitemap URL'lerini SSRF kontrolünden geçir
  - Files: `crw-crawl/src/crawl.rs`

- [x] **Task 1.4**: MCP proxy client'a `safe_redirect_policy` + timeout ekle
  - Files: `crw-mcp/src/main.rs`

### Phase 2: Warning Fixes

- [x] **Task 2.1**: Rate limiter TTL cleanup threshold'u kaldır (her 64 çağrıda bir yap)
  - Files: `crw-crawl/src/crawl.rs`

- [x] **Task 2.2**: Rate limiter RPS çakışmasında warn logla
  - Files: `crw-crawl/src/crawl.rs`

- [x] **Task 2.3**: CDP — hata durumunda orphan target cleanup
  - Files: `crw-renderer/src/cdp.rs`

### Phase 3: Verification

- [x] Run `cargo fmt`, `cargo clippy`, `cargo test` — 247 test ✅, clippy clean, fmt clean

---

## 📝 Notes & Decisions
| # | Note | Date |
|---|------|------|
| 1 | `RateLimiter::wait()` → `next_sleep()` olarak rename edildi, sleep dışarıda yapılıyor | 2026-03-04 |
| 2 | TTL cleanup amortized: her 64 çağrıda bir `AtomicU64` counter ile | 2026-03-04 |
| 3 | Sitemap SSRF fix'i `let-chains` (`if let Ok(..) && ..`) syntax'ı kullanıyor | 2026-03-04 |

## 🐛 Issues Encountered
| # | Issue | Status | Resolution |
|---|-------|--------|------------|
| 1 | clippy `manual_is_multiple_of` | ✅ Fixed | `% 64 == 0` → `.is_multiple_of(64)` |
| 2 | clippy `collapsible_if` (2 instance) | ✅ Fixed | Nested if → `&&` ile birleştirildi |

---

## ✅ Completion Summary
- **Started**: 2026-03-04
- **Completed**: 2026-03-04
- **Total tasks**: 8 (8 original + 0 added)
- **Issues encountered**: 2 (clippy warnings, hepsi fix edildi)
- **Tests passing**: ✅ All (247/247)

### Key Changes Made
1. `crw-crawl/src/crawl.rs`: Mutex-safe rate limiting, SSRF sitemap validation, expect() fix, TTL cleanup, RPS warn
2. `crw-renderer/src/cdp.rs`: Orphan target cleanup on attach/evaluate errors
3. `crw-mcp/src/main.rs`: Safe redirect policy + timeouts on proxy client
