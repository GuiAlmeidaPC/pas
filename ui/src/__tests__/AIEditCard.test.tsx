import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { AIEditCard } from "../ai/AIEditCard";
import * as tauriCore from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("AIEditCard", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders a create edit with all green lines and calls onApply", async () => {
    // read_file rejects → file does not exist → create is allowed.
    (tauriCore.invoke as ReturnType<typeof vi.fn>).mockRejectedValue(new Error("not found"));
    const onApply = vi.fn().mockResolvedValue(undefined);
    render(
      <AIEditCard
        edit={{ kind: "create", path: "programs/new.pas", contents: "data x;\nrun;" }}
        isProjectOpen
        onApply={onApply}
        onReview={vi.fn()}
      />
    );
    expect(screen.getByText("programs/new.pas")).toBeInTheDocument();
    expect(screen.getByText("new")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole("button", { name: /accept/i })).not.toBeDisabled());
    await userEvent.click(screen.getByRole("button", { name: /accept/i }));
    expect(onApply).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(screen.getByText(/applied/i)).toBeInTheDocument());
  });

  it("rejects a create when the target path already exists", async () => {
    (tauriCore.invoke as ReturnType<typeof vi.fn>).mockResolvedValue("existing content\n");
    render(
      <AIEditCard
        edit={{ kind: "create", path: "programs/existing.pas", contents: "data x;\nrun;" }}
        isProjectOpen
        onApply={vi.fn()}
        onReview={vi.fn()}
      />
    );
    await waitFor(() => expect(screen.getByText(/already exists/i)).toBeInTheDocument());
    expect(screen.getByRole("button", { name: /accept/i })).toBeDisabled();
  });

  it("rejects non-pas edit paths before contacting the backend", async () => {
    render(
      <AIEditCard
        edit={{ kind: "create", path: "programs/new.txt", contents: "x" }}
        isProjectOpen
        onApply={vi.fn()}
        onReview={vi.fn()}
      />
    );
    await waitFor(() => expect(screen.getByText(/only \.pas files/i)).toBeInTheDocument());
    expect(screen.getByRole("button", { name: /accept/i })).toBeDisabled();
    expect(tauriCore.invoke).not.toHaveBeenCalled();
  });

  it("does not allow create when existence check fails for reasons other than missing file", async () => {
    (tauriCore.invoke as ReturnType<typeof vi.fn>).mockRejectedValue(new Error("Access denied"));
    render(
      <AIEditCard
        edit={{ kind: "create", path: "programs/new.pas", contents: "x" }}
        isProjectOpen
        onApply={vi.fn()}
        onReview={vi.fn()}
      />
    );
    await waitFor(() => expect(screen.getByText(/access denied/i)).toBeInTheDocument());
    expect(screen.getByRole("button", { name: /accept/i })).toBeDisabled();
  });

  it("disables actions when no project is open", async () => {
    (tauriCore.invoke as ReturnType<typeof vi.fn>).mockRejectedValue(new Error("not found"));
    render(
      <AIEditCard
        edit={{ kind: "create", path: "x.pas", contents: "x" }}
        isProjectOpen={false}
        onApply={vi.fn()}
        onReview={vi.fn()}
      />
    );
    await waitFor(() => expect(screen.getByText(/open a project/i)).toBeInTheDocument());
    expect(screen.getByText(/open a project/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /accept/i })).toBeDisabled();
  });

  it("renders a patch edit by fetching current contents and showing -/+ lines", async () => {
    (tauriCore.invoke as ReturnType<typeof vi.fn>).mockResolvedValue("data want; set have; run;\n");
    render(
      <AIEditCard
        edit={{
          kind: "patch",
          path: "programs/foo.pas",
          hunks: [{ search: "data want; set have; run;", replace: "data want; set have; where x>0; run;" }],
        }}
        isProjectOpen
        onApply={vi.fn()}
        onReview={vi.fn()}
      />
    );
    await waitFor(() => {
      expect(screen.getByText(/data want; set have; run;/)).toBeInTheDocument();
      expect(screen.getByText(/data want; set have; where x>0; run;/)).toBeInTheDocument();
    });
  });

  it("surfaces a stale-base error when SEARCH no longer matches", async () => {
    (tauriCore.invoke as ReturnType<typeof vi.fn>).mockResolvedValue("UNRELATED\n");
    render(
      <AIEditCard
        edit={{
          kind: "patch",
          path: "programs/foo.pas",
          hunks: [{ search: "data want;", replace: "data x;" }],
        }}
        isProjectOpen
        onApply={vi.fn()}
        onReview={vi.fn()}
      />
    );
    await waitFor(() => {
      expect(screen.getByText(/file changed since proposal/i)).toBeInTheDocument();
    });
    expect(screen.getByRole("button", { name: /accept/i })).toBeDisabled();
  });

  it("renders a protocol error edit without contacting the backend", () => {
    render(
      <AIEditCard
        edit={{ kind: "error", path: "a.pas", reason: "bad mode", raw: "" }}
        isProjectOpen
        onApply={vi.fn()}
        onReview={vi.fn()}
      />
    );
    expect(screen.getByText(/bad mode/)).toBeInTheDocument();
    expect(tauriCore.invoke).not.toHaveBeenCalled();
  });

  it("keeps Review enabled in stale state and still renders a diff", async () => {
    const onReview = vi.fn();
    (tauriCore.invoke as ReturnType<typeof vi.fn>).mockResolvedValue("a\nb\nc\n");
    render(
      <AIEditCard
        edit={{
          kind: "patch",
          path: "x.pas",
          hunks: [
            { search: "missing", replace: "y" },
            { search: "b", replace: "BB" },
          ],
        }}
        isProjectOpen
        onApply={vi.fn()}
        onReview={onReview}
      />
    );
    await waitFor(() => {
      expect(screen.getByText(/file changed since proposal/i)).toBeInTheDocument();
    });
    const reviewBtn = screen.getByRole("button", { name: /review in editor/i });
    expect(reviewBtn).not.toBeDisabled();
    await userEvent.click(reviewBtn);
    expect(onReview).toHaveBeenCalledTimes(1);
    expect(onReview.mock.calls[0][1]).toMatchObject({ status: "stale" });
    expect(screen.getByRole("button", { name: /accept/i })).toBeDisabled();
  });

  it("uses the supplied open-tab reader before falling back to backend reads", async () => {
    const readFile = vi.fn().mockResolvedValue({ content: "old", source: "tab" });
    render(
      <AIEditCard
        edit={{ kind: "patch", path: "programs/open.pas", hunks: [{ search: "old", replace: "new" }] }}
        isProjectOpen
        readFile={readFile}
        onApply={vi.fn()}
        onReview={vi.fn()}
      />
    );
    await waitFor(() => expect(screen.getByText("new")).toBeInTheDocument());
    expect(readFile).toHaveBeenCalledWith("programs/open.pas");
    expect(tauriCore.invoke).not.toHaveBeenCalled();
  });
});
