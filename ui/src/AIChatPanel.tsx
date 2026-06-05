import { useState, useEffect, useLayoutEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { AISettingsModal, availableModels, type AIConfig, type OAuthStatus } from "./AISettingsModal";
import { parseEditBlocks, type EditFileSnapshot, type ProposedEdit, type ResolvedEdit } from "./ai/editProtocol";
import { AIEditCard } from "./ai/AIEditCard";

interface Message {
  role: "user" | "assistant" | "system";
  content: string;
}

interface AiCompletionRequest {
  provider: string;
  model: string;
  customUrl?: string;
  authMode?: string;
  systemPrompt: string;
  messages: Message[];
}

interface AiStreamPayload {
  requestId: string;
  kind: "chunk" | "done" | "error";
  text?: string;
  message?: string;
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
  readEditFile: (path: string) => Promise<EditFileSnapshot>;
  onApplyEdit: (edit: ProposedEdit, resolved: ResolvedEdit) => Promise<void>;
  onReviewEdit: (edit: ProposedEdit, resolved: ResolvedEdit) => void;
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
  readEditFile,
  onApplyEdit,
  onReviewEdit,
}: Props) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [config, setConfig] = useState<AIConfig | null>(null);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [streamingStarted, setStreamingStarted] = useState(false);
  const [oauthStatus, setOauthStatus] = useState<OAuthStatus | null>(null);
  const [dynamicModels, setDynamicModels] = useState<string[] | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  const loadCachedModels = (cfg: AIConfig) => {
    if (cfg.authMode === "chatgpt") {
      setDynamicModels(null);
      return;
    }
    const cacheKey = `pas.ai_models.${cfg.provider}`;
    const cached = localStorage.getItem(cacheKey);
    setDynamicModels(cached ? JSON.parse(cached) : null);
  };

  const cacheDynamicModels = (cfg: AIConfig, models: string[]) => {
    if (cfg.authMode === "chatgpt") {
      setDynamicModels(null);
      return;
    }
    const cacheKey = `pas.ai_models.${cfg.provider}`;
    localStorage.setItem(cacheKey, JSON.stringify(models));
    setDynamicModels(models);
  };

  const resizeInput = () => {
    const el = inputRef.current;
    if (!el) return;
    el.style.height = "auto";
    const styles = window.getComputedStyle(el);
    const lineHeight = Number.parseFloat(styles.lineHeight) || 18;
    const padding = Number.parseFloat(styles.paddingTop)
      + Number.parseFloat(styles.paddingBottom)
      + Number.parseFloat(styles.borderTopWidth)
      + Number.parseFloat(styles.borderBottomWidth);
    const maxHeight = lineHeight * 8 + padding;
    const nextHeight = Math.min(el.scrollHeight, maxHeight);
    el.style.height = `${nextHeight}px`;
    el.style.overflowY = el.scrollHeight > maxHeight ? "auto" : "hidden";
  };

  // Fetch ChatGPT sign-in status from the Rust backend.
  const refreshOauthStatus = async () => {
    try {
      const s = await invoke<OAuthStatus>("openai_oauth_status");
      setOauthStatus(s);
    } catch (e) {
      console.error("Failed to read OAuth status", e);
    }
  };

  useEffect(() => {
    refreshOauthStatus();
  }, []);

  const handleOauthLogin = async () => {
    const s = await invoke<OAuthStatus>("openai_oauth_login");
    setOauthStatus(s);
  };

  const handleOauthLogout = async () => {
    await invoke("openai_oauth_logout");
    await refreshOauthStatus();
  };

  // Load non-secret configuration from localStorage on mount.
  useEffect(() => {
    localStorage.removeItem("pas.ai_config");
    const saved = localStorage.getItem("pas.ai_config_public");
    if (saved) {
      try {
        const parsed = JSON.parse(saved);
        setConfig({ ...parsed, apiKey: "" });
        loadCachedModels({ ...parsed, apiKey: "" });
      } catch (e) {
        console.error("Failed to parse saved AI config", e);
      }
    }
  }, []);

  useLayoutEffect(() => {
    resizeInput();
  }, [input]);

  // Scroll to bottom on new messages
  useEffect(() => {
    if (listRef.current) {
      listRef.current.scrollTop = listRef.current.scrollHeight;
    }
  }, [messages, loading]);

  // Persist only the non-secret config (never the API key) to localStorage.
  const persistPublicConfig = (cfg: AIConfig) => {
    localStorage.setItem("pas.ai_config_public", JSON.stringify({
      provider: cfg.provider,
      model: cfg.model,
      customUrl: cfg.customUrl,
      authMode: cfg.authMode,
    }));
  };

  const saveConfig = async (newConfig: AIConfig) => {
    await invoke("set_ai_config", {
      config: {
        provider: newConfig.provider,
        apiKey: newConfig.apiKey,
        model: newConfig.model,
        customUrl: newConfig.customUrl,
        authMode: newConfig.authMode,
      },
    });
    setConfig(newConfig);
    persistPublicConfig(newConfig);
    loadCachedModels(newConfig);
    setErrorMsg(null);
  };

  const fetchModelsForSetup = async (cfg: AIConfig) => {
    await invoke("set_ai_config", {
      config: {
        provider: cfg.provider,
        apiKey: cfg.apiKey,
        model: cfg.model || availableModels(cfg.provider, cfg.authMode)[0] || "placeholder",
        customUrl: cfg.customUrl,
        authMode: cfg.authMode,
      },
    });
    const models = await invoke<string[]>("list_ai_models");
    const nextModels = Array.from(new Set(models.filter(Boolean)));
    cacheDynamicModels(cfg, nextModels);
    return nextModels;
  };

  // Quick model switch from the panel header. The model travels with each
  // request (ai_completion honors request.model), so no backend call is needed.
  const changeModel = (model: string) => {
    if (!config || model === config.model) return;
    const next = { ...config, model };
    setConfig(next);
    persistPublicConfig(next);
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

    // In ChatGPT mode the user must be signed in before sending.
    if (config.authMode === "chatgpt" && !oauthStatus?.signedIn) {
      setIsSettingsOpen(true);
      setErrorMsg("Sign in with ChatGPT in Setup to use this mode.");
      return;
    }

    setErrorMsg(null);

    // Build the history explicitly from current state instead of relying on the
    // setMessages updater running synchronously. React's eager-state
    // optimization that invokes the updater inline is not guaranteed, so the
    // previous side-effect capture could (and did) send an empty history.
    const userMessage: Message = { role: "user", content: promptText };
    const history = [...messages, userMessage];
    setMessages((prev) => [...prev, userMessage, { role: "assistant", content: "" }]);
    setLoading(true);
    setStreamingStarted(false);

    try {
      const responseText = await streamLLMCompletion(history, (chunk) => {
        if (!chunk) return;
        setStreamingStarted(true);
        setMessages((prev) => updateLastAssistantMessage(prev, (content) => content + chunk));
      });
      setMessages((prev) =>
        updateLastAssistantMessage(prev, (content) => responseText || content),
      );
    } catch (err) {
      console.error(err);
      setErrorMsg(String(err));
      setMessages((prev) => prev.filter((message) => message.role !== "assistant" || message.content.trim() !== ""));
    } finally {
      setLoading(false);
      setStreamingStarted(false);
    }
  };

  const handleSend = (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    const promptText = input;
    setInput("");
    sendMessageDirectly(promptText);
  };

  const handleInputKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  const updateLastAssistantMessage = (
    items: Message[],
    update: (content: string) => string,
  ): Message[] => {
    const next = [...items];
    for (let i = next.length - 1; i >= 0; i -= 1) {
      if (next[i].role === "assistant") {
        next[i] = { ...next[i], content: update(next[i].content) };
        return next;
      }
    }
    return [...next, { role: "assistant", content: update("") }];
  };

  const createRequestId = () => {
    if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
      return crypto.randomUUID();
    }
    return `ai-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  };

  const streamLLMCompletion = async (
    history: Message[],
    onChunk: (chunk: string) => void,
  ): Promise<string> => {
    const request = buildLLMRequest(history);
    const requestId = createRequestId();
    let unlisten: UnlistenFn | null = null;
    let streamedText = "";
    let streamError: string | null = null;

    try {
      unlisten = await listen<AiStreamPayload>("pas://ai-stream", (event) => {
        const payload = event.payload;
        if (!payload || payload.requestId !== requestId) return;
        if (payload.kind === "chunk" && payload.text) {
          streamedText += payload.text;
          onChunk(payload.text);
        } else if (payload.kind === "error") {
          streamError = payload.message ?? "AI stream failed";
        }
      });
      const finalText = await invoke<string>("ai_completion_stream", { request, requestId });
      if (streamError) throw new Error(streamError);
      return finalText || streamedText;
    } finally {
      if (unlisten) unlisten();
    }
  };

  const buildLLMRequest = (history: Message[]): AiCompletionRequest => {
    if (!config) throw new Error("AI Setup required");

    // Gather system metadata
    const localTime = new Date().toLocaleTimeString();
    const localDate = new Date().toLocaleDateString();
    const osPlatform = "Linux";
    const searchMarker = "<<<<<<< SEARCH";
    const separatorMarker = "=======";
    const replaceMarker = ">>>>>>> REPLACE";

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

File Edit Protocol:
When the user asks you to modify or create program files, propose edits using \`pas-edit\` fenced
code blocks. The UI will render them as red/green diff cards with Accept/Reject/Review buttons.

Three modes (always include both \`path\` and \`mode\` as quoted attributes):

1. Surgical edit (preferred):
\`\`\`pas-edit path="programs/foo.sas" mode="patch"
${searchMarker}
exact existing text, byte-for-byte
${separatorMarker}
new text
${replaceMarker}
\`\`\`
You may include multiple SEARCH/REPLACE hunks in one block; they apply atomically.

2. New file:
\`\`\`pas-edit path="programs/new.sas" mode="create"
<full file contents>
\`\`\`

3. Full overwrite (only when a patch would be larger than the file):
\`\`\`pas-edit path="programs/big.sas" mode="replace"
<full file contents>
\`\`\`

Rules:
- The SEARCH text must match the current file contents exactly (whitespace included).
- Use file paths from the <active_project> listing in the workspace context.
- Only .sas files can be edited.
- For explanation-only snippets the user will copy by hand, continue to use plain \`\`\`sas blocks — do not use pas-edit for non-applicable code samples.

Context Information:
The user's active workspace state is provided below inside structured XML tags. Analyze this context to answer questions accurately and tailor code references to the active project's libraries and datasets.

<workspace_context>
${workspaceContext}
${activeContent ? `<open_file_buffer>\n${activeContent}\n</open_file_buffer>` : ""}
${activeSelection ? `<active_selection>\n${activeSelection}\n</active_selection>` : ""}
</workspace_context>`;

    return {
      provider: config.provider,
      model: config.model,
      customUrl: config.customUrl,
      authMode: config.authMode,
      systemPrompt,
      messages: history,
    };
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

  // Stable cache of parsed pas-edit blocks keyed by the raw fence text.
  // Without this, every re-render produces fresh ProposedEdit object
  // identities, re-running AIEditCard's effect and re-issuing read_file.
  const editParseCache = useRef<Map<string, ReturnType<typeof parseEditBlocks>[number]>>(new Map());
  const parseEditOnce = (fenceText: string) => {
    const cached = editParseCache.current.get(fenceText);
    if (cached) return cached;
    const [edit] = parseEditBlocks(fenceText);
    if (edit) editParseCache.current.set(fenceText, edit);
    return edit;
  };

  // Helper to parse text and extract code snippets, returning text mixed with CodeBlock items
  const renderMessageContent = (content: string) => {
    const parts: React.ReactNode[] = [];

    // 1. Slice off pas-edit blocks and render each as an AIEditCard.
    const editFence = /```pas-edit\b[^\n]*\n[\s\S]*?\n```/g;
    let cursor = 0;
    let match: RegExpExecArray | null;
    const segments: Array<{ kind: "text" | "edit"; text: string }> = [];
    while ((match = editFence.exec(content)) !== null) {
      if (match.index > cursor) {
        segments.push({ kind: "text", text: content.slice(cursor, match.index) });
      }
      segments.push({ kind: "edit", text: match[0] });
      cursor = match.index + match[0].length;
    }
    if (cursor < content.length) {
      segments.push({ kind: "text", text: content.slice(cursor) });
    }

    segments.forEach((seg, segIdx) => {
      if (seg.kind === "edit") {
        const edit = parseEditOnce(seg.text);
        if (edit) {
          parts.push(
            <AIEditCard
              key={`edit-${segIdx}`}
              edit={edit}
              isProjectOpen={isProjectOpen}
              readFile={readEditFile}
              onApply={onApplyEdit}
              onReview={onReviewEdit}
            />
          );
        }
        return;
      }
      // Existing sas/sql snippet rendering for plain text segments.
      const codeBlockRegex = /```(?:sas|sql)?([\s\S]*?)```/g;
      let lastIndex = 0;
      let m: RegExpExecArray | null;
      while ((m = codeBlockRegex.exec(seg.text)) !== null) {
        const textBefore = seg.text.substring(lastIndex, m.index);
        if (textBefore.trim()) {
          parts.push(...parseMarkdownToReact(textBefore, `text-${segIdx}-${m.index}`));
        }
        const code = m[1].trim();
        parts.push(
          <div key={`code-${segIdx}-${m.index}`} className="ai-code-snippet">
            <pre><code>{code}</code></pre>
            <div className="snippet-actions">
              <button onClick={() => onInsertCode(code)} title="Insert at cursor position in editor">Insert</button>
              <button onClick={() => onReplaceCode(code)} title="Replace highlighted selection in editor" disabled={!activeSelection}>Replace</button>
              <button onClick={() => onNewTab(code)} title="Write to a new tab">New Tab</button>
              <button onClick={() => onAddToProject(code)} title={isProjectOpen ? "Add this program to the current project JSON" : "Open a project to enable adding programs"} disabled={!isProjectOpen}>Add to Project</button>
            </div>
          </div>
        );
        lastIndex = codeBlockRegex.lastIndex;
      }
      const remainingText = seg.text.substring(lastIndex);
      if (remainingText.trim() || lastIndex === 0) {
        parts.push(...parseMarkdownToReact(remainingText || seg.text, `text-end-${segIdx}`));
      }
    });

    return parts;
  };

  return (
    <div className="ai-chat-panel">
      <div className="panel-header">
        <span className="title">Agent</span>
        <div className="actions">
          {config && (
            <select
              className="agent-model-select"
              value={config.model}
              onChange={(e) => changeModel(e.target.value)}
              title="Model"
            >
              {Array.from(new Set([
                ...(dynamicModels && dynamicModels.length > 0
                  ? dynamicModels
                  : availableModels(config.provider, config.authMode)),
                config.model,
              ]))
                .filter(Boolean)
                .map((m) => (
                  <option key={m} value={m}>{m}</option>
                ))}
            </select>
          )}
          <button className="icon-btn" onClick={clearChat} title="Clear Chat history">
            Clear
          </button>
          <button className="icon-btn" onClick={() => setIsSettingsOpen(true)} title="Agent Setup Configuration">
            Setup
          </button>
        </div>
      </div>

      <div className="chat-body" ref={listRef}>
        {messages.length === 0 && (
          <div className="empty-state">
            <h4>Welcome to the PAS Agent!</h4>
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
              <span className="sender">{m.role === "user" ? "You" : `${config?.provider.toUpperCase() || "AI"} Agent`}</span>
            </div>
            <div className="message-body">
              {m.role === "user" ? <p className="chat-text">{m.content}</p> : renderMessageContent(m.content)}
            </div>
          </div>
        ))}

        {loading && !streamingStarted && (
          <div className="chat-message assistant thinking">
            <div className="message-header">
              <span className="sender">Agent is working</span>
            </div>
            <div className="message-body">
              <div className="thinking-loader" aria-live="polite" aria-label="Agent is working">
                <span className="thinking-spinner" aria-hidden="true" />
                <span>Working...</span>
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
        <textarea
          ref={inputRef}
          rows={1}
          placeholder={config ? `Ask ${config.provider.toUpperCase()} Agent...` : "Set up the Agent first..."}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleInputKeyDown}
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
        onFetchModels={fetchModelsForSetup}
        initialConfig={config}
        oauthStatus={oauthStatus}
        onOauthLogin={handleOauthLogin}
        onOauthLogout={handleOauthLogout}
      />
    </div>
  );
}
