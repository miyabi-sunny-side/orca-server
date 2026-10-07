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

const plateRow = (name: RegExp) => screen.findByRole("link", { name });

it("archives from the row menu without confirmation and reports it", async () => {
  const calls: [string, string][] = [];
  let fail = true;
  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>().mockImplementation(async (input, init) => {
      const method = init?.method ?? "GET";
      calls.push([method, String(input)]);
      if (method === "PUT") {
        if (fail) {
          fail = false;
          return new Response(JSON.stringify({ error: "x" }), { status: 503 });
        }
        return new Response(null, { status: 204 });
      }
      return new Response(
        JSON.stringify(String(input).includes("/api/plates?") ? [saved] : []),
      );
    }),
  );
  render(Home);
  await fireEvent.contextMenu(await plateRow(/机の小物入れ/));
  const archive = await screen.findByRole("button", { name: "アーカイブ" });
  await fireEvent.click(archive);
  // A failure stays in the menu and can be retried.
  expect(await screen.findByRole("alert")).toBeTruthy();
  await fireEvent.click(screen.getByRole("button", { name: "アーカイブ" }));
  expect(await screen.findByRole("status")).toHaveProperty(
    "textContent",
    "「机の小物入れ」をアーカイブしました",
  );
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.queryByRole("link", { name: /机の小物入れ/ })).toBeNull();
  expect(calls.filter(([m]) => m === "PUT")).toEqual([
    ["PUT", "/api/plates/saved/archive"],
    ["PUT", "/api/plates/saved/archive"],
  ]);
  const link = screen.getByRole("link", { name: "アーカイブ済み" });
  expect(link.getAttribute("title")).toBe("アーカイブ済み");
  expect(link.getAttribute("href")).toBe("/plates?archived=1");
});

it("lists archived plates separately and restores them", async () => {
  vi.stubGlobal("location", new URL("http://localhost/plates?archived=1"));
  const calls: [string, string][] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn<typeof fetch>().mockImplementation(async (input, init) => {
      const method = init?.method ?? "GET";
      calls.push([method, String(input)]);
      if (method === "DELETE") return new Response(null, { status: 204 });
      return new Response(
        JSON.stringify(String(input).includes("missing") ? [] : [saved]),
      );
    }),
  );
  render(Home);
  await fireEvent.contextMenu(await plateRow(/机の小物入れ/));
  expect(calls[0]).toEqual(["GET", "/api/plates?q=&archived=true"]);
  expect(screen.queryByRole("button", { name: "キュー追加" })).toBeNull();
  expect(screen.getByRole("button", { name: "削除" })).toBeTruthy();
  await fireEvent.click(screen.getByRole("button", { name: "復元" }));
  expect(await screen.findByRole("status")).toHaveProperty(
    "textContent",
    "「机の小物入れ」を復元しました",
  );
  expect(calls.at(-1)).toEqual(["DELETE", "/api/plates/saved/archive"]);
  expect(
    await screen.findByText("アーカイブ済みのプレートはありません"),
  ).toBeTruthy();
  expect(
    screen
      .getByRole("link", { name: "プレート一覧へ戻る" })
      .getAttribute("href"),
  ).toBe("/plates");
  expect(screen.queryByRole("link", { name: "新規作成" })).toBeNull();
  await fireEvent.input(screen.getByRole("searchbox"), {
    target: { value: "missing" },
  });
  expect(await screen.findByText("一致するプレートがありません")).toBeTruthy();
});
