import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import Home from "./Home.svelte";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function response(input: RequestInfo | URL) {
  return new Response(
    JSON.stringify(String(input) === "/api/health" ? { status: "ok" } : []),
  );
}

it("checks the service and shows its connection state", async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation(async (input) => response(input));
  vi.stubGlobal("fetch", fetchMock);
  render(Home);
  expect(await screen.findByText("OrcaServerに接続しました")).toBeTruthy();
  expect(fetchMock).toHaveBeenCalledWith("/api/health", expect.anything());
  expect(screen.queryByRole("searchbox")).toBeNull();
});

it("keeps a failed connection retryable", async () => {
  vi.stubGlobal(
    "fetch",
    vi
      .fn<typeof fetch>()
      .mockRejectedValueOnce(new Error("offline"))
      .mockImplementation(async (input) => response(input)),
  );
  render(Home);
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "接続できませんでした",
  );
  await fireEvent.click(screen.getByRole("button", { name: "再試行" }));
  expect(await screen.findByText("OrcaServerに接続しました")).toBeTruthy();
});
