import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AIChatPanel } from "../AIChatPanel";
import * as tauriCore from "@tauri-apps/api/core";
import * as tauriEvent from "@tauri-apps/api/event";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function renderPanel() {
  return render(
    <AIChatPanel
      activeContent=""
      activeSelection=""
      onInsertCode={vi.fn()}
      onReplaceCode={vi.fn()}
      onNewTab={vi.fn()}
      onAddToProject={vi.fn()}
      isProjectOpen
      workspaceContext=""
      readEditFile={vi.fn()}
      onApplyEdit={vi.fn()}
      onReviewEdit={vi.fn()}
    />,
  );
}

function installStorageShim() {
  const values = new Map<string, string>();
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => values.set(key, value),
    removeItem: (key: string) => values.delete(key),
    clear: () => values.clear(),
    key: (index: number) => Array.from(values.keys())[index] ?? null,
    get length() {
      return values.size;
    },
  } as Storage;
  Object.defineProperty(window, "localStorage", { value: storage, configurable: true });
  Object.defineProperty(globalThis, "localStorage", { value: storage, configurable: true });
}

describe("AIChatPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    installStorageShim();
    window.localStorage.clear();
    window.localStorage.setItem(
      "pas.ai_config_public",
      JSON.stringify({
        provider: "openai",
        model: "gpt-5.1",
        customUrl: "",
        authMode: "api_key",
      }),
    );
  });

  it("renders the prompt control as a multiline textarea", async () => {
    vi.mocked(tauriEvent.listen).mockResolvedValue(() => undefined);
    vi.mocked(tauriCore.invoke).mockImplementation((cmd: string) => {
      if (cmd === "openai_oauth_status") return Promise.resolve({ signedIn: false });
      if (cmd === "list_ai_models") return Promise.reject(new Error("offline"));
      return Promise.resolve(null);
    });

    renderPanel();

    const prompt = await screen.findByPlaceholderText(/ask openai agent/i);
    expect(prompt.tagName).toBe("TEXTAREA");
  });

  it("keeps the prompt open on Shift+Enter and inserts a newline", async () => {
    const user = userEvent.setup();
    vi.mocked(tauriEvent.listen).mockResolvedValue(() => undefined);
    vi.mocked(tauriCore.invoke).mockImplementation((cmd: string) => {
      if (cmd === "openai_oauth_status") return Promise.resolve({ signedIn: false });
      if (cmd === "list_ai_models") return Promise.reject(new Error("offline"));
      return Promise.resolve(null);
    });

    renderPanel();

    const prompt = await screen.findByPlaceholderText(/ask openai agent/i);
    await user.click(prompt);
    await user.keyboard("first line{Shift>}{Enter}{/Shift}second line");

    expect(prompt).toHaveValue("first line\nsecond line");
    expect(tauriCore.invoke).not.toHaveBeenCalledWith("ai_completion_stream", expect.anything());
  });

  it("shows a compact loading indicator while waiting for the Agent response", async () => {
    const completion = deferred<string>();
    vi.mocked(tauriEvent.listen).mockResolvedValue(() => undefined);
    vi.mocked(tauriCore.invoke).mockImplementation((cmd: string) => {
      if (cmd === "openai_oauth_status") return Promise.resolve({ signedIn: false });
      if (cmd === "list_ai_models") return Promise.reject(new Error("offline"));
      if (cmd === "ai_completion_stream") return completion.promise;
      return Promise.resolve(null);
    });

    renderPanel();

    await userEvent.type(screen.getByPlaceholderText(/ask openai agent/i), "Create a report");
    await userEvent.click(screen.getByRole("button", { name: /send/i }));

    expect(await screen.findByLabelText(/agent is working/i)).toBeInTheDocument();
    expect(screen.queryByText(/reading workspace context/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/checking open project and files/i)).not.toBeInTheDocument();
    expect(screen.queryByText(/drafting response/i)).not.toBeInTheDocument();

    completion.resolve("Here is the report.");
    await waitFor(() => expect(screen.getByText("Here is the report.")).toBeInTheDocument());
    expect(screen.queryByLabelText(/agent is working/i)).not.toBeInTheDocument();
  });

  it("renders streamed assistant chunks before the request completes", async () => {
    const streamDone = deferred<string>();
    const listeners: Array<(event: { payload: unknown }) => void> = [];
    let activeRequestId = "";
    vi.mocked(tauriEvent.listen).mockImplementation((_eventName: string, callback: (event: { payload: unknown }) => void) => {
      listeners.push(callback);
      return Promise.resolve(() => undefined);
    });
    vi.mocked(tauriCore.invoke).mockImplementation((cmd: string, args?: unknown) => {
      if (cmd === "openai_oauth_status") return Promise.resolve({ signedIn: false });
      if (cmd === "list_ai_models") return Promise.reject(new Error("offline"));
      if (cmd === "ai_completion_stream") {
        activeRequestId = (args as { requestId: string }).requestId;
        return streamDone.promise;
      }
      return Promise.resolve(null);
    });

    renderPanel();

    await userEvent.type(screen.getByPlaceholderText(/ask openai agent/i), "Create a report");
    await userEvent.click(screen.getByRole("button", { name: /send/i }));

    await waitFor(() => expect(tauriEvent.listen).toHaveBeenCalledWith("pas://ai-stream", expect.any(Function)));
    listeners.forEach((callback) =>
      callback({
        payload: {
          requestId: activeRequestId,
          kind: "chunk",
          text: "Here ",
        },
      }),
    );
    expect(await screen.findByText("Here")).toBeInTheDocument();

    listeners.forEach((callback) =>
      callback({
        payload: {
          requestId: activeRequestId,
          kind: "chunk",
          text: "is the report.",
        },
      }),
    );
    expect(await screen.findByText("Here is the report.")).toBeInTheDocument();

    streamDone.resolve("Here is the report.");
    await waitFor(() => expect(screen.queryByText(/reading workspace context/i)).not.toBeInTheDocument());
  });
});
