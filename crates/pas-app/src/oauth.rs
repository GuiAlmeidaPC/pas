//! ChatGPT "Sign in with ChatGPT" OAuth support.
//!
//! Implements the PKCE OAuth flow used by the OpenAI Codex CLI: a loopback
//! HTTP server on port 1455 receives the redirect, the auth code is exchanged
//! for access/refresh tokens, and those tokens are used against the Codex
//! Responses API. Tokens are persisted by the caller in an AES-GCM-encrypted
//! file keyed by a machine-derived key (obfuscation only — see DIVERGENCE.md).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::AiMessage;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const CALLBACK_PORT: u16 = 1455;
const SCOPES: &str = "openid profile email offline_access";
const RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";

const B64URL: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

// ---------------------------------------------------------------------------
// PKCE
// ---------------------------------------------------------------------------

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

pub fn generate_pkce() -> Pkce {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = B64URL.encode(bytes);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = B64URL.encode(digest);
    Pkce { verifier, challenge }
}

// ---------------------------------------------------------------------------
// id_token claim extraction
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct IdClaims {
    pub account_id: String,
    pub email: Option<String>,
}

pub fn parse_id_token(jwt: &str) -> Result<IdClaims, String> {
    let payload_b64 = jwt.split('.').nth(1).ok_or("id_token missing payload")?;
    let bytes = B64URL
        .decode(payload_b64)
        .map_err(|e| format!("id_token base64: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("id_token json: {e}"))?;
    // JSON Pointer escapes `/` as `~1` and `~` as `~0`, so the claim key
    // `https://api.openai.com/auth` becomes `https:~1~1api.openai.com~1auth`.
    let account_id = v
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(|x| x.as_str())
        .ok_or("id_token missing chatgpt_account_id")?
        .to_string();
    let email = v.get("email").and_then(|x| x.as_str()).map(String::from);
    Ok(IdClaims { account_id, email })
}

// ---------------------------------------------------------------------------
// Encrypted token storage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub account_id: String,
    pub email: Option<String>,
    pub expires_at_unix: u64,
}

/// Obfuscation-only key derived from a stable per-install path. NOT strong
/// at-rest protection — documented in DIVERGENCE.md.
pub fn derive_key(app_data_dir: &Path) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"pas-chatgpt-oauth-v1");
    hasher.update(app_data_dir.to_string_lossy().as_bytes());
    hasher.finalize().into()
}

pub fn encrypt_tokens(key: &[u8; 32], tokens: &StoredTokens) -> Result<Vec<u8>, String> {
    use rand::RngCore;
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let plaintext = serde_json::to_vec(tokens).map_err(|e| e.to_string())?;
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(|e| format!("encrypt: {e}"))?;
    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn decrypt_tokens(key: &[u8; 32], blob: &[u8]) -> Result<StoredTokens, String> {
    if blob.len() < 12 {
        return Err("token blob too short".into());
    }
    let (nonce_bytes, ct) = blob.split_at(12);
    let cipher = Aes256Gcm::new(key.into());
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|e| format!("decrypt: {e}"))?;
    serde_json::from_slice(&plaintext).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Login orchestration: loopback server, browser open, exchange, refresh
// ---------------------------------------------------------------------------

fn open_browser(url: &str) {
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();
}

pub fn authorize_url(pkce: &Pkce, state: &str) -> String {
    let mut u = format!("{AUTHORIZE_URL}?response_type=code&client_id={CLIENT_ID}");
    u.push_str(&format!("&redirect_uri={}", urlencode(REDIRECT_URI)));
    u.push_str(&format!("&scope={}", urlencode(SCOPES)));
    u.push_str(&format!(
        "&code_challenge={}&code_challenge_method=S256",
        pkce.challenge
    ));
    u.push_str(&format!("&state={state}&prompt=login"));
    u
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{:02X}", b),
        })
        .collect()
}

fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(b);
                    i += 3;
                    continue;
                }
                out.push(b'%');
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn random_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut b);
    B64URL.encode(b)
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_callback_query(path: &str) -> (Option<String>, Option<String>) {
    let query = path.splitn(2, '?').nth(1).unwrap_or("");
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        let mut it = pair.splitn(2, '=');
        match (it.next(), it.next()) {
            (Some("code"), Some(v)) => code = Some(urldecode(v)),
            (Some("state"), Some(v)) => state = Some(urldecode(v)),
            _ => {}
        }
    }
    (code, state)
}

/// Blocks until the browser redirects to the callback, returns the auth code.
/// Validates `state`. Times out after 5 minutes.
fn wait_for_code(expected_state: &str) -> Result<String, String> {
    let listener = TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .map_err(|e| format!("port {CALLBACK_PORT} unavailable: {e}"))?;
    let deadline = SystemTime::now() + Duration::from_secs(300);
    for stream in listener.incoming() {
        if SystemTime::now() > deadline {
            return Err("login timed out".into());
        }
        let mut stream = stream.map_err(|e| e.to_string())?;
        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
        let req = String::from_utf8_lossy(&buf[..n]);
        let path = req
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("");
        if !path.starts_with("/auth/callback") {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            continue;
        }
        let (code, state) = parse_callback_query(path);
        let body = "<html><body style='font-family:sans-serif'><h3>PAS sign-in complete.</h3><p>You can close this tab and return to PAS.</p></body></html>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        let _ = stream.write_all(resp.as_bytes());
        if state.as_deref() != Some(expected_state) {
            return Err("state mismatch".into());
        }
        return code.ok_or_else(|| "callback missing code".into());
    }
    Err("callback server closed".into())
}

async fn tokens_from_response(res: reqwest::Response) -> Result<StoredTokens, String> {
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("token endpoint {}: {}", status.as_u16(), text));
    }
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let access_token = v
        .get("access_token")
        .and_then(|x| x.as_str())
        .ok_or("no access_token")?
        .to_string();
    let refresh_token = v
        .get("refresh_token")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let id_token = v.get("id_token").and_then(|x| x.as_str()).unwrap_or("");
    let expires_in = v.get("expires_in").and_then(|x| x.as_u64()).unwrap_or(3600);
    let claims = parse_id_token(id_token)?;
    Ok(StoredTokens {
        access_token,
        refresh_token,
        account_id: claims.account_id,
        email: claims.email,
        expires_at_unix: now_unix() + expires_in,
    })
}

async fn exchange_code(
    client: &reqwest::Client,
    code: &str,
    verifier: &str,
) -> Result<StoredTokens, String> {
    let res = client
        .post(TOKEN_URL)
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "client_id": CLIENT_ID,
            "code": code,
            "redirect_uri": REDIRECT_URI,
            "code_verifier": verifier,
        }))
        .send()
        .await
        .map_err(|e| format!("token request: {e}"))?;
    tokens_from_response(res).await
}

pub async fn refresh(
    client: &reqwest::Client,
    refresh_token: &str,
) -> Result<StoredTokens, String> {
    let res = client
        .post(TOKEN_URL)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": CLIENT_ID,
            "refresh_token": refresh_token,
        }))
        .send()
        .await
        .map_err(|e| format!("refresh request: {e}"))?;
    tokens_from_response(res).await
}

/// Full interactive login. Opens browser, waits for loopback callback,
/// exchanges the code. Returns fresh tokens.
pub async fn interactive_login() -> Result<StoredTokens, String> {
    let pkce = generate_pkce();
    let state = random_token();
    let url = authorize_url(&pkce, &state);
    open_browser(&url);
    let verifier = pkce.verifier.clone();
    let code = tokio::task::spawn_blocking(move || wait_for_code(&state))
        .await
        .map_err(|e| e.to_string())??;
    let client = reqwest::Client::new();
    exchange_code(&client, &code, &verifier).await
}

/// Returns a valid access token, refreshing in place if it expires within 5
/// minutes. The caller is responsible for persisting `tokens` if it changed.
pub async fn valid_access_token(
    tokens: &mut StoredTokens,
    client: &reqwest::Client,
) -> Result<String, String> {
    if tokens.expires_at_unix <= now_unix() + 300 {
        let refreshed = refresh(client, &tokens.refresh_token).await?;
        // A refresh response may omit refresh_token; keep the old one if so.
        let keep_refresh = if refreshed.refresh_token.is_empty() {
            tokens.refresh_token.clone()
        } else {
            refreshed.refresh_token.clone()
        };
        *tokens = StoredTokens {
            refresh_token: keep_refresh,
            ..refreshed
        };
    }
    Ok(tokens.access_token.clone())
}

// ---------------------------------------------------------------------------
// Codex Responses API
// ---------------------------------------------------------------------------

pub fn build_responses_body(
    model: &str,
    system_prompt: &str,
    history: &[AiMessage],
) -> serde_json::Value {
    let input: Vec<_> = history
        .iter()
        .map(|m| {
            let content_type = if m.role == "assistant" {
                "output_text"
            } else {
                "input_text"
            };
            serde_json::json!({
                "role": if m.role == "assistant" { "assistant" } else { "user" },
                "content": [{ "type": content_type, "text": m.content }],
            })
        })
        .collect();
    serde_json::json!({
        "model": model,
        "instructions": system_prompt,
        "input": input,
        "stream": true,
        "store": false,
    })
}

/// Accumulate `response.output_text.delta` events from an SSE stream body.
pub fn parse_responses_sse(body: &str) -> Result<String, String> {
    let mut out = String::new();
    for line in body.lines() {
        let line = line.trim_start();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data == "[DONE]" || data.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        // Codex backend emits {"delta": "..."} on output_text.delta events.
        if let Some(d) = v.get("delta").and_then(|x| x.as_str()) {
            out.push_str(d);
        }
    }
    if out.is_empty() {
        return Err("Responses API returned no text".into());
    }
    Ok(out)
}

/// One call to the Codex Responses API. Streams SSE server-side and returns
/// the fully concatenated text, preserving the UI's non-streaming contract.
pub async fn responses_completion(
    client: &reqwest::Client,
    access_token: &str,
    account_id: &str,
    body: &serde_json::Value,
) -> Result<String, String> {
    let res = client
        .post(RESPONSES_URL)
        .header("Authorization", format!("Bearer {access_token}"))
        .header("chatgpt-account-id", account_id)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .json(body)
        .send()
        .await
        .map_err(|e| format!("Responses request: {e}"))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Responses API {}: {}", status.as_u16(), text));
    }
    parse_responses_sse(&text)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_sha256_of_verifier() {
        let p = generate_pkce();
        let expected = B64URL.encode(Sha256::digest(p.verifier.as_bytes()));
        assert_eq!(p.challenge, expected);
        assert!(!p.verifier.contains('=') && !p.verifier.contains('+'));
    }

    #[test]
    fn extracts_account_id_and_email_from_id_token() {
        let payload = serde_json::json!({
            "email": "user@example.com",
            "https://api.openai.com/auth": { "chatgpt_account_id": "acct_123" }
        });
        let body = B64URL.encode(serde_json::to_vec(&payload).unwrap());
        let jwt = format!("h.{}.s", body);
        let claims = parse_id_token(&jwt).unwrap();
        assert_eq!(claims.account_id, "acct_123");
        assert_eq!(claims.email.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn token_blob_round_trips_through_encryption() {
        let key = derive_key(Path::new("/home/u/.local/share/pas"));
        let tokens = StoredTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            account_id: "acct_1".into(),
            email: Some("u@e.com".into()),
            expires_at_unix: 1_900_000_000,
        };
        let blob = encrypt_tokens(&key, &tokens).unwrap();
        let back = decrypt_tokens(&key, &blob).unwrap();
        assert_eq!(back.access_token, "at");
        assert_eq!(back.email.as_deref(), Some("u@e.com"));
        assert_eq!(back.expires_at_unix, 1_900_000_000);
    }

    #[test]
    fn decrypt_with_wrong_key_fails() {
        let key = derive_key(Path::new("/a"));
        let other = derive_key(Path::new("/b"));
        let tokens = StoredTokens {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            account_id: "acct".into(),
            email: None,
            expires_at_unix: 0,
        };
        let blob = encrypt_tokens(&key, &tokens).unwrap();
        assert!(decrypt_tokens(&other, &blob).is_err());
    }

    #[test]
    fn parses_callback_code_and_state() {
        let (code, state) = parse_callback_query("/auth/callback?code=abc&state=xyz");
        assert_eq!(code.as_deref(), Some("abc"));
        assert_eq!(state.as_deref(), Some("xyz"));
    }

    #[test]
    fn urlencode_roundtrips_reserved() {
        assert_eq!(urldecode(&urlencode("a b/c?d")), "a b/c?d");
    }

    #[test]
    fn authorize_url_has_pkce_and_state() {
        let p = Pkce {
            verifier: "v".into(),
            challenge: "chal".into(),
        };
        let u = authorize_url(&p, "st");
        assert!(u.contains("code_challenge=chal"));
        assert!(u.contains("code_challenge_method=S256"));
        assert!(u.contains("state=st"));
        assert!(u.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
    }

    #[test]
    fn sse_accumulates_output_text_deltas() {
        let sse = "\
event: response.output_text.delta
data: {\"delta\":\"Hello\"}

event: response.output_text.delta
data: {\"delta\":\", world\"}

event: response.completed
data: {\"response\":{\"status\":\"completed\"}}

";
        assert_eq!(parse_responses_sse(sse).unwrap(), "Hello, world");
    }

    #[test]
    fn build_responses_body_maps_roles() {
        let history = vec![
            AiMessage {
                role: "user".into(),
                content: "hi".into(),
            },
            AiMessage {
                role: "assistant".into(),
                content: "yo".into(),
            },
        ];
        let body = build_responses_body("gpt-5.5", "sys", &history);
        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(body["instructions"], "sys");
        assert_eq!(body["input"][0]["role"], "user");
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][1]["content"][0]["type"], "output_text");
    }
}
