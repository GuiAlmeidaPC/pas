import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AISettingsModal } from "../AISettingsModal";

describe("AISettingsModal", () => {
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
});
