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
  onSave: (config: AIConfig) => void;
  initialConfig?: AIConfig | null;
}

const DEFAULT_MODELS: Record<AIConfig["provider"], string[]> = {
  openai: ["gpt-5.5-instant", "gpt-5.5-pro", "gpt-5.4-mini"],
  anthropic: ["claude-sonnet-4.6", "claude-opus-4.7", "claude-haiku-4.5"],
  gemini: ["gemini-3.5-flash", "gemini-3.1-pro", "gemini-3.1-flash-lite"],
  deepseek: ["deepseek-v4-flash", "deepseek-v4-pro"],
  openrouter: ["google/gemini-2.5-pro", "meta-llama/llama-3.3-70b-instruct"],
};

export function AISettingsModal({ isOpen, onClose, onSave, initialConfig }: Props) {
  const [provider, setProvider] = useState<AIConfig["provider"]>("openai");
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState("");
  const [customUrl, setCustomUrl] = useState("");
  const [customModel, setCustomModel] = useState("");
  const [useCustomModel, setUseCustomModel] = useState(false);

  useEffect(() => {
    if (initialConfig) {
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
      setProvider("openai");
      setApiKey("");
      setCustomUrl("");
      setModel("gpt-5.5-instant");
      setCustomModel("");
      setUseCustomModel(false);
    }
  }, [initialConfig, isOpen]);

  // Adjust model select when provider changes
  const handleProviderChange = (p: AIConfig["provider"]) => {
    setProvider(p);
    const defaults = DEFAULT_MODELS[p] || [];
    setModel(defaults[0] || "");
    setCustomUrl("");
  };

  if (!isOpen) return null;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onSave({
      provider,
      apiKey: apiKey.trim(),
      model: useCustomModel ? customModel.trim() : model,
      customUrl: customUrl.trim() || undefined,
    });
    onClose();
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
                  disabled={useCustomModel}
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
                  onChange={(e) => setUseCustomModel(e.target.checked)}
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
