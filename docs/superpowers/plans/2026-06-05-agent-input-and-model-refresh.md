# Agent Input And Model Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Agent prompt field wrap and auto-grow up to eight lines, and make Agent Setup fetch live provider models explicitly before trusting the dropdown.

**Architecture:** Keep the prompt UX in `AIChatPanel`, but move setup-time model refresh ownership into `AISettingsModal` so model selection and model fetching live together. Reuse the existing Tauri `list_ai_models` command through a modal callback instead of introducing a new backend surface.

**Tech Stack:** React 18, TypeScript, Vitest, Testing Library, Tauri invoke.

---

## File map

- Modify: `ui/src/AIChatPanel.tsx` — textarea prompt, capped auto-grow, keyboard handling, modal fetch callback.
- Modify: `ui/src/AISettingsModal.tsx` — explicit fetch button, fetched-model state, fallback/error handling.
- Modify: `ui/src/styles.css` — textarea styling and setup modal controls.
- Modify: `ui/src/__tests__/AIChatPanel.test.tsx` — prompt behavior tests.
- Create: `ui/src/__tests__/AISettingsModal.test.tsx` — setup modal fetch flow tests.

### Task 1: Cover the settings modal model-fetch flow with tests

**Files:**
- Create: `ui/src/__tests__/AISettingsModal.test.tsx`
- Modify: `ui/src/AISettingsModal.tsx`

- [ ] **Step 1: Write the failing test for successful model refresh**

```tsx
it("replaces curated model options after a successful fetch", async () => {
  const user = userEvent.setup();
  const onFetchModels = vi.fn().mockResolvedValue(["gpt-4.1", "gpt-4.1-mini"]);

  render(
    <AISettingsModal
      isOpen
      onClose={vi.fn()}
      onSave={vi.fn()}
      onFetchModels={onFetchModels}
      initialConfig={{ provider: "openai", apiKey: "sk-test", model: "gpt-4o", authMode: "api_key" }}
    />,
  );

  expect(screen.getByRole("option", { name: "gpt-4o" })).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: /fetch models/i }));

  await waitFor(() => expect(screen.getByRole("option", { name: "gpt-4.1" })).toBeInTheDocument());
  expect(screen.queryByRole("option", { name: "gpt-4o" })).not.toBeInTheDocument();
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `pnpm test -- AISettingsModal.test.tsx`
Expected: FAIL because `AISettingsModal` has no `onFetchModels` prop or fetch button yet.

- [ ] **Step 3: Write the failing test for fetch failure fallback**

```tsx
it("keeps curated fallback models and shows an inline error when fetch fails", async () => {
  const user = userEvent.setup();
  const onFetchModels = vi.fn().mockRejectedValue(new Error("401 Unauthorized"));

  render(
    <AISettingsModal
      isOpen
      onClose={vi.fn()}
      onSave={vi.fn()}
      onFetchModels={onFetchModels}
      initialConfig={{ provider: "openai", apiKey: "sk-test", model: "gpt-4o", authMode: "api_key" }}
    />,
  );

  await user.click(screen.getByRole("button", { name: /fetch models/i }));

  await waitFor(() => expect(screen.getByText(/401 unauthorized/i)).toBeInTheDocument());
  expect(screen.getByRole("option", { name: "gpt-4o" })).toBeInTheDocument();
});
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `pnpm test -- AISettingsModal.test.tsx`
Expected: FAIL for the same missing modal behavior.

- [ ] **Step 5: Write the failing test for ChatGPT auth hiding refresh**

```tsx
it("hides model refresh for chatgpt auth mode", () => {
  render(
    <AISettingsModal
      isOpen
      onClose={vi.fn()}
      onSave={vi.fn()}
      onFetchModels={vi.fn()}
      initialConfig={{ provider: "openai", apiKey: "", model: "gpt-5.5", authMode: "chatgpt" }}
    />,
  );

  expect(screen.queryByRole("button", { name: /fetch models/i })).not.toBeInTheDocument();
});
```

- [ ] **Step 6: Run the test to verify it fails**

Run: `pnpm test -- AISettingsModal.test.tsx`
Expected: FAIL until the modal branches on auth mode.

- [ ] **Step 7: Implement the modal fetch flow minimally**

```tsx
interface Props {
  // ...existing props...
  onFetchModels?: (config: AIConfig) => Promise<string[]>;
}

const [fetchedModels, setFetchedModels] = useState<string[] | null>(null);
const [fetchingModels, setFetchingModels] = useState(false);
const [fetchError, setFetchError] = useState<string | null>(null);

const visibleModels = isChatgpt
  ? CHATGPT_MODELS
  : fetchedModels && fetchedModels.length > 0
    ? fetchedModels
    : DEFAULT_MODELS[provider] || [];
```

- [ ] **Step 8: Add the fetch button and error rendering**

```tsx
{!isChatgpt && (
  <div className="model-actions">
    <button
      type="button"
      className="btn-secondary btn-sm"
      disabled={fetchingModels || !apiKey.trim()}
      onClick={async () => {
        if (!onFetchModels) return;
        setFetchingModels(true);
        setFetchError(null);
        try {
          const models = await onFetchModels({
            provider,
            apiKey: apiKey.trim(),
            model,
            customUrl: customUrl.trim() || undefined,
            authMode,
          });
          setFetchedModels(models);
          if (models[0]) setModel(models[0]);
        } catch (e) {
          setFetchError(String(e));
        } finally {
          setFetchingModels(false);
        }
      }}
    >
      {fetchingModels ? "Fetching…" : "Fetch Models"}
    </button>
    {fetchError && <span className="field-hint field-error">{fetchError}</span>}
  </div>
)}
```

- [ ] **Step 9: Re-run the modal tests**

Run: `pnpm test -- AISettingsModal.test.tsx`
Expected: PASS.

### Task 2: Cover the multiline prompt behavior with tests

**Files:**
- Modify: `ui/src/__tests__/AIChatPanel.test.tsx`
- Modify: `ui/src/AIChatPanel.tsx`

- [ ] **Step 1: Write the failing test for Shift+Enter newline**

```tsx
it("keeps the prompt open on Shift+Enter and inserts a newline", async () => {
  const user = userEvent.setup();
  renderPanel();

  const prompt = screen.getByPlaceholderText(/ask openai agent/i);
  await user.type(prompt, "first line");
  await user.keyboard("{Shift>}{Enter}{/Shift}second line");

  expect(prompt).toHaveValue("first line\nsecond line");
  expect(tauriCore.invoke).not.toHaveBeenCalledWith("ai_completion_stream", expect.anything());
});
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `pnpm test -- AIChatPanel.test.tsx`
Expected: FAIL because the control is still a single-line input.

- [ ] **Step 3: Write the failing test for Enter submit**

```tsx
it("submits the prompt on Enter", async () => {
  const user = userEvent.setup();
  const completion = deferred<string>();
  (tauriCore.invoke as ReturnType<typeof vi.fn>).mockImplementation((cmd: string) => {
    if (cmd === "openai_oauth_status") return Promise.resolve({ signedIn: false });
    if (cmd === "ai_completion_stream") return completion.promise;
    return Promise.resolve();
  });

  renderPanel();
  const prompt = screen.getByPlaceholderText(/ask openai agent/i);
  await user.type(prompt, "Create a report{Enter}");

  await waitFor(() => expect(tauriCore.invoke).toHaveBeenCalledWith("ai_completion_stream", expect.anything()));
});
```

- [ ] **Step 4: Run the test to verify it fails for the right reason**

Run: `pnpm test -- AIChatPanel.test.tsx`
Expected: FAIL because Enter handling is still tied to form submit on a one-line input, not the textarea contract.

- [ ] **Step 5: Implement the textarea and keyboard handling minimally**

```tsx
const inputRef = useRef<HTMLTextAreaElement>(null);

<textarea
  ref={inputRef}
  rows={1}
  value={input}
  onChange={(e) => setInput(e.target.value)}
  onKeyDown={(e) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  }}
/>
```

- [ ] **Step 6: Add capped auto-grow behavior**

```tsx
useLayoutEffect(() => {
  const el = inputRef.current;
  if (!el) return;
  el.style.height = "0px";
  const next = Math.min(el.scrollHeight, INPUT_MAX_HEIGHT);
  el.style.height = `${next}px`;
  el.style.overflowY = el.scrollHeight > INPUT_MAX_HEIGHT ? "auto" : "hidden";
}, [input]);
```

- [ ] **Step 7: Re-run the panel tests**

Run: `pnpm test -- AIChatPanel.test.tsx`
Expected: PASS.

### Task 3: Wire modal fetch to the backend and finish styles

**Files:**
- Modify: `ui/src/AIChatPanel.tsx`
- Modify: `ui/src/AISettingsModal.tsx`
- Modify: `ui/src/styles.css`

- [ ] **Step 1: Add the modal fetch callback in the panel**

```tsx
const fetchModelsForSetup = async (cfg: AIConfig) => {
  await invoke("set_ai_config", {
    config: {
      provider: cfg.provider,
      apiKey: cfg.apiKey,
      model: cfg.model || "placeholder",
      customUrl: cfg.customUrl,
      authMode: cfg.authMode,
    },
  });
  return invoke<string[]>("list_ai_models");
};
```

- [ ] **Step 2: Pass the callback into the modal**

```tsx
<AISettingsModal
  // ...existing props...
  onFetchModels={fetchModelsForSetup}
/>
```

- [ ] **Step 3: Update styles for the textarea and fetch controls**

```css
.chat-input-form textarea {
  flex: 1;
  min-height: 36px;
  max-height: calc(1.4em * 8 + 16px);
  resize: none;
  overflow-y: auto;
  white-space: pre-wrap;
}

.ai-settings-form .model-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.ai-settings-form .field-error {
  color: var(--danger);
}
```

- [ ] **Step 4: Run the focused UI test files**

Run: `pnpm test -- AIChatPanel.test.tsx AISettingsModal.test.tsx`
Expected: PASS.

### Task 4: Final verification

**Files:**
- Modify: `ui/src/AIChatPanel.tsx`
- Modify: `ui/src/AISettingsModal.tsx`
- Modify: `ui/src/styles.css`
- Modify: `ui/src/__tests__/AIChatPanel.test.tsx`
- Create: `ui/src/__tests__/AISettingsModal.test.tsx`

- [ ] **Step 1: Run the full frontend test suite**

Run: `pnpm test`
Expected: PASS.

- [ ] **Step 2: Run the frontend build**

Run: `pnpm run build`
Expected: PASS.

- [ ] **Step 3: Commit the change**

```bash
git add ui/src/AIChatPanel.tsx ui/src/AISettingsModal.tsx ui/src/styles.css ui/src/__tests__/AIChatPanel.test.tsx ui/src/__tests__/AISettingsModal.test.tsx docs/superpowers/specs/2026-06-05-agent-input-and-model-refresh-design.md docs/superpowers/plans/2026-06-05-agent-input-and-model-refresh.md
git commit -m "feat(ui): improve agent input and model refresh"
```
