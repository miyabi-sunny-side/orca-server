import { cleanup, render, screen } from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import App from "./App.svelte";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

it("keeps the product header and theme controls on the service page", async () => {
  vi.stubGlobal(
    "fetch",
    vi
      .fn<typeof fetch>()
      .mockImplementation(
        async (input) =>
          new Response(
            JSON.stringify(
              String(input) === "/api/health" ? { status: "ok" } : [],
            ),
          ),
      ),
  );
  render(App);
  expect(
    screen.getByRole("link", { name: "OrcaServer" }).getAttribute("href"),
  ).toBe("/");
  expect(screen.getByRole("button", { name: "メニュー" })).toBeTruthy();
  expect(screen.getByRole("banner").querySelectorAll("a, button")).toHaveLength(
    2,
  );
  expect(await screen.findByText("OrcaServerに接続しました")).toBeTruthy();
});
