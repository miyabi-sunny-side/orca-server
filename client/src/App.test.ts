import { cleanup, render, screen } from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import App from "./App.svelte";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

it("opens the printer queue home with both tabs and settings", async () => {
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
    3,
  );
  expect(
    screen.getByRole("link", { name: "プレート" }).getAttribute("href"),
  ).toBe("/plates");
  expect(await screen.findByText(/印刷先が登録されていません/)).toBeTruthy();
});
