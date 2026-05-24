import { useState, useEffect, useRef } from "react";
import { AISettingsModal, type AIConfig } from "./AISettingsModal";

interface Message {
  role: "user" | "assistant" | "system";
  content: string;
}

interface Props {
  activeContent: string;
  activeSelection: string;
  onInsertCode: (code: string) => void;
  onReplaceCode: (code: string) => void;
  onNewTab: (code: string) => void;
}

export function AIChatPanel({
  activeContent,
  activeSelection,
  onInsertCode,
  onReplaceCode,
  onNewTab,
}: Props) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [config, setConfig] = useState<AIConfig | null>(null);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Load configuration from localStorage on mount
  useEffect(() => {
    const saved = localStorage.getItem("pas.ai_config");
    if (saved) {
      try {
        setConfig(JSON.parse(saved));
      } catch (e) {
        console.error("Failed to parse saved AI config", e);
      }
    }
  }, []);

  // Scroll to bottom on new messages
  useEffect(() => {
    if (listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [messages, loading]);

  const saveConfig = (newConfig: AIConfig) => {
    setConfig(newConfig);
    localStorage.setItem("pas.ai_config", JSON.stringify(newConfig));
    setErrorMsg(null);
  };

  const handleSend = async (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    if (!input.trim() || loading) return;

    if (!config || !config.apiKey) {
      setIsSettingsOpen(true);
      return;
    }

    const userPrompt = input;
    setInput("");
    setErrorMsg(null);

    // Append user message
    const updatedMessages: Message[] = [...messages, { role: "user", content: userPrompt }];
    setMessages(updatedMessages);
    setLoading(true);

    try {
      const responseText = await fetchLLMCompletion(updatedMessages);
      setMessages((prev) => [...prev, { role: "assistant", content: responseText }]);
    } catch (err) {
      console.error(err);
      setErrorMsg(String(err));
    } finally {
      setLoading(false);
    }
  };

  const fetchLLMCompletion = async (history: Message[]): Promise<string> => {
    if (!config) throw new Error("AI Setup required");

    const systemPrompt = `You are an expert SAS and PAS (Practical Analytics Studio) database programmer.
Your goal is to help the user write, debug, and explain SAS DATA step programs and PROC SQL scripts.

Guidelines:
1. Always generate clean, syntactically correct SAS/PAS code.
2. PAS supports DATA steps (with set, merge, by, first., last., retain, array, do loops) and PROC SQL (backed by DuckDB), PROC SORT, PROC PRINT, and PROC TRANSPOSE.
3. If you generate a block of code, wrap it inside standard markdown code blocks, like this:
\`\`\`sas
data work.example;
    set input_ds;
run;
\`\`\`
4. When writing code, return ONLY the code blocks or keep explanations extremely concise. Always prioritize valid, functional code.

Context:
${activeContent ? `Currently open file contents:\n\`\`\`sas\n${activeContent}\n\`\`\`\n` : ""}
${activeSelection ? `Currently selected code segment:\n\`\`\`sas\n${activeSelection}\n\`\`\`\n` : ""}`;

    const headers: Record<string, string> = {
      "Content-Type": "application/json",
    };

    let url = "";
    let body = {};

    switch (config.provider) {
      case "openai": {
        url = config.customUrl || "https://api.openai.com/v1/chat/completions";
        headers["Authorization"] = `Bearer ${config.apiKey}`;
        body = {
          model: config.model,
          messages: [
            { role: "system", content: systemPrompt },
            ...history.map((m) => ({ role: m.role, content: m.content })),
          ],
        };
        break;
      }
      case "deepseek": {
        url = config.customUrl || "https://api.deepseek.com/v1/chat/completions";
        headers["Authorization"] = `Bearer ${config.apiKey}`;
        body = {
          model: config.model,
          messages: [
            { role: "system", content: systemPrompt },
            ...history.map((m) => ({ role: m.role, content: m.content })),
          ],
        };
        break;
      }
      case "openrouter": {
        url = config.customUrl || "https://openrouter.ai/api/v1/chat/completions";
        headers["Authorization"] = `Bearer ${config.apiKey}`;
        headers["HTTP-Referer"] = "https://pas.app";
        headers["X-Title"] = "PAS";
        body = {
          model: config.model,
          messages: [
            { role: "system", content: systemPrompt },
            ...history.map((m) => ({ role: m.role, content: m.content })),
          ],
        };
        break;
      }
      case "anthropic": {
        url = "https://api.anthropic.com/v1/messages";
        headers["x-api-key"] = config.apiKey;
        headers["anthropic-version"] = "2023-06-01";
        headers["dangerously-allow-browser"] = "true";
        body = {
          model: config.model,
          max_tokens: 4096,
          system: systemPrompt,
          messages: history.map((m) => ({
            role: m.role === "system" ? "assistant" : m.role, // Anthropic only supports user/assistant
            content: m.content,
          })),
        };
        break;
      }
      case "gemini": {
        url = `https://generativelanguage.googleapis.com/v1beta/models/${config.model}:generateContent?key=${config.apiKey}`;
        
        // Convert history to Gemini format
        const contents = [
          {
            role: "user",
            parts: [{ text: systemPrompt + "\n\nUnderstood. Please prompt me for the code task." }],
          },
          {
            role: "model",
            parts: [{ text: "Understood. I will act as a SAS/PAS programming assistant." }],
          },
          ...history.map((m) => ({
            role: m.role === "user" ? "user" : "model",
            parts: [{ text: m.content }],
          })),
        ];

        body = { contents };
        break;
      }
      default:
        throw new Error(`Unsupported provider: ${config.provider}`);
    }

    const res = await fetch(url, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
    });

    if (!res.ok) {
      const errText = await res.text();
      let parsedErr = errText;
      try {
        const json = JSON.parse(errText);
        parsedErr = json.error?.message || json.message || errText;
      } catch (_) {}
      throw new Error(`API Error (${res.status}): ${parsedErr}`);
    }

    const data = await res.json();

    // Extract text depending on provider
    if (config.provider === "openai" || config.provider === "deepseek" || config.provider === "openrouter") {
      return data.choices?.[0]?.message?.content || "";
    } else if (config.provider === "anthropic") {
      return data.content?.[0]?.text || "";
    } else if (config.provider === "gemini") {
      return data.candidates?.[0]?.content?.parts?.[0]?.text || "";
    }

    return "";
  };

  const insertSuggestedContext = (text: string) => {
    setInput(text);
  };

  const clearChat = () => {
    setMessages([]);
    setErrorMsg(null);
  };

  // Helper to parse text and extract code snippets, returning text mixed with CodeBlock items
  const renderMessageContent = (content: string) => {
    const codeBlockRegex = /```(?:sas|sql)?([\s\S]*?)```/g;
    const parts = [];
    let lastIndex = 0;
    let match;

    while ((match = codeBlockRegex.exec(content)) !== null) {
      const textBefore = content.substring(lastIndex, match.index);
      if (textBefore.trim()) {
        parts.push(<p key={`text-${match.index}`} className="chat-text">{textBefore}</p>);
      }

      const code = match[1].trim();
      parts.push(
        <div key={`code-${match.index}`} className="ai-code-snippet">
          <pre><code>{code}</code></pre>
          <div className="snippet-actions">
            <button
              onClick={() => onInsertCode(code)}
              title="Insert at cursor position in editor"
            >
              Insert
            </button>
            <button
              onClick={() => onReplaceCode(code)}
              title="Replace highlighted selection in editor"
              disabled={!activeSelection}
            >
              Replace
            </button>
            <button
              onClick={() => onNewTab(code)}
              title="Write to a new tab"
            >
              New Tab
            </button>
          </div>
        </div>
      );
      lastIndex = codeBlockRegex.lastIndex;
    }

    const remainingText = content.substring(lastIndex);
    if (remainingText.trim() || parts.length === 0) {
      parts.push(<p key="text-end" className="chat-text">{remainingText || content}</p>);
    }

    return parts;
  };

  return (
    <div className="ai-chat-panel">
      <div className="panel-header">
        <span className="title">AI Assistant</span>
        <div className="actions">
          <button className="icon-btn" onClick={clearChat} title="Clear Chat history">
            Clear
          </button>
          <button className="icon-btn" onClick={() => setIsSettingsOpen(true)} title="AI Setup Configuration">
            Setup
          </button>
        </div>
      </div>

      <div className="chat-body" ref={listRef}>
        {messages.length === 0 && (
          <div className="empty-state">
            <h4>Welcome to the PAS AI Assistant!</h4>
            <p>Type a prompt to write, edit, or refactor SAS DATA steps and SQL statements.</p>
            
            <div className="suggestion-cards">
              <button onClick={() => insertSuggestedContext("Create a mock dataset called sales representing 10 records with region, item, and qty.")}>
                📝 Create a Sales Mock dataset
              </button>
              <button 
                onClick={() => insertSuggestedContext("Rewrite this SAS DATA step code to compute total compensation and filter out values below 6000.")}
                disabled={!activeContent}
              >
                ⚙️ Refactor open tab code
              </button>
              <button onClick={() => insertSuggestedContext("Write a SAS macro to loop through a given count and append table outputs.")}>
                📦 Generate a SAS Macro loop
              </button>
            </div>
            
            {!config && (
              <div className="setup-warning">
                <p>⚠️ Click <strong>Setup</strong> at the top to configure your API key.</p>
              </div>
            )}
          </div>
        )}

        {messages.map((m, i) => (
          <div key={i} className={`chat-message ${m.role}`}>
            <div className="message-header">
              <span className="sender">{m.role === "user" ? "You" : `${config?.provider.toUpperCase() || "AI"} Assistant`}</span>
            </div>
            <div className="message-body">
              {m.role === "user" ? <p className="chat-text">{m.content}</p> : renderMessageContent(m.content)}
            </div>
          </div>
        ))}

        {loading && (
          <div className="chat-message assistant thinking">
            <div className="message-header">
              <span className="sender">Assistant is thinking...</span>
            </div>
            <div className="message-body">
              <div className="thinking-loader">
                <span></span>
                <span></span>
                <span></span>
              </div>
            </div>
          </div>
        )}

        {errorMsg && (
          <div className="chat-error-card">
            <strong>API Connection Error:</strong>
            <p>{errorMsg}</p>
            <button className="btn-secondary btn-sm" onClick={() => setIsSettingsOpen(true)}>
              Check Settings
            </button>
          </div>
        )}
      </div>

      <form onSubmit={handleSend} className="chat-input-form">
        <input
          type="text"
          placeholder={config ? `Ask ${config.provider.toUpperCase()} Assistant...` : "Setup AI credentials first..."}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          disabled={loading}
        />
        <button type="submit" className="btn-primary" disabled={loading || !input.trim()}>
          Send
        </button>
      </form>

      <AISettingsModal
        isOpen={isSettingsOpen}
        onClose={() => setIsSettingsOpen(false)}
        onSave={saveConfig}
        initialConfig={config}
      />
    </div>
  );
}
