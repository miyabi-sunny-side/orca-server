import { cleanup, fireEvent, render, screen } from "@testing-library/svelte";
import { afterEach, expect, it, vi } from "vitest";
import Home from "./Home.svelte";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

const saved = {
  id: "saved",
  name: "机の小物入れ",
  models: [{ name: "box.stl" }],
  project: "project.3mf",
  print: "print.gcode.3mf",
};

it("opens saved plates and filters using server-ranked search results", async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation(
      async (input) =>
        new Response(
          JSON.stringify(String(input).includes("missing") ? [] : [saved]),
        ),
    );
  vi.stubGlobal("fetch", fetchMock);
  render(Home);
  expect(
    await screen.findByRole("link", { name: /机の小物入れ/ }),
  ).toHaveProperty("href", expect.stringContaining("/plates/saved"));
  expect(screen.getByRole("link", { name: "新規作成" })).toBeTruthy();
  await fireEvent.input(screen.getByRole("searchbox"), {
    target: { value: "missing" },
  });
  expect(await screen.findByText("一致するプレートがありません")).toBeTruthy();
  expect(
    fetchMock.mock.calls.some(([url]) => String(url).includes("q=missing")),
  ).toBe(true);
});

it("keeps a failed list retryable and distinguishes an empty collection", async () => {
  vi.stubGlobal(
    "fetch",
    vi
      .fn()
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValue(new Response("[]")),
  );
  render(Home);
  expect(await screen.findByRole("alert")).toBeTruthy();
  await fireEvent.click(screen.getByRole("button", { name: "再試行" }));
  expect(await screen.findByText("保存済みプレートはありません")).toBeTruthy();
});
