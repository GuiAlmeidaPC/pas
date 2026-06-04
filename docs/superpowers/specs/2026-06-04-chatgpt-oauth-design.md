# Design: "Sign in with ChatGPT" (OAuth) for the PAS AI Assistant

**Date:** 2026-06-04
**Status:** Approved design — pending implementation plan

## 1. Goal

Let users authenticate the PAS AI Assistant with their **ChatGPT subscription**
(Plus/Pro/Team) via OAuth — the same "Sign in with ChatGPT" flow used by the
OpenAI Codex CLI and the opencode plugins — as an alternative to entering a
per-token OpenAI API key. Usage then bills against the ChatGPT subscription
instead of API credits.

## 2. Background / current state

The assistant today (`ui/src/AIChatPanel.tsx` + `ui/src/AISettingsModal.tsx`)
supports five **API-key** providers (`openai`, `anthropic`, `gemini`,
`deepseek`, `openrouter`). The key is sent to a Rust proxy command
`set_ai_config` and used by `ai_completion` in `crates/pas-app/src/lib.rs`. Key
facts that constrain this design:

- **No secret ever reaches browser storage.** Only non-secret config
  (provider/model/customUrl) is written to `localStorage`; a smoke test
  (`pnpm run test:smoke`) enforces this. The API key lives only in
  `AppState.ai_config: Mutex<Option<StoredAiConfig>>` (Rust memory) and is **not
  persisted** — re-entered each launch.
- All AI HTTP calls are non-streaming: `ai_completion` does one `POST` and
  returns a single `String` that the UI `await`s.
- Endpoints are forced to `https://`.

## 3. Reference OAuth parameters (grounded)

From the Codex CLI / opencode community implementations:

| Item | Value |
|------|-------|
| `client_id` | `app_EMoamEEZ73f0CkXaXp7hrann` |
| Authorize URL | `https://auth.openai.com/oauth/authorize` |
| Token URL | `https://auth.openai.com/oauth/token` |
| Redirect URI | `http://localhost:1455/auth/callback` |
| PKCE | S256 (code_verifier / code_challenge) |
| Scopes | `openid profile email offline_access` |
| Account id claim | `id_token` JWT → `https://api.openai.com/auth` → `chatgpt_account_id` |
| Model endpoint | `https://chatgpt.com/backend-api/codex/responses` (Responses API, **SSE streamed**) |
| Call headers | `Authorization: Bearer <access_token>`, `chatgpt-account-id: <id>`, `Content-Type: application/json` |

**Critical constraint:** the OAuth access token is **rejected by the standard
`/v1/chat/completions` API** ("Missing scopes"). It only works against the Codex
backend **Responses API**, which has a different request/response shape and
**streams Server-Sent Events**. This is why the ChatGPT path needs its own
request builder and response parser.

Sources: Codex auth knowledge base, `numman-ali/opencode-openai-codex-auth`,
`openai/codex#8112`.

## 4. Architecture

All OAuth and token handling lives in **Rust** (`crates/pas-app`), keeping
secrets out of the webview — consistent with the existing rule. New module
`crates/pas-app/src/oauth.rs` owns: PKCE generation, the loopback callback
server, the token exchange/refresh, `id_token` claim extraction, and the
encrypted token file. The UI only triggers the flow and reads back non-secret
status (signed in + account email).

### 4.1 Auth flow (loopback server)

1. UI calls new command `openai_oauth_login`.
2. Rust generates a PKCE verifier/challenge and a random `state`, starts a
   one-shot HTTP server on `127.0.0.1:1455`, and opens the system browser to the
   authorize URL (`tauri-plugin-opener` / equivalent).
3. The browser redirects to `/auth/callback?code=…&state=…`. The handler
   validates `state`, returns a minimal "you may close this tab" HTML page, and
   shuts the server down.
4. Rust exchanges the code at the token endpoint (PKCE `code_verifier`,
   `grant_type=authorization_code`, the redirect URI, the client id) →
   `{access_token, refresh_token, id_token, expires_in}`.
5. Rust decodes the `id_token` payload (base64url JSON — **no signature
   verification needed**, we only read claims) to obtain `chatgpt_account_id`
   and the account email.
6. Tokens are persisted (§4.3); the command returns `{email}` to the UI.

### 4.2 Provider integration (auth-mode toggle)

- `AISettingsModal` gains an **auth mode** control shown only for the OpenAI
  provider: *API key* (today) vs *ChatGPT login*. In ChatGPT mode the API-key
  field is replaced by a **Sign in with ChatGPT** button plus signed-in status
  (email + Sign out). The model dropdown defaults to **`gpt-5.5`** and also
  lists the coding variants `gpt-5.2-codex` / `gpt-5.3-codex`; the existing
  custom-model field still overrides.
- The auth mode (a non-secret string) is persisted to `localStorage` alongside
  provider/model, so the UI restores the right view on launch.
- Rust side: `StoredAiConfig` / `AiCompletionRequest` carry an `auth_mode`
  (`"api_key"` | `"chatgpt"`, default `"api_key"`). When `auth_mode = "chatgpt"`
  and provider = `openai`, `build_ai_request` targets the **Responses API**:
  - Body: `{ "model": <model>, "instructions": <system_prompt>,
    "input": [ {role, content:[{type:"input_text"|"output_text", text}]} … ],
    "stream": true }` (exact field names confirmed against the codex backend
    during implementation).
  - Headers: the OAuth bearer + `chatgpt-account-id`.
  - A new `parse_responses_sse` accumulates `response.output_text.delta`
    events server-side and returns the **full concatenated string**, preserving
    the existing non-streaming `invoke<string>("ai_completion")` contract — the
    UI is unchanged.
- Before each call, if the access token expires within 5 minutes, Rust refreshes
  it via `grant_type=refresh_token` and re-persists.

### 4.3 Token storage (encrypted file, machine-derived key)

- File in Tauri's app-data dir: `chatgpt_tokens.enc`. Plaintext contents
  (serialized JSON): access token, refresh token, expiry timestamp,
  `chatgpt_account_id`, email.
- Encrypted with an AEAD cipher (AES-256-GCM or ChaCha20-Poly1305, with a random
  nonce stored alongside the ciphertext) using a key **derived from stable
  machine/app identifiers**. This is obfuscation, not strong protection —
  explicitly chosen for zero extra dependencies and full offline operation;
  documented as such.
- The "no secrets in **browser** storage" rule is preserved — nothing secret
  reaches the webview.
- New commands:
  - `openai_oauth_status` → `{ signedIn: bool, email?: string }`
  - `openai_oauth_logout` → deletes the file and clears in-memory state.
  - On startup, the file (if present) is loaded into `AppState`.

## 5. Error handling

Surface clear errors for: browser-open failure; port `1455` already in use;
`state` mismatch; token-exchange failure; refresh failure (→ prompt re-login,
clear stored tokens); and Responses API non-200 (reuse `parse_ai_error`).

## 6. Testing

Rust unit tests:
- PKCE verifier/challenge generation (challenge = base64url(SHA256(verifier))).
- `id_token` claim extraction (account id + email from a sample payload).
- `parse_responses_sse` accumulation from a captured SSE fixture.
- Encrypt → decrypt round-trip of the token blob.

Frontend:
- A UI test that the auth-mode toggle renders the Sign-in button vs the API-key
  field appropriately.

Docs: update `CHANGELOG.md` under `[Unreleased]`; note the new dependency
audit surface if any crate is added (`cargo audit`).

## 7. Out of scope (YAGNI)

- Device-code flow for headless/SSH environments.
- Reusing tokens from a system Codex install.
- Streaming the response into the UI token-by-token (the backend streams, but we
  collapse it to a single string server-side to keep the UI contract).
- OAuth for any provider other than OpenAI.

## 8. New/changed surface (summary)

| Area | Change |
|------|--------|
| `crates/pas-app/src/oauth.rs` | New: PKCE, loopback server, token exchange/refresh, id_token claims, encrypted file. |
| `crates/pas-app/src/lib.rs` | New commands `openai_oauth_login/status/logout`; `auth_mode` on config/request structs; Responses-API branch in `build_ai_request` + `parse_responses_sse`; load tokens on startup. |
| `ui/src/AISettingsModal.tsx` | Auth-mode toggle, Sign-in button, signed-in status, ChatGPT model defaults. |
| `ui/src/AIChatPanel.tsx` | Persist/restore `auth_mode`; pass it in the completion request; status display. |
| `Cargo.toml` | Possibly add an AEAD crate + base64/sha2 (verify against existing deps first). |
| `CHANGELOG.md` | `[Unreleased]` entry. |
