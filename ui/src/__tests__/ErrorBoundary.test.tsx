import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ErrorBoundary } from "../ErrorBoundary";

function BrokenView(): never {
  throw new Error("render failed");
}

describe("ErrorBoundary", () => {
  it("renders a recovery screen when a child throws", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    render(
      <ErrorBoundary>
        <BrokenView />
      </ErrorBoundary>,
    );

    expect(
      screen.getByRole("alert", { name: "PAS encountered an interface error" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reload PAS" })).toBeInTheDocument();
    error.mockRestore();
  });
});
