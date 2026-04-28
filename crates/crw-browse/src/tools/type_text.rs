//! `type_text` — type characters into the currently focused element by
//! dispatching one `Input.dispatchKeyEvent` triple (`keyDown`+`char`+`keyUp`)
//! per character.
//!
//! Why not use `Input.insertText` like Chrome's "type" actions do? Because
//! Lightpanda accepts `Input.insertText` and returns success without ever
//! mutating the element's value — so we'd silently drop the text. The
//! `dispatchKeyEvent` path round-trips through the keyboard event pipeline
//! and works correctly on both backends.
//!
//! This tool does NOT focus the element first — call `click` (or use a
//! browser-native pattern) before `type_text` to ensure the right element
//! is focused. Splitting focus and typing keeps the API composable.

use std::time::{Duration, Instant};

use rmcp::{ErrorData as McpError, model::CallToolResult, schemars};
use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, ErrorResponse};
use crate::response::ToolResponse;
use crate::server::CrwBrowse;
use crate::tools::common::{
    MAX_TIMEOUT_MS, MAX_TYPE_TEXT_LEN, clamp_timeout, err_result, no_session_err, no_target_err,
    ok_result, resolve_ref,
};

/// Cap on the per-character delay so a misuser can't pin a session for hours.
const MAX_DELAY_MS: u64 = 1_000;

/// Control characters we map to named `KeyboardEvent.key` values. Anything
/// else goes through the printable path (single-codepoint string, `char`
/// event with `text`/`unmodifiedText`).
fn is_control_char(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\t' | '\x08' | '\x1b')
}

/// `(key, code, windowsVirtualKeyCode)` triple for a control char, or `None`
/// for printable characters. VK codes follow the WHATWG/Windows convention:
/// Enter=13, Tab=9, Backspace=8, Escape=27.
fn control_key_metadata(ch: char) -> Option<(&'static str, &'static str, u32)> {
    match ch {
        '\n' | '\r' => Some(("Enter", "Enter", 13)),
        '\t' => Some(("Tab", "Tab", 9)),
        '\x08' => Some(("Backspace", "Backspace", 8)),
        '\x1b' => Some(("Escape", "Escape", 27)),
        _ => None,
    }
}

/// Build the `Input.dispatchKeyEvent` payload for a single keystroke. `kind`
/// is one of `"keyDown"`, `"char"`, `"keyUp"`. Pulled out of `handle()` so
/// the CDP wire shape can be unit-tested without a live session and so the
/// tests cannot drift from the production code path.
fn build_key_event_params(ch: char, kind: &str) -> serde_json::Value {
    let s = ch.to_string();

    if kind == "char" {
        // `text`/`unmodifiedText` belong only on the `char` event.
        // Chromium treats `keyDown` with `text` as text input and
        // synthesizes its own internal `char` — sending an explicit
        // `char` afterwards then double-inserts the character ("a"
        // becomes "aa"). Carrying `text` only on `char` keeps the
        // pipeline single-insert across both Chrome and Lightpanda.
        return serde_json::json!({
            "type": kind,
            "text": s,
            "unmodifiedText": s,
        });
    }

    if let Some((key_name, code_name, vk_code)) = control_key_metadata(ch) {
        let mut p = serde_json::json!({
            "type": kind,
            "key": key_name,
            "code": code_name,
            "windowsVirtualKeyCode": vk_code,
            "nativeVirtualKeyCode": vk_code,
        });
        // Enter is the only control key that fires a `keypress` in real
        // browsers — vanilla-JS pages (TodoMVC, jQuery 1.x) hook
        // `keypress` and check `event.keyCode === 13`. CDP only emits
        // keypress when the `keyDown` event carries a `text` field;
        // without it Chromium's InputDispatcher emits keydown+keyup but
        // skips keypress. So for Enter we include `text: "\r"` on the
        // `keyDown` event. Tab/Backspace/Escape stay text-less because
        // they don't produce keypress in real keyboards either.
        if kind == "keyDown" && matches!(ch, '\n' | '\r') {
            p["text"] = serde_json::Value::String("\r".into());
            p["unmodifiedText"] = serde_json::Value::String("\r".into());
        }
        return p;
    }

    serde_json::json!({
        "type": kind,
        "key": s,
    })
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct TypeTextInput {
    /// Text to type. UTF-8; capped at 4096 bytes.
    pub text: String,
    /// Per-character delay in ms (default 0; capped at 1000). Useful when
    /// the page-side handler debounces typed input and zero-delay typing
    /// fires events too fast for the listener to settle.
    #[serde(default)]
    pub delay_ms: Option<u64>,
    /// Optional `@e<N>` ref of the element to focus before typing. Without
    /// this, the caller is responsible for focus (typically by calling
    /// `click` first). With it, `type_text` issues `DOM.focus` before the
    /// keyboard event loop — matches the chromedp belt-and-braces pattern
    /// and makes the tool work on pages where `click` doesn't transfer
    /// focus (no autofocus, focus stolen by overlay, etc.).
    #[serde(default, rename = "ref")]
    pub ref_id: Option<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct TypeTextData {
    pub typed: usize,
}

pub async fn handle(server: &CrwBrowse, input: TypeTextInput) -> Result<CallToolResult, McpError> {
    let started = Instant::now();
    if input.text.is_empty() {
        return Ok(err_result(&ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "text must not be empty",
        )));
    }
    if input.text.len() > MAX_TYPE_TEXT_LEN {
        return Ok(err_result(&ErrorResponse::new(
            ErrorCode::InvalidArgs,
            format!("text exceeds maximum length of {MAX_TYPE_TEXT_LEN} bytes"),
        )));
    }

    let (timeout, timeout_clamped) = clamp_timeout(input.timeout_ms, server.config().page_timeout);
    let delay = Duration::from_millis(input.delay_ms.unwrap_or(0).min(MAX_DELAY_MS));

    let Some(session) = server.default_session_get().await else {
        return Ok(err_result(&no_session_err()));
    };
    let Some(cdp_sid) = session.cdp_session_id().await else {
        return Ok(err_result(&no_target_err()));
    };

    // Optional belt-and-braces focus: if the caller passed a ref, focus the
    // element via `DOM.focus` before dispatching keys. Element.click() does
    // NOT focus per WHATWG; click() in our path now calls .focus() too, but
    // a caller that fills without clicking first (e.g. autofocus moved away
    // or fill-only flows) still needs this. Failure here is a hard error —
    // typing into the wrong element would silently corrupt page state.
    if let Some(ref_id) = input.ref_id.as_deref() {
        let backend_id = match resolve_ref(&session, ref_id).await {
            Ok(id) => id,
            Err(e) => return Ok(err_result(&e)),
        };
        if let Err(e) = session
            .conn
            .send_recv(
                "DOM.focus",
                serde_json::json!({ "backendNodeId": backend_id }),
                Some(&cdp_sid),
                timeout,
            )
            .await
        {
            let lower = e.to_string().to_ascii_lowercase();
            // Reuse the same stale-vs-real-error split as ref_to_object_id.
            // "Element is not focusable" is a distinct case — surface it as
            // NODE_NOT_CLICKABLE so the caller picks a focusable target.
            let resp = if lower.contains("not focusable") {
                ErrorResponse::new(
                    ErrorCode::NodeNotClickable,
                    format!(
                        "ref {ref_id} is not focusable — pick an input/textarea/contenteditable"
                    ),
                )
            } else if lower.contains("does not belong to the document")
                || lower.contains("could not find node")
                || lower.contains("no node with given id")
                || lower.contains("node with given id")
            {
                ErrorResponse::new(
                    ErrorCode::NodeStale,
                    format!("ref {ref_id} no longer attached — call `tree` again"),
                )
                .with_retry(crate::errors::RetryHint::Snapshot)
            } else {
                ErrorResponse::new(ErrorCode::CdpError, format!("DOM.focus failed: {e}"))
            };
            return Ok(err_result(&resp));
        }
    }

    // Global deadline shared across all CDP round-trips. Without this, a 100-
    // char string at 200ms each could blow past the caller's `timeout_ms`
    // budget even though every individual call respected it.
    let deadline = Instant::now() + timeout;
    let mut typed = 0usize;
    for ch in input.text.chars() {
        // CDP wants the character as a string for `text` — UTF-8 codepoints
        // larger than the BMP are sent verbatim and the renderer handles
        // surrogate splitting on its end. `key` is also populated so Chrome's
        // `KeyboardEvent.key` listener (the modern, recommended path) fires
        // — Lightpanda ignores it, but Chrome relies on it.
        // Map control characters to their `KeyboardEvent.key` names so
        // listeners that branch on `event.key === "Enter"` actually fire.
        // Sending `key: "\n"` makes Chrome dispatch a keydown whose `.key`
        // is the literal newline, which no real handler matches.
        //
        // Why also set `code` and `windowsVirtualKeyCode`: legacy listeners
        // (vanilla TodoMVC, jQuery 1.x form pages) check `event.keyCode ===
        // 13` rather than the modern `event.key === "Enter"`. Without
        // `windowsVirtualKeyCode`, Chrome's KeyboardEvent pipeline emits
        // `keyCode = 0` and the legacy branch never fires.
        //
        // Real keyboards do not produce a `char` event for non-printable
        // keys (Enter, Tab, Backspace, Escape). CDP follows the same
        // contract — sending a `char` event for `\n` makes Chrome
        // double-dispatch and produces a stray keyDown whose
        // `KeyboardEvent.charCode` is the raw 0x0a value. Skip the
        // `char` step entirely for control characters.
        let is_control = is_control_char(ch);
        let events: &[&str] = if is_control {
            &["keyDown", "keyUp"]
        } else {
            &["keyDown", "char", "keyUp"]
        };
        // Build the keyUp release params once so the timeout-leak and
        // char-failure paths can both reuse the same shape.
        let release_params = || build_key_event_params(ch, "keyUp");
        let mut down_sent = false;
        for kind in events {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                // If keyDown succeeded but the deadline expired before
                // keyUp, fire a best-effort keyUp so the renderer
                // doesn't see the key as still held down on the next
                // tool call. 500ms cap is independent of the call's
                // exhausted budget — releasing a stuck key is worth
                // the small overshoot.
                if down_sent {
                    let _ = session
                        .conn
                        .send_recv(
                            "Input.dispatchKeyEvent",
                            release_params(),
                            Some(&cdp_sid),
                            Duration::from_millis(500),
                        )
                        .await;
                }
                return Ok(err_result(
                    &ErrorResponse::timeout(timeout.as_millis() as u64).with_partial_count(typed),
                ));
            }
            let params = build_key_event_params(ch, kind);
            let result = session
                .conn
                .send_recv("Input.dispatchKeyEvent", params, Some(&cdp_sid), remaining)
                .await;
            match (*kind, result) {
                ("keyDown", Ok(_)) => {
                    down_sent = true;
                }
                ("keyDown", Err(e)) => {
                    return Ok(err_result(
                        &ErrorResponse::new(
                            ErrorCode::CdpError,
                            format!("Input.dispatchKeyEvent(keyDown) failed: {e}"),
                        )
                        .with_partial_count(typed),
                    ));
                }
                ("char", Err(e)) => {
                    if down_sent {
                        let _ = session
                            .conn
                            .send_recv(
                                "Input.dispatchKeyEvent",
                                release_params(),
                                Some(&cdp_sid),
                                Duration::from_millis(500),
                            )
                            .await;
                    }
                    return Ok(err_result(
                        &ErrorResponse::new(
                            ErrorCode::CdpError,
                            format!("Input.dispatchKeyEvent(char) failed: {e}"),
                        )
                        .with_partial_count(typed),
                    ));
                }
                ("keyUp", Err(e)) => {
                    // keyDown (and `char` for printables) already
                    // succeeded — the page almost certainly observed
                    // the character. Reporting `typed + 1` reflects
                    // that page-side state advanced even though the
                    // protocol-level release failed.
                    return Ok(err_result(
                        &ErrorResponse::new(
                            ErrorCode::CdpError,
                            format!("Input.dispatchKeyEvent(keyUp) failed: {e}"),
                        )
                        .with_partial_count(typed + 1),
                    ));
                }
                _ => {}
            }
        }
        typed += 1;
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }

    let mut payload = ToolResponse::new(
        &session.short_id,
        session.last_url().await,
        TypeTextData { typed },
    )
    .with_elapsed_ms(started.elapsed().as_millis() as u64);
    if timeout_clamped {
        payload = payload.with_warning(format!(
            "timeout_ms clamped to {MAX_TIMEOUT_MS} ms (server-side cap)"
        ));
    }
    Ok(ok_result(&payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_control_chars() {
        assert!(is_control_char('\n'));
        assert!(is_control_char('\r'));
        assert!(is_control_char('\t'));
        assert!(is_control_char('\x08'));
        assert!(is_control_char('\x1b'));
        assert!(!is_control_char('a'));
        assert!(!is_control_char(' '));
        assert!(!is_control_char('é'));
        assert!(!is_control_char('中'));
    }

    #[test]
    fn maps_enter_to_vk_13() {
        assert_eq!(control_key_metadata('\n'), Some(("Enter", "Enter", 13)));
        assert_eq!(control_key_metadata('\r'), Some(("Enter", "Enter", 13)));
    }

    #[test]
    fn maps_tab_to_vk_9() {
        assert_eq!(control_key_metadata('\t'), Some(("Tab", "Tab", 9)));
    }

    #[test]
    fn maps_backspace_to_vk_8() {
        assert_eq!(
            control_key_metadata('\x08'),
            Some(("Backspace", "Backspace", 8))
        );
    }

    #[test]
    fn maps_escape_to_vk_27() {
        assert_eq!(control_key_metadata('\x1b'), Some(("Escape", "Escape", 27)));
    }

    #[test]
    fn printable_chars_have_no_metadata() {
        assert_eq!(control_key_metadata('a'), None);
        assert_eq!(control_key_metadata('Z'), None);
        assert_eq!(control_key_metadata(' '), None);
        assert_eq!(control_key_metadata('é'), None);
        assert_eq!(control_key_metadata('中'), None);
        assert_eq!(control_key_metadata('🎉'), None);
    }

    // R1 (test-quality review) flagged a test-only mirror of the param
    // builder as drift-prone. Tests below now drive the real production
    // function `build_key_event_params` directly.

    #[test]
    fn enter_keydown_carries_carriage_return_text() {
        let p = build_key_event_params('\n', "keyDown");
        assert_eq!(p["type"], "keyDown");
        assert_eq!(p["key"], "Enter");
        assert_eq!(p["code"], "Enter");
        assert_eq!(p["windowsVirtualKeyCode"], 13);
        assert_eq!(p["nativeVirtualKeyCode"], 13);
        // Critical: keypress only fires when keyDown carries `text`.
        assert_eq!(p["text"], "\r");
        assert_eq!(p["unmodifiedText"], "\r");
    }

    #[test]
    fn tab_keydown_has_no_text() {
        let p = build_key_event_params('\t', "keyDown");
        assert_eq!(p["key"], "Tab");
        assert_eq!(p["windowsVirtualKeyCode"], 9);
        assert!(p.get("text").is_none(), "tab keyDown must not carry text");
    }

    #[test]
    fn backspace_keydown_has_no_text() {
        let p = build_key_event_params('\x08', "keyDown");
        assert_eq!(p["key"], "Backspace");
        assert_eq!(p["windowsVirtualKeyCode"], 8);
        assert!(p.get("text").is_none());
    }

    #[test]
    fn escape_keydown_has_no_text() {
        let p = build_key_event_params('\x1b', "keyDown");
        assert_eq!(p["key"], "Escape");
        assert_eq!(p["windowsVirtualKeyCode"], 27);
        assert!(p.get("text").is_none());
    }

    #[test]
    fn printable_keydown_has_only_key() {
        let p = build_key_event_params('a', "keyDown");
        assert_eq!(p["type"], "keyDown");
        assert_eq!(p["key"], "a");
        assert!(p.get("text").is_none());
        assert!(p.get("windowsVirtualKeyCode").is_none());
    }

    #[test]
    fn char_event_carries_text_for_printable() {
        let p = build_key_event_params('z', "char");
        assert_eq!(p["type"], "char");
        assert_eq!(p["text"], "z");
        assert_eq!(p["unmodifiedText"], "z");
        assert!(p.get("key").is_none());
    }

    #[test]
    fn keyup_for_control_carries_no_text() {
        let p = build_key_event_params('\n', "keyUp");
        assert_eq!(p["type"], "keyUp");
        assert_eq!(p["key"], "Enter");
        assert_eq!(p["windowsVirtualKeyCode"], 13);
        // keyUp must NEVER carry `text` — char already delivered the
        // printable rendition; repeating it on keyUp is meaningless and
        // duplicates input on some Chromium pipelines.
        assert!(p.get("text").is_none(), "keyUp must not carry text");
    }

    #[test]
    fn keyup_for_printable_is_minimal() {
        let p = build_key_event_params('a', "keyUp");
        assert_eq!(p["type"], "keyUp");
        assert_eq!(p["key"], "a");
        assert!(p.get("text").is_none());
        assert!(p.get("windowsVirtualKeyCode").is_none());
    }

    #[test]
    fn unhandled_low_ascii_is_treated_as_printable() {
        // R1 (test-quality review) noted that `\x00`-`\x07`, `\x0b`, etc.
        // fall through the printable path. Document and lock that
        // contract: the byte is sent verbatim as `key`, no VK code, no
        // event-type weirdness. If we ever add NUL/DEL to the control
        // set, this test fails and forces a deliberate decision.
        let p = build_key_event_params('\x00', "keyDown");
        assert_eq!(p["type"], "keyDown");
        assert_eq!(p["key"], "\u{0}");
        assert!(p.get("windowsVirtualKeyCode").is_none());
    }
}
