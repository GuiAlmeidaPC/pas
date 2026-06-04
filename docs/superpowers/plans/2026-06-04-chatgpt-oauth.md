# ChatGPT OAuth Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add "Sign in with ChatGPT" OAuth as an alternative to the OpenAI API key in the PAS AI assistant, billing usage against the user's ChatGPT subscription via the Codex Responses API.

**Architecture:** All OAuth/token logic lives in Rust (`crates/pas-app/src/oauth.rs`): PKCE flow, a std `TcpListener` loopback callback on port 1455, token exchange/refresh, `id_token` claim extraction, and an AES-GCM-encrypted token file keyed by a machine-derived key. `lib.rs` gains commands + an `auth_mode` that routes the OpenAI path to the Codex Responses API (SSE collapsed to one string). The UI gets an auth-mode toggle and a sign-in button.

**Tech Stack:** Rust (Tauri 2, reqwest, tokio, std::net), new crates `sha2`, `base64`, `aes-gcm`, `rand`; React/TypeScript UI.

---

## File Structure

| File | Responsibility |
|------|----------------|
| `crates/pas-app/Cargo.toml` | Add `sha2`, `base64`, `aes-gcm`, `rand` deps. |
| `crates/pas-app/src/oauth.rs` | New: PKCE, loopback server, token exchange/refresh, id_token claims, encrypted token file, in-memory token state. |
| `crates/pas-app/src/lib.rs` | New commands; `auth_mode` on config/request; Responses-API branch + SSE parser; load tokens at startup. |
| `ui/src/AISettingsModal.tsx` | Auth-mode toggle, sign-in button, signed-in status, ChatGPT model defaults. |
| `ui/src/AIChatPanel.tsx` | Persist/restore `auth_mode`, pass it in the request, show status. |
| `CHANGELOG.md` | `[Unreleased]` entry. |
| `DIVERGENCE.md` | Note the hardcoded public client_id and obfuscation-only token-at-rest. |

OAuth constants (module-level in `oauth.rs`):
```rust
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const CALLBACK_PORT: u16 = 1455;
const SCOPES: &str = "openid profile email offline_access";
const RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
```

---

## Task 1: Dependencies + PKCE generation

**Files:**
- Modify: `crates/pas-app/Cargo.toml`
- Create: `crates/pas-app/src/oauth.rs`
- Modify: `crates/pas-app/src/lib.rs` (add `mod oauth;`)

- [ ] **Step 1: Add deps.** In `[dependencies]` of `crates/pas-app/Cargo.toml` add:
```toml
sha2 = "0.10"
base64 = "0.22"
aes-gcm = "0.10"
rand = "0.8"
```

- [ ] **Step 2: Create `oauth.rs` with PKCE + a failing test.**
```rust
use base64::Engine;
use sha2::{Digest, Sha256};

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const CALLBACK_PORT: u16 = 1455;
const SCOPES: &str = "openid profile email offline_access";
const RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";

const B64URL: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

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
}
```

- [ ] **Step 3: Wire module.** Add `mod oauth;` near the top of `crates/pas-app/src/lib.rs` (after the existing `use` block).

- [ ] **Step 4: Run.** `cargo test -p pas-app oauth::tests::pkce` — Expected: PASS (compiles, test green).

- [ ] **Step 5: Commit.** `git add -A && git commit -m "feat(oauth): add deps and PKCE generation"`

---

## Task 2: id_token claim extraction

**Files:** Modify `crates/pas-app/src/oauth.rs`

- [ ] **Step 1: Failing test** (append to `mod tests`):
```rust
#[test]
fn extracts_account_id_and_email_from_id_token() {
    // header.payload.signature — only payload matters.
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
```

- [ ] **Step 2: Run, expect fail** (`parse_id_token` undefined): `cargo test -p pas-app extracts_account_id`

- [ ] **Step 3: Implement** (module body):
```rust
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
    let account_id = v
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(|x| x.as_str())
        .ok_or("id_token missing chatgpt_account_id")?
        .to_string();
    let email = v.get("email").and_then(|x| x.as_str()).map(String::from);
    Ok(IdClaims { account_id, email })
}
```
Note: JSON Pointer escapes `/` as `~1` and `~` as `~0`, so the key `https://api.openai.com/auth` becomes `https:~1~1api.openai.com~1auth`.

- [ ] **Step 4: Run, expect pass.** `cargo test -p pas-app extracts_account_id`

- [ ] **Step 5: Commit.** `git commit -am "feat(oauth): parse id_token claims"`

---

## Task 3: Encrypted token storage

**Files:** Modify `crates/pas-app/src/oauth.rs`

- [ ] **Step 1: Failing test:**
```rust
#[test]
fn token_blob_round_trips_through_encryption() {
    let key = derive_key(std::path::Path::new("/home/u/.local/share/pas"));
    let tokens = StoredTokens {
        access_token: "at".into(),
        refresh_token: "rt".into(),
        account_id: "acct_1".into(),
        email: Some("u@e.com".into()),
        expires_at_unix: 1_900_000_000,
    };
    let blob = encrypt_tokens(&key, &tokens).unwrap();
    assert_ne!(blob.windows(2).count(), 0);
    let back = decrypt_tokens(&key, &blob).unwrap();
    assert_eq!(back.access_token, "at");
    assert_eq!(back.email.as_deref(), Some("u@e.com"));
    assert_eq!(back.expires_at_unix, 1_900_000_000);
}
```

- [ ] **Step 2: Run, expect fail.** `cargo test -p pas-app token_blob_round_trips`

- [ ] **Step 3: Implement:**
```rust
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
```

- [ ] **Step 4: Run, expect pass.** `cargo test -p pas-app token_blob_round_trips`

- [ ] **Step 5: Commit.** `git commit -am "feat(oauth): encrypted token storage"`

---

## Task 4: Login orchestration (loopback + token exchange + refresh)

**Files:** Modify `crates/pas-app/src/oauth.rs`

- [ ] **Step 1: Implement loopback + browser open + exchange + refresh** (no unit test — exercised manually; depends on network/browser). Add:
```rust
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn open_browser(url: &str) {
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
}

pub fn authorize_url(pkce: &Pkce, state: &str) -> String {
    let mut u = format!("{AUTHORIZE_URL}?response_type=code&client_id={CLIENT_ID}");
    u.push_str(&format!("&redirect_uri={}", urlencode(REDIRECT_URI)));
    u.push_str(&format!("&scope={}", urlencode(SCOPES)));
    u.push_str(&format!("&code_challenge={}&code_challenge_method=S256", pkce.challenge));
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

fn random_token() -> String {
    use rand::RngCore;
    let mut b = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut b);
    B64URL.encode(b)
}

/// Blocks until the browser redirects to the callback, returns the auth code.
/// Validates `state`. Times out after 5 minutes.
fn wait_for_code(expected_state: &str) -> Result<String, String> {
    let listener = TcpListener::bind(("127.0.0.1", CALLBACK_PORT))
        .map_err(|e| format!("port {CALLBACK_PORT} unavailable: {e}"))?;
    listener
        .set_nonblocking(false)
        .map_err(|e| e.to_string())?;
    let deadline = SystemTime::now() + Duration::from_secs(300);
    for stream in listener.incoming() {
        if SystemTime::now() > deadline {
            return Err("login timed out".into());
        }
        let mut stream = stream.map_err(|e| e.to_string())?;
        let mut buf = [0u8; 2048];
        let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
        let req = String::from_utf8_lossy(&buf[..n]);
        let path = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("");
        if !path.starts_with("/auth/callback") {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            continue;
        }
        let (code, state) = parse_callback_query(path);
        let body = "<html><body style='font-family:sans-serif'><h3>PAS sign-in complete.</h3><p>You can close this tab and return to PAS.</p></body></html>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
            body.len(), body
        );
        let _ = stream.write_all(resp.as_bytes());
        if state.as_deref() != Some(expected_state) {
            return Err("state mismatch".into());
        }
        return code.ok_or_else(|| "callback missing code".into());
    }
    Err("callback server closed".into())
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
            b'+' => { out.push(b' '); i += 1; }
            c => { out.push(c); i += 1; }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

async fn exchange_code(client: &reqwest::Client, code: &str, verifier: &str) -> Result<StoredTokens, String> {
    let res = client
        .post(TOKEN_URL)
        .json(&serde_json::json!({
            "grant_type": "authorization_code",
            "client_id": CLIENT_ID,
            "code": code,
            "redirect_uri": REDIRECT_URI,
            "code_verifier": verifier,
        }))
        .send().await.map_err(|e| format!("token request: {e}"))?;
    tokens_from_response(res).await
}

pub async fn refresh(client: &reqwest::Client, refresh_token: &str) -> Result<StoredTokens, String> {
    let res = client
        .post(TOKEN_URL)
        .json(&serde_json::json!({
            "grant_type": "refresh_token",
            "client_id": CLIENT_ID,
            "refresh_token": refresh_token,
        }))
        .send().await.map_err(|e| format!("refresh request: {e}"))?;
    tokens_from_response(res).await
}

async fn tokens_from_response(res: reqwest::Response) -> Result<StoredTokens, String> {
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("token endpoint {}: {}", status.as_u16(), text));
    }
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let access_token = v.get("access_token").and_then(|x| x.as_str()).ok_or("no access_token")?.to_string();
    let refresh_token = v.get("refresh_token").and_then(|x| x.as_str()).unwrap_or("").to_string();
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

/// Returns a valid access token, refreshing if it expires within 5 minutes.
/// Re-persists refreshed tokens via the provided closure.
pub async fn valid_access_token(
    tokens: &mut StoredTokens,
    client: &reqwest::Client,
) -> Result<String, String> {
    if tokens.expires_at_unix <= now_unix() + 300 {
        let refreshed = refresh(client, &tokens.refresh_token).await?;
        // refresh may omit refresh_token; keep the old one if so.
        let keep_refresh = if refreshed.refresh_token.is_empty() {
            tokens.refresh_token.clone()
        } else {
            refreshed.refresh_token.clone()
        };
        *tokens = StoredTokens { refresh_token: keep_refresh, ..refreshed };
    }
    Ok(tokens.access_token.clone())
}
```

- [ ] **Step 2: Add unit tests for the pure helpers:**
```rust
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
    let p = Pkce { verifier: "v".into(), challenge: "chal".into() };
    let u = authorize_url(&p, "st");
    assert!(u.contains("code_challenge=chal"));
    assert!(u.contains("code_challenge_method=S256"));
    assert!(u.contains("state=st"));
    assert!(u.contains("client_id=app_EMoamEEZ73f0CkXaXp7hrann"));
}
```

- [ ] **Step 3: Run.** `cargo test -p pas-app oauth` — Expected: PASS.

- [ ] **Step 4: Commit.** `git commit -am "feat(oauth): loopback login, token exchange and refresh"`

---

## Task 5: Responses-API request + SSE parser

**Files:** Modify `crates/pas-app/src/oauth.rs`

- [ ] **Step 1: Failing test for SSE accumulation:**
```rust
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
```

- [ ] **Step 2: Run, expect fail.** `cargo test -p pas-app sse_accumulates`

- [ ] **Step 3: Implement:**
```rust
use crate::AiMessage;

pub fn build_responses_body(model: &str, system_prompt: &str, history: &[AiMessage]) -> serde_json::Value {
    let input: Vec<_> = history
        .iter()
        .map(|m| {
            let content_type = if m.role == "assistant" { "output_text" } else { "input_text" };
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
        let Some(data) = line.strip_prefix("data:") else { continue };
        let data = data.trim();
        if data == "[DONE]" || data.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else { continue };
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

/// One non-streaming-from-the-UI call to the Codex Responses API.
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
        .send().await.map_err(|e| format!("Responses request: {e}"))?;
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Responses API {}: {}", status.as_u16(), text));
    }
    parse_responses_sse(&text)
}
```
Note: `AiMessage` is defined in `lib.rs`; the `use crate::AiMessage;` import requires it to be `pub` (it already is).

- [ ] **Step 4: Run, expect pass.** `cargo test -p pas-app sse_accumulates`

- [ ] **Step 5: Commit.** `git commit -am "feat(oauth): Responses API request builder and SSE parser"`

---

## Task 6: Tauri commands + lib.rs wiring

**Files:** Modify `crates/pas-app/src/lib.rs`

- [ ] **Step 1: Extend state and structs.** Add to `AppState`:
```rust
    chatgpt_tokens: Mutex<Option<oauth::StoredTokens>>,
```
Initialize it `Mutex::new(None)` in both `run()` and the test `AppState` (line ~1102). Add `auth_mode` to the OpenAI config/request structs:
```rust
// In AiConfigInput and AiCompletionRequest:
    #[serde(default)]
    pub auth_mode: Option<String>, // "api_key" (default) | "chatgpt"
```
Add `auth_mode` to `StoredAiConfig` (`Option<String>`), set it in `set_ai_config`. For `auth_mode == Some("chatgpt")`, **skip** the `api_key` required check.

- [ ] **Step 2: Add a token-file path helper + load/save.** Add near other helpers:
```rust
fn chatgpt_token_path(app: &AppHandle) -> Result<PathBuf, String> {
    use tauri::Manager;
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("chatgpt_tokens.enc"))
}

fn load_chatgpt_tokens(app: &AppHandle) -> Option<oauth::StoredTokens> {
    let path = chatgpt_token_path(app).ok()?;
    let blob = std::fs::read(&path).ok()?;
    let key = oauth::derive_key(path.parent()?);
    oauth::decrypt_tokens(&key, &blob).ok()
}

fn save_chatgpt_tokens(app: &AppHandle, tokens: &oauth::StoredTokens) -> Result<(), String> {
    let path = chatgpt_token_path(app)?;
    let key = oauth::derive_key(path.parent().ok_or("no parent")?);
    let blob = oauth::encrypt_tokens(&key, tokens)?;
    std::fs::write(&path, blob).map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Add commands:**
```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct OauthStatus {
    signed_in: bool,
    email: Option<String>,
}

#[tauri::command]
async fn openai_oauth_login(app: AppHandle, state: State<'_, AppState>) -> Result<OauthStatus, String> {
    let tokens = oauth::interactive_login().await?;
    save_chatgpt_tokens(&app, &tokens)?;
    let status = OauthStatus { signed_in: true, email: tokens.email.clone() };
    *state.chatgpt_tokens.lock().map_err(|_| "token lock poisoned")? = Some(tokens);
    Ok(status)
}

#[tauri::command]
fn openai_oauth_status(state: State<'_, AppState>) -> Result<OauthStatus, String> {
    let guard = state.chatgpt_tokens.lock().map_err(|_| "token lock poisoned")?;
    Ok(match guard.as_ref() {
        Some(t) => OauthStatus { signed_in: true, email: t.email.clone() },
        None => OauthStatus { signed_in: false, email: None },
    })
}

#[tauri::command]
fn openai_oauth_logout(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    *state.chatgpt_tokens.lock().map_err(|_| "token lock poisoned")? = None;
    if let Ok(path) = chatgpt_token_path(&app) {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}
```

- [ ] **Step 4: Route `ai_completion` through ChatGPT when selected.** At the top of `ai_completion`, before the existing provider logic, branch:
```rust
    let use_chatgpt = request.provider == "openai"
        && request.auth_mode.as_deref() == Some("chatgpt");
    if use_chatgpt {
        let mut tokens = state
            .chatgpt_tokens.lock().map_err(|_| "token lock poisoned")?
            .clone()
            .ok_or("Not signed in with ChatGPT")?;
        let client = reqwest::Client::new();
        let access = oauth::valid_access_token(&mut tokens, &client).await?;
        // persist any refresh
        save_chatgpt_tokens(&app, &tokens)?;
        *state.chatgpt_tokens.lock().map_err(|_| "token lock poisoned")? = Some(tokens.clone());
        let model = if request.model.trim().is_empty() { "gpt-5.5" } else { request.model.as_str() };
        let body = oauth::build_responses_body(model, &request.system_prompt, &request.messages);
        return oauth::responses_completion(&client, &access, &tokens.account_id, &body).await;
    }
```
This requires `ai_completion` to take `app: AppHandle`. Add `app: AppHandle` to its signature (Tauri injects it).

- [ ] **Step 5: Register commands + load tokens at startup.** Add the three commands to `generate_handler!`. In `.setup(|app| { ... })`, load tokens:
```rust
            if let Some(tokens) = load_chatgpt_tokens(&app.handle()) {
                if let Some(state) = app.try_state::<AppState>() {
                    if let Ok(mut g) = state.chatgpt_tokens.lock() {
                        *g = Some(tokens);
                    }
                }
            }
```
(Add `use tauri::Manager;` if not present at call site.)

- [ ] **Step 6: Build + test + lint.**
```
cargo test -p pas-app
cargo clippy -p pas-app -- -D warnings
cargo fmt --all
```
Expected: all green.

- [ ] **Step 7: Commit.** `git commit -am "feat(app): ChatGPT OAuth commands and Responses routing"`

---

## Task 7: UI — auth-mode toggle in AISettingsModal

**Files:** Modify `ui/src/AISettingsModal.tsx`

- [ ] **Step 1: Extend `AIConfig`:**
```ts
export interface AIConfig {
  provider: "openai" | "anthropic" | "gemini" | "deepseek" | "openrouter";
  apiKey: string;
  model: string;
  customUrl?: string;
  authMode?: "api_key" | "chatgpt";
}
```
Add ChatGPT models to the openai default list note (used only when authMode is chatgpt):
```ts
const CHATGPT_MODELS = ["gpt-5.5", "gpt-5.2-codex", "gpt-5.3-codex"];
```

- [ ] **Step 2: Add props for sign-in.** Extend `Props`:
```ts
  oauthStatus?: { signedIn: boolean; email?: string } | null;
  onOauthLogin?: () => Promise<void>;
  onOauthLogout?: () => Promise<void>;
```

- [ ] **Step 3: Add state + UI.** Add `const [authMode, setAuthMode] = useState<"api_key" | "chatgpt">("api_key");` initialized from `initialConfig?.authMode ?? "api_key"`. When `provider === "openai"`, render a radio/select toggle for auth mode. When `authMode === "chatgpt"`: hide the API-key input; show signed-in email + a "Sign in with ChatGPT" / "Sign out" button calling the new props; make the model dropdown use `CHATGPT_MODELS`. Pass `authMode` through in `onSave` (only meaningful for openai; send `undefined` otherwise). Relax the required `apiKey` when in chatgpt mode.

- [ ] **Step 4: Build (type-check).** `cd ui && pnpm run build` — Expected: PASS.

- [ ] **Step 5: Commit.** `git commit -am "feat(ui): ChatGPT auth-mode toggle in AI settings"`

---

## Task 8: UI — wire AIChatPanel

**Files:** Modify `ui/src/AIChatPanel.tsx`

- [ ] **Step 1: Persist/restore `authMode`.** Include `authMode` in the `pas.ai_config_public` localStorage object (it is non-secret) and restore it into `config`.

- [ ] **Step 2: Pass `authMode` in the request.** In `fetchLLMCompletion`, add `authMode: config.authMode` to the `ai_completion` request payload, and add it to the `set_ai_config` call in `saveConfig`.

- [ ] **Step 3: Add OAuth handlers + status.** Add state `oauthStatus`; on mount and after login/logout call `invoke("openai_oauth_status")`. Implement `onOauthLogin = async () => { const s = await invoke("openai_oauth_login"); setOauthStatus(s); }` and `onOauthLogout`. Pass `oauthStatus`, `onOauthLogin`, `onOauthLogout` to `AISettingsModal`. When `config.authMode === "chatgpt"`, the "Setup required" gating in `sendMessageDirectly` should treat a signed-in status as configured (don't force-open settings if signed in).

- [ ] **Step 4: Build + tests.** `cd ui && pnpm run build && pnpm test && pnpm run test:smoke` — Expected: PASS (smoke test still green: no secret in browser storage).

- [ ] **Step 5: Commit.** `git commit -am "feat(ui): wire ChatGPT OAuth login into chat panel"`

---

## Task 9: Docs + final verification

**Files:** Modify `CHANGELOG.md`, `DIVERGENCE.md`

- [ ] **Step 1: CHANGELOG.** Under `[Unreleased]` add: `- AI assistant: "Sign in with ChatGPT" (OAuth) as an alternative to the OpenAI API key, using the Codex Responses API.`

- [ ] **Step 2: DIVERGENCE.** Add a short note: PAS embeds the public Codex OAuth `client_id`; ChatGPT tokens are stored AES-GCM-encrypted with a machine-derived key (obfuscation, not strong at-rest protection).

- [ ] **Step 3: Full verification.**
```
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo test -p pas-engine
cargo test -p pas-app
cd ui && pnpm run build && pnpm test && pnpm run test:smoke
```
Expected: all green.

- [ ] **Step 4: Commit.** `git commit -am "docs: document ChatGPT OAuth feature"`

---

## Self-Review notes

- **Spec coverage:** loopback (T4), encrypted file + machine key (T3/T6), auth-mode toggle (T7/T6), Responses API + SSE collapse (T5/T6), refresh-on-expiry (T4/T6), commands status/logout (T6), tests (T1-5, T8), docs (T9). All spec sections mapped.
- **Type consistency:** `StoredTokens`, `IdClaims`, `Pkce`, `parse_responses_sse`, `build_responses_body`, `interactive_login`, `valid_access_token`, `refresh` referenced consistently. `auth_mode` string `"chatgpt"` used identically in UI and Rust.
- **Risk to confirm at runtime:** exact Codex Responses SSE event/field names (`delta`) and request body field names — verify against a live response during T6/manual test; `parse_responses_sse` is lenient (ignores unparseable lines) to reduce breakage.
