import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
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
  onAddToProject: (code: string) => void;
  isProjectOpen: boolean;
  customTrigger?: { prompt: string; timestamp: number } | null;
  workspaceContext: string;
}

export function AIChatPanel({
  activeContent,
  activeSelection,
  onInsertCode,
  onReplaceCode,
  onNewTab,
  onAddToProject,
  isProjectOpen,
  customTrigger,
  workspaceContext,
}: Props) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [config, setConfig] = useState<AIConfig | null>(null);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Load non-secret configuration from localStorage on mount.
  useEffect(() => {
    localStorage.removeItem("pas.ai_config");
    const saved = localStorage.getItem("pas.ai_config_public");
    if (saved) {
      try {
        const parsed = JSON.parse(saved);
        setConfig({ ...parsed, apiKey: "" });
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

  const saveConfig = async (newConfig: AIConfig) => {
    await invoke("set_ai_config", {
      config: {
        provider: newConfig.provider,
        apiKey: newConfig.apiKey,
        model: newConfig.model,
        customUrl: newConfig.customUrl,
      },
    });
    setConfig(newConfig);
    localStorage.setItem("pas.ai_config_public", JSON.stringify({
      provider: newConfig.provider,
      model: newConfig.model,
      customUrl: newConfig.customUrl,
    }));
    setErrorMsg(null);
  };

  // Listen for Monaco right-click context menu actions
  useEffect(() => {
    if (customTrigger) {
      sendMessageDirectly(customTrigger.prompt);
    }
  }, [customTrigger]);

  const sendMessageDirectly = async (promptText: string) => {
    if (!promptText.trim() || loading) return;

    if (!config) {
      setIsSettingsOpen(true);
      return;
    }

    setErrorMsg(null);

    // Closure-safe state capture of messages history
    let currentMessages: Message[] = [];
    setMessages((prev) => {
      currentMessages = [...prev, { role: "user", content: promptText }];
      return currentMessages;
    });
    setLoading(true);

    try {
      const responseText = await fetchLLMCompletion(currentMessages);
      setMessages((prev) => [...prev, { role: "assistant", content: responseText }]);
    } catch (err) {
      console.error(err);
      setErrorMsg(String(err));
    } finally {
      setLoading(false);
    }
  };

  const handleSend = (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    const promptText = input;
    setInput("");
    sendMessageDirectly(promptText);
  };

  const fetchLLMCompletion = async (history: Message[]): Promise<string> => {
    if (!config) throw new Error("AI Setup required");

    // Gather system metadata
    const localTime = new Date().toLocaleTimeString();
    const localDate = new Date().toLocaleDateString();
    const osPlatform = "Linux";

    const systemPrompt = `You are an expert SAS and PAS (Practical Analytics Studio) database programmer.
Your goal is to help the user write, debug, and explain SAS DATA step programs and PROC SQL scripts.

System Metadata:
- Platform: ${osPlatform}
- Current Local Time: ${localDate} ${localTime}
- Backed by: DuckDB (for PROC SQL queries)

Programming Constraints & Instructions:
1. Always generate clean, syntactically correct SAS/PAS code.
2. PAS supports DATA steps (with set, merge, by, first., last., retain, array, do loops) and PROC SQL (backed by DuckDB), PROC SORT, PROC PRINT, and PROC TRANSPOSE.
3. If assigning PROC SQL query results into macro variables, utilize the SAS trimmed syntax:
   \`\`\`sas
   select count(*) into :variable trimmed from table;
   \`\`\`
4. Wrap all code blocks in triple-backticks with the explicit language tag (e.g. \`\`\`sas or \`\`\`sql) to ensure the editor's UI snippet card actions can parse and apply them. Never omit the language tag.
5. Avoid excessive conversational filler or introductory greetings (e.g., "Sure, I'd be happy to help!"). Jump straight to the core explanation or code solution.
6. If requested to explain or refactor, briefly detail your logic in 1-2 concise bullet points before showing the code.

Context Information:
The user's active workspace state is provided below inside structured XML tags. Analyze this context to answer questions accurately and tailor code references to the active project's libraries and datasets.

<workspace_context>
${workspaceContext}
${activeContent ? `<open_file_buffer>\n${activeContent}\n</open_file_buffer>` : ""}
${activeSelection ? `<active_selection>\n${activeSelection}\n</active_selection>` : ""}
</workspace_context>`;

    return invoke<string>("ai_completion", {
      request: {
        provider: config.provider,
        model: config.model,
        customUrl: config.customUrl,
        systemPrompt,
        messages: history,
      },
    });
  };

  const insertSuggestedContext = (text: string) => {
    setInput(text);
  };

  const clearChat = () => {
    setMessages([]);
    setErrorMsg(null);
  };

  // Helper to parse text and extract code snippets, returning text mixed with CodeBlock items
  // Helper to parse plain text segments into rich markdown react elements
  const parseMarkdownToReact = (text: string, keyPrefix: string): React.ReactNode[] => {
    const paragraphs = text.split(/\n\s*\n/);
    return paragraphs.map((para, paraIdx) => {
      para = para.trim();
      if (!para) return null;

      // Check if it is a heading
      if (para.startsWith("#")) {
        const match = para.match(/^(#{1,6})\s+(.*)$/);
        if (match) {
          const level = match[1].length;
          const headingText = match[2];
          const Tag = `h${Math.min(6, level + 1)}` as keyof JSX.IntrinsicElements;
          return (
            <Tag key={`${keyPrefix}-h-${paraIdx}`} className="markdown-heading">
              {parseInlineMarkdown(headingText)}
            </Tag>
          );
        }
      }

      // Check if it is a blockquote
      if (para.startsWith(">")) {
        const quoteText = para.replace(/^>\s*/gm, "");
        return (
          <blockquote key={`${keyPrefix}-q-${paraIdx}`} className="markdown-blockquote">
            {parseMarkdownToReact(quoteText, `${keyPrefix}-q-${paraIdx}`)}
          </blockquote>
        );
      }

      // Check if it is an unordered list
      if (para.startsWith("- ") || para.startsWith("* ")) {
        const items = para.split(/\n[-*]\s+/).map((item, itemIdx) => {
          const cleaned = itemIdx === 0 ? item.replace(/^[-*]\s+/, "") : item;
          return <li key={itemIdx}>{parseInlineMarkdown(cleaned)}</li>;
        });
        return (
          <ul key={`${keyPrefix}-ul-${paraIdx}`} className="markdown-list">
            {items}
          </ul>
        );
      }

      // Check if it is an ordered list
      if (/^\d+\.\s+/.test(para)) {
        const items = para.split(/\n\d+\.\s+/).map((item, itemIdx) => {
          const cleaned = itemIdx === 0 ? item.replace(/^\d+\.\s+/, "") : item;
          return <li key={itemIdx}>{parseInlineMarkdown(cleaned)}</li>;
        });
        return (
          <ol key={`${keyPrefix}-ol-${paraIdx}`} className="markdown-list">
            {items}
          </ol>
        );
      }

      // Default paragraph
      return (
        <p key={`${keyPrefix}-p-${paraIdx}`} className="chat-text">
          {parseInlineMarkdown(para)}
        </p>
      );
    }).filter(Boolean) as React.ReactNode[];
  };

  const parseInlineMarkdown = (text: string): React.ReactNode => {
    const regex = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*]+\*|_[^_]+_)/g;
    const parts = text.split(regex);
    return parts.map((part, index) => {
      if (part.startsWith("`") && part.endsWith("`")) {
        return (
          <code key={index} className="markdown-inline-code">
            {part.slice(1, -1)}
          </code>
        );
      }
      if (part.startsWith("**") && part.endsWith("**")) {
        return <strong key={index}>{part.slice(2, -2)}</strong>;
      }
      if ((part.startsWith("*") && part.endsWith("*")) || (part.startsWith("_") && part.endsWith("_"))) {
        return <em key={index}>{part.slice(1, -1)}</em>;
      }
      return part;
    });
  };

  // Helper to parse text and extract code snippets, returning text mixed with CodeBlock items
  const renderMessageContent = (content: string) => {
    const codeBlockRegex = /```(?:sas|sql)?([\s\S]*?)```/g;
    const parts: React.ReactNode[] = [];
    let lastIndex = 0;
    let match;

    while ((match = codeBlockRegex.exec(content)) !== null) {
      const textBefore = content.substring(lastIndex, match.index);
      if (textBefore.trim()) {
        parts.push(...parseMarkdownToReact(textBefore, `text-${match.index}`));
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
            <button
              onClick={() => onAddToProject(code)}
              title={isProjectOpen ? "Add this program to the current project JSON" : "Open a project to enable adding programs"}
              disabled={!isProjectOpen}
            >
              Add to Project
            </button>
          </div>
        </div>
      );
      lastIndex = codeBlockRegex.lastIndex;
    }

    const remainingText = content.substring(lastIndex);
    if (remainingText.trim() || parts.length === 0) {
      parts.push(...parseMarkdownToReact(remainingText || content, "text-end"));
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
