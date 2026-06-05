/**
 * Smoke tests for DatasetViewer.
 *
 * All Tauri `invoke` calls are intercepted by a vi.mock so this can run
 * in jsdom without a real Rust backend.
 */
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { DatasetViewer } from "../DatasetViewer";

// ---------------------------------------------------------------------------
// Arrow IPC helper – produce a minimal valid Arrow IPC stream with one batch
// containing two columns (name: Utf8, age: Int32) and two rows.
// We use the real apache-arrow library so the decode path is exercised.
// ---------------------------------------------------------------------------
import { tableToIPC, Table, Schema, Field, Utf8, Int32, Float64, vectorFromArray } from "apache-arrow";

function buildArrowIpc(totalRows: number, offset: number): ArrayBuffer {
  const nameVec = vectorFromArray(["Alice", "Bob"], new Utf8());
  const ageVec = vectorFromArray([30, 25], new Int32());
  const schema = new Schema([
    new Field("name", new Utf8()),
    new Field("age", new Int32()),
  ], new Map([
    ["total_rows", String(totalRows)],
    ["offset", String(offset)],
  ]));
  const table = new Table(schema, { name: nameVec, age: ageVec });
  const buf = tableToIPC(table, "stream");
  // slice() always returns a plain ArrayBuffer (never SharedArrayBuffer)
  return buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength) as ArrayBuffer;
}

function buildFormattedArrowIpc(): ArrayBuffer {
  const dateVec = vectorFromArray([22295], new Int32());
  const salaryVec = vectorFromArray([62000], new Float64());
  const schema = new Schema([
    new Field("hire_date", new Int32(), true, new Map([["pas_format", "date9."]])),
    new Field("base_salary", new Float64(), true, new Map([["pas_format", "dollar12.2"]])),
  ], new Map([
    ["total_rows", "1"],
    ["offset", "0"],
  ]));
  const table = new Table(schema, { hire_date: dateVec, base_salary: salaryVec });
  const buf = tableToIPC(table, "stream");
  return buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength) as ArrayBuffer;
}

// ---------------------------------------------------------------------------
// Mock @tauri-apps/api/core so tests don't need a Tauri runtime.
// ---------------------------------------------------------------------------
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import { invoke } from "@tauri-apps/api/core";
const mockInvoke = vi.mocked(invoke);

const TEST_DS = { libref: "work", name: "demo" };

beforeEach(() => {
  vi.clearAllMocks();
  mockInvoke.mockResolvedValue(buildArrowIpc(2, 0));
});

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
describe("DatasetViewer", () => {
  it("renders column headers from the Arrow schema", async () => {
    render(<DatasetViewer ds={TEST_DS} />);
    await waitFor(() => expect(screen.getByText("name")).toBeInTheDocument());
    expect(screen.getByText("age")).toBeInTheDocument();
  });

  it("renders the correct number of rows", async () => {
    render(<DatasetViewer ds={TEST_DS} />);
    await waitFor(() => expect(screen.getByText("Alice")).toBeInTheDocument());
    expect(screen.getByText("Bob")).toBeInTheDocument();
  });

  it("shows total row count in the toolbar", async () => {
    render(<DatasetViewer ds={TEST_DS} />);
    await waitFor(() => expect(screen.getByText(/of 2/)).toBeInTheDocument());
  });

  it("shows dataset name in the toolbar", async () => {
    render(<DatasetViewer ds={TEST_DS} />);
    await waitFor(() => expect(screen.getByText(/WORK\.demo/)).toBeInTheDocument());
  });

  it("formats cells using Arrow field metadata", async () => {
    mockInvoke.mockResolvedValue(buildFormattedArrowIpc());
    render(<DatasetViewer ds={TEST_DS} />);
    await waitFor(() => expect(screen.getByText("15JAN2021")).toBeInTheDocument());
    expect(screen.getByText("$62,000.00")).toBeInTheDocument();
  });

  it("invokes dataset_page_arrow with correct args on mount", async () => {
    render(<DatasetViewer ds={TEST_DS} />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalled());
    expect(mockInvoke).toHaveBeenCalledWith("dataset_page_arrow", expect.objectContaining({
      libref: "work",
      name: "demo",
      offset: 0,
    }));
  });

  it("sends filter value on column filter input", async () => {
    const user = userEvent.setup();
    render(<DatasetViewer ds={TEST_DS} />);
    // Wait for initial render
    await waitFor(() => screen.getByText("Alice"));
    mockInvoke.mockResolvedValue(buildArrowIpc(1, 0));

    // Type a filter in the first column's filter input
    const inputs = screen.getAllByPlaceholderText("filter…");
    await user.type(inputs[0], "Alic");

    // After debounce, invoke should be called with filters
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenLastCalledWith(
        "dataset_page_arrow",
        expect.objectContaining({ filters: expect.objectContaining({ name: "Alic" }) }),
      ),
    );
  });
});
