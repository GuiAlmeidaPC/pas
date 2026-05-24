import { useState, useEffect } from "react";

export interface AIConfig {
  provider: "openai" | "anthropic" | "gemini" | "deepseek" | "openrouter";
  apiKey: string;
  model: string;
  customUrl?: string;
}

interface Props {
  isOpen: boolean;
  onClose: () => void;
  onSave: (config: AIConfig) => void | Promise<void>;
  initialConfig?: AIConfig | null;
}

const DEFAULT_MODELS: Record<AIConfig["provider"], string[]> = {
  openai: ["gpt-4o", "gpt-4o-mini", "gpt-4-turbo"],
  anthropic: ["claude-3-5-sonnet-latest", "claude-3-5-haiku-latest", "claude-3-opus-latest"],
  gemini: ["gemini-1.5-pro", "gemini-1.5-flash"],
  deepseek: ["deepseek-chat", "deepseek-coder"],
  openrouter: ["meta-llama/llama-3.3-70b-instruct", "google/gemini-2.5-pro"],
};

export function AISettingsModal({ isOpen, onClose, onSave, initialConfig }: Props) {
  const [provider, setProvider] = useState<AIConfig["provider"]>("openai");
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState("");
  const [customUrl, setCustomUrl] = useState("");
  const [customModel, setCustomModel] = useState("");
  const [useCustomModel, setUseCustomModel] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    if (initialConfig) {
      setFormError(null);
      setProvider(initialConfig.provider);
      setApiKey(initialConfig.apiKey || "");
      setCustomUrl(initialConfig.customUrl || "");
      
      const defaults = DEFAULT_MODELS[initialConfig.provider] || [];
      if (defaults.includes(initialConfig.model)) {
        setModel(initialConfig.model);
        setUseCustomModel(false);
      } else {
        setCustomModel(initialConfig.model);
        setUseCustomModel(true);
      }
    } else {
      // Set defaults
      setFormError(null);
      setProvider("openai");
      setApiKey("");
      setCustomUrl("");
      setModel("");
      setCustomModel("");
      setUseCustomModel(true);
    }
  }, [initialConfig, isOpen]);

  // Adjust model select when provider changes
  const handleProviderChange = (p: AIConfig["provider"]) => {
    setProvider(p);
    const defaults = DEFAULT_MODELS[p] || [];
    setModel(defaults[0] || "");
    setUseCustomModel(defaults.length === 0);
    setCustomUrl("");
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
      });
      onClose();
    } catch (e) {
      setFormError(String(e));
    }
  };

  const defaultModels = DEFAULT_MODELS[provider] || [];

  return (
    <div className="modal-overlay">
      <div className="modal-content ai-settings-modal">
        <div className="modal-header">
          <h3>AI Assistant Setup</h3>
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

          <div className="form-group">
            <label htmlFor="ai-apikey">API Key</label>
            <input
              id="ai-apikey"
              type="password"
              placeholder={`Enter your ${provider.toUpperCase()} API key`}
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              required
            />
          </div>

          <div className="form-group">
            <label>Model Selection</label>
            <div className="model-selector-wrapper">
              {!useCustomModel ? (
                <select
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  disabled={useCustomModel || defaultModels.length === 0}
                >
                  {defaultModels.map((m) => (
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
                  onChange={(e) => setUseCustomModel(defaultModels.length === 0 ? true : e.target.checked)}
                  disabled={defaultModels.length === 0}
                />
                Use custom model name
              </label>
            </div>
          </div>

          {(provider === "openrouter" || provider === "deepseek" || provider === "openai") && (
            <div className="form-group">
              <label htmlFor="ai-customurl">Custom Base URL (Optional)</label>
              <input
                id="ai-customurl"
                type="text"
                placeholder="https://..."
                value={customUrl}
                onChange={(e) => setCustomUrl(e.target.value)}
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
