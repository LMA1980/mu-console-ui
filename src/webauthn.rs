// SPDX-License-Identifier: AGPL-3.0-only
//! Pure WebAuthn base64url and JSON envelope helpers shared across native tests and `wasm32` boot.

use std::fmt::Write as _;

const B64URL_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encode raw bytes into unpadded URL-safe Base64 (`base64url`).
#[must_use]
pub fn b64url_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n =
            (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8) | u32::from(bytes[i + 2]);
        out.push(B64URL_TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(B64URL_TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(B64URL_TABLE[((n >> 6) & 0x3f) as usize] as char);
        out.push(B64URL_TABLE[(n & 0x3f) as usize] as char);
        i += 3;
    }
    match bytes.len() - i {
        1 => {
            let n = u32::from(bytes[i]) << 16;
            out.push(B64URL_TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(B64URL_TABLE[((n >> 12) & 0x3f) as usize] as char);
        }
        2 => {
            let n = (u32::from(bytes[i]) << 16) | (u32::from(bytes[i + 1]) << 8);
            out.push(B64URL_TABLE[((n >> 18) & 0x3f) as usize] as char);
            out.push(B64URL_TABLE[((n >> 12) & 0x3f) as usize] as char);
            out.push(B64URL_TABLE[((n >> 6) & 0x3f) as usize] as char);
        }
        _ => {}
    }
    out
}

/// Decode Base64 or URL-safe Base64 (with or without padding) into raw bytes.
#[must_use]
pub fn b64url_decode(input: &str) -> Vec<u8> {
    let mut vals = Vec::with_capacity(input.len());
    for b in input.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => continue,
        };
        vals.push(v);
    }
    let mut out = Vec::with_capacity((vals.len() * 3) / 4);
    let mut idx = 0;
    while idx + 4 <= vals.len() {
        let n = (u32::from(vals[idx]) << 18)
            | (u32::from(vals[idx + 1]) << 12)
            | (u32::from(vals[idx + 2]) << 6)
            | u32::from(vals[idx + 3]);
        out.push(((n >> 16) & 0xff) as u8);
        out.push(((n >> 8) & 0xff) as u8);
        out.push((n & 0xff) as u8);
        idx += 4;
    }
    match vals.len() - idx {
        2 => {
            let n = (u32::from(vals[idx]) << 18) | (u32::from(vals[idx + 1]) << 12);
            out.push(((n >> 16) & 0xff) as u8);
        }
        3 => {
            let n = (u32::from(vals[idx]) << 18)
                | (u32::from(vals[idx + 1]) << 12)
                | (u32::from(vals[idx + 2]) << 6);
            out.push(((n >> 16) & 0xff) as u8);
            out.push(((n >> 8) & 0xff) as u8);
        }
        _ => {}
    }
    out
}

/// Escape an arbitrary Rust string for safe inclusion inside a JSON string literal (RFC 8259).
#[must_use]
pub fn escape_json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str(r#"\""#),
            '\\' => out.push_str(r#"\\"#),
            '\u{08}' => out.push_str(r"\b"),
            '\u{0C}' => out.push_str(r"\f"),
            '\n' => out.push_str(r"\n"),
            '\r' => out.push_str(r"\r"),
            '\t' => out.push_str(r"\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

/// Build the JSON request body for `POST /api/auth/passkeys/register/start`.
#[must_use]
pub fn register_start_body(
    label: Option<&str>,
    step_up_challenge_id: Option<&str>,
    step_up_credential_json: Option<&str>,
) -> String {
    let mut fields = Vec::new();
    if let Some(lbl) = label.map(str::trim).filter(|s| !s.is_empty()) {
        fields.push(format!(r#""label":"{}""#, escape_json_str(lbl)));
    }
    if let (Some(cid), Some(cred_json)) = (
        step_up_challenge_id
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        step_up_credential_json
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    ) {
        fields.push(format!(
            r#""step_up_challenge_id":"{}""#,
            escape_json_str(cid)
        ));
        fields.push(format!(r#""step_up_credential":{cred_json}"#));
    }
    format!("{{{}}}", fields.join(","))
}

/// Build the JSON request body for `POST /api/auth/elevate/start` targeting `role_id`.
#[must_use]
pub fn elevate_start_body(role_id: &str) -> String {
    format!(r#"{{"role_id":"{}"}}"#, escape_json_str(role_id.trim()))
}

/// Build the JSON request body for `POST /api/auth/roles/{role_id}/renew/start` targeting `role_id`.
#[must_use]
pub fn renew_start_body(role_id: &str) -> String {
    elevate_start_body(role_id)
}

/// Canonical JSON request body for `POST /api/auth/elevate/start` with `passkey_manage` purpose.
#[must_use]
pub const fn passkey_manage_start_body() -> &'static str {
    r#"{"purpose":"passkey_manage"}"#
}

/// Wrap a serialized WebAuthn credential JSON object alongside its single-use `challenge_id`.
#[must_use]
pub fn wrap_credential_envelope_json(challenge_id: Option<&str>, credential_json: &str) -> String {
    match challenge_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(cid) => format!(
            r#"{{"challenge_id":"{}","credential":{credential_json}}}"#,
            escape_json_str(cid)
        ),
        None => format!(r#"{{"credential":{credential_json}}}"#),
    }
}

/// Normalize a browser WebAuthn `DOMException` or rejection payload into concise UI error copy.
#[must_use]
pub fn format_webauthn_js_error(name: Option<&str>, message: Option<&str>) -> String {
    let clean_name = name.map(str::trim).filter(|s| !s.is_empty());
    let clean_msg = message.map(str::trim).filter(|s| !s.is_empty());
    if matches!(clean_name, Some("NotAllowedError" | "AbortError")) {
        return "Passkey verification was cancelled or timed out.".to_owned();
    }
    match (clean_name, clean_msg) {
        (Some(n), Some(m)) => format!("{n}: {m}"),
        (_, Some(m)) => m.to_owned(),
        (Some(n), _) => n.to_owned(),
        (None, None) => "WebAuthn ceremony rejected".to_owned(),
    }
}

fn extract_json_string_field(json: &str, key: &str) -> Option<String> {
    let needle = format!(r#""{key}""#);
    let mut rest = json;
    while let Some(pos) = rest.find(&needle) {
        let after_key = rest[pos + needle.len()..].trim_start();
        if let Some(after_colon) = after_key.strip_prefix(':') {
            let val = after_colon.trim_start();
            if let Some(inner) = val.strip_prefix('"') {
                let mut out = String::new();
                let mut chars = inner.chars();
                while let Some(ch) = chars.next() {
                    match ch {
                        '"' => {
                            let trimmed = out.trim();
                            return (!trimmed.is_empty()).then(|| trimmed.to_owned());
                        }
                        '\\' => match chars.next() {
                            Some('"') => out.push('"'),
                            Some('\\') => out.push('\\'),
                            Some('/') => out.push('/'),
                            Some('b') => out.push('\u{08}'),
                            Some('f') => out.push('\u{0C}'),
                            Some('n') => out.push('\n'),
                            Some('r') => out.push('\r'),
                            Some('t') => out.push('\t'),
                            Some(other) => out.push(other),
                            None => return None,
                        },
                        other => out.push(other),
                    }
                }
            }
        }
        rest = &rest[pos + needle.len()..];
    }
    None
}

/// Extract a concise error message from a failed WebAuthn start HTTP response body.
#[must_use]
pub fn extract_webauthn_http_error(status: u16, url: &str, body: Option<&str>) -> String {
    if let Some(raw) = body.map(str::trim).filter(|s| !s.is_empty()) {
        if raw.starts_with('{') {
            if let Some(msg) = extract_json_string_field(raw, "error")
                .or_else(|| extract_json_string_field(raw, "message"))
            {
                return msg;
            }
        }
        if !raw.starts_with('<') && !raw.starts_with('{') {
            return raw.to_owned();
        }
    }
    if status == 0 {
        format!("Network error calling {url}")
    } else {
        format!("HTTP {status} from {url}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64url_roundtrip_handles_all_remainders() {
        for len in 0..=16 {
            let input: Vec<u8> = (0..len).map(|i| ((i * 37 + 11) & 0xff) as u8).collect();
            let encoded = b64url_encode(&input);
            assert!(!encoded.contains('='));
            assert!(!encoded.contains('+'));
            assert!(!encoded.contains('/'));
            assert_eq!(b64url_decode(&encoded), input);
        }
        assert_eq!(b64url_decode("SGVsbG8td29ybGQ_"), b"Hello-world?");
    }

    #[test]
    fn escape_json_str_escapes_control_and_quote_chars() {
        let raw = "a\"b\\c\n\r\t\u{08}\u{0c}\u{0001}z";
        assert_eq!(escape_json_str(raw), r#"a\"b\\c\n\r\t\b\f\u0001z"#);
    }

    #[test]
    fn register_start_body_formats_optional_label_and_step_up() {
        assert_eq!(register_start_body(None, None, None), "{}");
        assert_eq!(register_start_body(Some("  "), None, None), "{}");
        assert_eq!(
            register_start_body(Some("  YubiKey \"5\" "), None, None),
            r#"{"label":"YubiKey \"5\""}"#
        );
        assert_eq!(
            register_start_body(
                Some("Backup"),
                Some("ch-9"),
                Some(r#"{"id":"c1","type":"public-key"}"#),
            ),
            r#"{"label":"Backup","step_up_challenge_id":"ch-9","step_up_credential":{"id":"c1","type":"public-key"}}"#
        );
    }

    #[test]
    fn elevate_and_passkey_manage_bodies_and_envelope_format_cleanly() {
        assert_eq!(
            elevate_start_body("platform-admin"),
            r#"{"role_id":"platform-admin"}"#
        );
        assert_eq!(
            renew_start_body("super-operator"),
            r#"{"role_id":"super-operator"}"#
        );
        assert_eq!(
            passkey_manage_start_body(),
            r#"{"purpose":"passkey_manage"}"#
        );
        assert_eq!(
            wrap_credential_envelope_json(Some("ch-1"), r#"{"id":"pk1"}"#),
            r#"{"challenge_id":"ch-1","credential":{"id":"pk1"}}"#
        );
        assert_eq!(
            wrap_credential_envelope_json(Some("  "), r#"{"id":"pk1"}"#),
            r#"{"credential":{"id":"pk1"}}"#
        );
        assert_eq!(
            wrap_credential_envelope_json(None, r#"{"id":"pk1"}"#),
            r#"{"credential":{"id":"pk1"}}"#
        );
    }

    #[test]
    fn webauthn_error_formatting_and_http_extraction_handle_cancellation_and_json() {
        assert_eq!(
            format_webauthn_js_error(Some("NotAllowedError"), Some("The operation timed out")),
            "Passkey verification was cancelled or timed out."
        );
        assert_eq!(
            format_webauthn_js_error(Some("AbortError"), None),
            "Passkey verification was cancelled or timed out."
        );
        assert_eq!(
            format_webauthn_js_error(Some("SecurityError"), Some("Invalid RP ID")),
            "SecurityError: Invalid RP ID"
        );
        assert_eq!(
            extract_webauthn_http_error(
                403,
                "/api/auth/elevate/start",
                Some(r#"{"error":"Role \"super-operator\" is not bound"}"#),
            ),
            r#"Role "super-operator" is not bound"#
        );
        assert_eq!(
            extract_webauthn_http_error(
                400,
                "/api/auth/elevate/start",
                Some(r#"{"message":"Step-up challenge expired"}"#),
            ),
            "Step-up challenge expired"
        );
        assert_eq!(
            extract_webauthn_http_error(502, "/api/auth/elevate/start", Some("<html>502</html>")),
            "HTTP 502 from /api/auth/elevate/start"
        );
    }
}
