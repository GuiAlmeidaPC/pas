import { useState, useEffect } from "react";

export interface AIConfig {
  provider: "openai" | "anthropic" | "gemini" | "deepseek" | "openrouter";
  apiKey: string;
  model: string;
  customUrl?: string;
  /** "api_key" (default) | "chatgpt". Only meaningful for the openai provider. */
  authMode?: "api_key" | "chatgpt";
}

export interface OAuthStatus {
  signedIn: boolean;
  email?: string;
}

interface Props {
  isOpen: boolean;
  onClose: () => void;
  onSave: (config: AIConfig) => void | Promise<void>;
  onFetchModels?: (config: AIConfig) => Promise<string[]>;
  initialConfig?: AIConfig | null;
  oauthStatus?: OAuthStatus | null;
  onOauthLogin?: () => Promise<void>;
  onOauthLogout?: () => Promise<void>;
}

const DEFAULT_MODELS: Record<AIConfig["provider"], string[]> = {
  openai: ["gpt-4o", "gpt-4o-mini", "gpt-4-turbo"],
  anthropic: ["claude-3-5-sonnet-latest", "claude-3-5-haiku-latest", "claude-3-opus-latest"],
  gemini: ["gemini-1.5-pro", "gemini-1.5-flash"],
  deepseek: ["deepseek-chat", "deepseek-coder"],
  openrouter: ["meta-llama/llama-3.3-70b-instruct", "google/gemini-2.5-pro"],
};

// Models reachable through the ChatGPT (Codex Responses) backend.
const CHATGPT_MODELS = ["gpt-5.5", "gpt-5.4", "gpt-5.4-mini", "gpt-5.2-codex", "gpt-5.3-codex"];

/** The model choices to offer for a given provider and auth mode. */
export function availableModels(
  provider: AIConfig["provider"],
  authMode?: "api_key" | "chatgpt",
): string[] {
  if (provider === "openai" && authMode === "chatgpt") return CHATGPT_MODELS;
  return DEFAULT_MODELS[provider] || [];
}

export function AISettingsModal({
  isOpen,
  onClose,
  onSave,
  onFetchModels,
  initialConfig,
  oauthStatus,
  onOauthLogin,
  onOauthLogout,
}: Props) {
  const [provider, setProvider] = useState<AIConfig["provider"]>("openai");
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState("");
  const [customUrl, setCustomUrl] = useState("");
  const [customModel, setCustomModel] = useState("");
  const [useCustomModel, setUseCustomModel] = useState(false);
  const [authMode, setAuthMode] = useState<"api_key" | "chatgpt">("api_key");
  const [oauthBusy, setOauthBusy] = useState(false);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [fetchedModels, setFetchedModels] = useState<string[] | null>(null);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [formError, setFormError] = useState<string | null>(null);
  const resetFetchedModels = () => {
    setFetchedModels(null);
    setFetchError(null);
  };

  useEffect(() => {
    if (initialConfig) {
      setFormError(null);
      setProvider(initialConfig.provider);
      setApiKey(initialConfig.apiKey || "");
      setCustomUrl(initialConfig.customUrl || "");
      setAuthMode(initialConfig.authMode === "chatgpt" ? "chatgpt" : "api_key");

      const defaults = availableModels(initialConfig.provider, initialConfig.authMode);
      resetFetchedModels();
      if (defaults.includes(initialConfig.model)) {
        setModel(initialConfig.model);
        setCustomModel("");
        setUseCustomModel(false);
      } else {
        setModel(defaults[0] || "");
        setCustomModel(initialConfig.model);
        setUseCustomModel(true);
      }
    } else {
      // Set defaults
      const defaults = availableModels("openai", "api_key");
      setFormError(null);
      resetFetchedModels();
      setProvider("openai");
      setApiKey("");
      setCustomUrl("");
      setModel(defaults[0] || "");
      setCustomModel("");
      setUseCustomModel(false);
      setAuthMode("api_key");
    }
  }, [initialConfig, isOpen]);

  // Adjust model select when provider changes
  const handleProviderChange = (p: AIConfig["provider"]) => {
    setProvider(p);
    const defaults = availableModels(p, p === "openai" ? authMode : undefined);
    resetFetchedModels();
    setModel(defaults[0] || "");
    setCustomModel("");
    setUseCustomModel(defaults.length === 0);
    setCustomUrl("");
    if (p !== "openai") setAuthMode("api_key");
  };

  // Switch between API-key and ChatGPT-login auth (openai only).
  const handleAuthModeChange = (mode: "api_key" | "chatgpt") => {
    setAuthMode(mode);
    const defaults = availableModels("openai", mode);
    resetFetchedModels();
    setModel(defaults[0] || "");
    setCustomModel("");
    setUseCustomModel(false);
  };

  const runOauth = async (fn?: () => Promise<void>) => {
    if (!fn) return;
    setOauthBusy(true);
    setFormError(null);
    try {
      await fn();
    } catch (e) {
      setFormError(String(e));
    } finally {
      setOauthBusy(false);
    }
  };
  const handleFetchModels = async () => {
    if (!onFetchModels) return;
    setFetchingModels(true);
    setFetchError(null);
    try {
      const models = await onFetchModels({
        provider,
        apiKey: apiKey.trim(),
        model: useCustomModel ? customModel.trim() : model,
        customUrl: customUrl.trim() || undefined,
        authMode: provider === "openai" ? authMode : undefined,
      });
      const nextModels = Array.from(new Set(models.filter(Boolean)));
      setFetchedModels(nextModels);
      if (nextModels.length > 0) {
        setModel((current) => (nextModels.includes(current) ? current : nextModels[0]));
        setUseCustomModel(false);
      }
    } catch (e) {
      setFetchedModels(null);
      setFetchError(e instanceof Error ? e.message : String(e));
    } finally {
      setFetchingModels(false);
    }
  };

  if (!isOpen) return null;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setFormError(null);
    try {
      await onSave({
        provider,
        apiKey: apiKey.trim(),
        model: useCustomModel ? customModel.trim() : model,
        customUrl: customUrl.trim() || undefined,
        authMode: provider === "openai" ? authMode : undefined,
      });
      onClose();
    } catch (e) {
      setFormError(String(e));
    }
  };

  const isChatgpt = provider === "openai" && authMode === "chatgpt";
  const defaultModels = availableModels(provider, authMode);
  const visibleModels = isChatgpt
    ? defaultModels
    : fetchedModels && fetchedModels.length > 0
      ? fetchedModels
      : defaultModels;

  return (
    <div className="modal-overlay">
      <div className="modal-content ai-settings-modal">
        <div className="modal-header">
          <h3>Agent Setup</h3>
          <button className="close-btn" onClick={onClose}>&times;</button>
        </div>
        <form onSubmit={handleSubmit} className="ai-settings-form">
          <div className="form-group">
            <label htmlFor="ai-provider">AI Provider</label>
            <select
              id="ai-provider"
              value={provider}
              onChange={(e) => handleProviderChange(e.target.value as AIConfig["provider"])}
            >
              <option value="openai">OpenAI</option>
              <option value="anthropic">Anthropic (Claude)</option>
              <option value="gemini">Google Gemini</option>
              <option value="deepseek">DeepSeek</option>
              <option value="openrouter">OpenRouter</option>
            </select>
          </div>

          {provider === "openai" && (
            <div className="form-group">
              <label htmlFor="ai-authmode">Authentication</label>
              <select
                id="ai-authmode"
                value={authMode}
                onChange={(e) => handleAuthModeChange(e.target.value as "api_key" | "chatgpt")}
              >
                <option value="api_key">API Key</option>
                <option value="chatgpt">Sign in with ChatGPT</option>
              </select>
              <span className="field-hint">
                ChatGPT login uses your ChatGPT subscription via the Codex backend instead of an API key.
              </span>
            </div>
          )}

          {isChatgpt ? (
            <div className="form-group">
              <label>ChatGPT Account</label>
              {oauthStatus?.signedIn ? (
                <div className="oauth-status">
                  <span className="oauth-signed-in">
                    ✅ Signed in{oauthStatus.email ? ` as ${oauthStatus.email}` : ""}
                  </span>
                  <button
                    type="button"
                    className="btn-secondary btn-sm"
                    disabled={oauthBusy}
                    onClick={() => runOauth(onOauthLogout)}
                  >
                    Sign out
                  </button>
                </div>
              ) : (
                <button
                  type="button"
                  className="btn-primary"
                  disabled={oauthBusy}
                  onClick={() => runOauth(onOauthLogin)}
                >
                  {oauthBusy ? "Waiting for browser…" : "Sign in with ChatGPT"}
                </button>
              )}
            </div>
          ) : (
            <div className="form-group">
              <label htmlFor="ai-apikey">API Key</label>
              <input
                id="ai-apikey"
                type="password"
                placeholder={`Enter your ${provider.toUpperCase()} API key`}
                value={apiKey}
                onChange={(e) => {
                  resetFetchedModels();
                  setApiKey(e.target.value);
                }}
                required
              />
            </div>
          )}

          <div className="form-group">
            <label>Model Selection</label>
            <div className="model-selector-wrapper">
              {!useCustomModel ? (
                <select
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  disabled={useCustomModel || visibleModels.length === 0 || fetchingModels}
                >
                  {visibleModels.map((m) => (
                    <option key={m} value={m}>{m}</option>
                  ))}
                </select>
              ) : (
                <input
                  type="text"
                  placeholder="Enter custom model identifier"
                  value={customModel}
                  onChange={(e) => setCustomModel(e.target.value)}
                  required
                />
              )}
            </div>
            <div className="checkbox-option">
              <label>
                <input
                  type="checkbox"
                  checked={useCustomModel}
                  onChange={(e) => setUseCustomModel(visibleModels.length === 0 ? true : e.target.checked)}
                  disabled={visibleModels.length === 0}
                />
                Use custom model name
              </label>
            </div>
            {!isChatgpt && (
              <div className="model-actions">
                <button
                  type="button"
                  className="btn-secondary btn-sm"
                  disabled={fetchingModels || !apiKey.trim() || !onFetchModels}
                  onClick={handleFetchModels}
                >
                  {fetchingModels ? "Fetching…" : "Fetch Models"}
                </button>
                {fetchError && <span className="field-hint field-error">{fetchError}</span>}
              </div>
            )}
          </div>

          {!isChatgpt && (provider === "openrouter" || provider === "deepseek" || provider === "openai") && (
            <div className="form-group">
              <label htmlFor="ai-customurl">Custom Base URL (Optional)</label>
              <input
                id="ai-customurl"
                type="text"
                placeholder="https://..."
                value={customUrl}
                onChange={(e) => {
                  resetFetchedModels();
                  setCustomUrl(e.target.value);
                }}
              />
              <span className="field-hint">Leave blank to use default API endpoints.</span>
            </div>
          )}

          {formError && (
            <div className="chat-error-card">
              {formError}
            </div>
          )}

          <div className="form-actions">
            <button type="button" className="btn-secondary" onClick={onClose}>
              Cancel
            </button>
            <button type="submit" className="btn-primary">
              Save Configuration
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
