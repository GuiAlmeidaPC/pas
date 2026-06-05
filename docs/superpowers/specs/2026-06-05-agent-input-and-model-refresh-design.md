# Agent input and model refresh design

## Goal

Fix two UX problems in the PAS Agent UI:

1. The ask box must wrap long prompts, auto-grow up to eight lines, then scroll internally.
2. Agent Setup must let the user explicitly fetch live provider models before trusting the model dropdown.

## Current state

- `ui/src/AIChatPanel.tsx` renders the prompt field as a single-line `<input>`.
- `ui/src/styles.css` styles that field only for one-line input.
- `ui/src/AISettingsModal.tsx` renders model choices from curated defaults only.
- `ui/src/AIChatPanel.tsx` already has a `refreshModels()` path that fetches live models after config save/load, but that happens outside the setup modal, so the setup dropdown can present stale or misleading choices while the user is still configuring credentials.

## Decisions

### 1. Prompt input becomes a capped auto-growing textarea

Replace the single-line input with a textarea in `AIChatPanel`.

Behavior:
- Long text wraps onto new lines.
- Height grows with content until eight visible lines.
- After eight lines, the textarea keeps a fixed max height and scrolls internally.
- `Enter` submits the prompt.
- `Shift+Enter` inserts a newline.
- The existing disabled/loading behavior stays unchanged.

Why:
- This matches chat-style prompt entry.
- It avoids horizontal overflow and makes long prompts readable before submission.
- Keeping `Enter` to send preserves the current quick-send flow.

### 2. Live model fetch belongs in the setup modal

Move the model-fetch interaction into `AISettingsModal`, where provider, auth mode, API key, and model choice are edited together.

Behavior:
- For API-key-backed providers, show a `Fetch Models` button near the model selector.
- Before any successful fetch, the modal shows curated fallback models.
- After a successful fetch, the dropdown switches to the fetched models only for the current provider/auth/base-url combination.
- If fetch fails, the curated fallback remains available and the modal shows an inline error.
- ChatGPT auth mode remains curated-only and does not show the fetch button.

Why:
- The user explicitly decides when credentials are ready to validate.
- The model dropdown no longer implies that curated defaults are authoritative live results.
- Fetched results become authoritative only after a successful explicit refresh.

### 3. Conservative failure behavior

When model fetch fails:
- do not clear the curated fallback list,
- do not block saving,
- do show an inline error message explaining the failure.

Why:
- Setup must remain usable offline or when a provider model-list endpoint is unavailable.
- The user can still proceed with a curated or custom model name.

## Component changes

### `ui/src/AIChatPanel.tsx`

- Replace the input element with a textarea.
- Add a small resize helper that measures `scrollHeight` and applies a capped height.
- Handle keyboard submission (`Enter` vs `Shift+Enter`).
- Remove ownership of setup-time live-model fetching from the panel.
- Pass a modal callback that fetches models using the backend command and returns results/error to the modal.

### `ui/src/AISettingsModal.tsx`

- Accept an `onFetchModels` async prop.
- Add local state for:
  - fetch-in-progress
  - fetched models
  - fetch error
- Derive displayed dropdown options from:
  - curated ChatGPT models for ChatGPT auth,
  - fetched models after success,
  - curated fallback otherwise.
- Reset fetched-model state when provider/auth/base URL changes.

### `ui/src/styles.css`

- Update `.chat-input-form` styles to support a textarea.
- Disable manual resize to keep layout stable.
- Set line height and max-height consistent with the eight-line cap.

## Testing strategy

### `ui/src/__tests__/AIChatPanel.test.tsx`

Add tests for:
- `Enter` submits the prompt.
- `Shift+Enter` inserts a newline instead of submitting.
- the prompt control is a textarea with multiline value behavior.

In jsdom, layout metrics are limited, so the auto-grow test should assert behavior through textarea semantics and keyboard handling rather than pixel-perfect visual height.

### `ui/src/__tests__/AISettingsModal.test.tsx`

Add modal-focused tests for:
- curated fallback shown before fetch,
- successful `Fetch Models` replaces dropdown options with fetched models,
- failed `Fetch Models` leaves fallback options available and renders inline error,
- ChatGPT auth hides fetch button.

## Non-goals

- No backend API changes unless the existing `list_ai_models` contract proves insufficient.
- No persistence of fetched model lists beyond the current setup interaction unless existing panel logic still needs cache for later quick switching.
- No redesign of the rest of the Agent setup modal.
