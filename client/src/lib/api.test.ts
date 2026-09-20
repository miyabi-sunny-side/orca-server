import { afterEach, expect, it, vi } from "vitest";
import { request } from "./api";

afterEach(() => vi.unstubAllGlobals());

it("passes typed responses and request options through the API", async () => {
  const fetchMock = vi
    .fn()
    .mockResolvedValue(new Response(JSON.stringify({ id: "saved" })));
  vi.stubGlobal("fetch", fetchMock);
  expect(
    await request("/api/plates/import", { method: "POST", body: "{}" }),
  ).toEqual({ id: "saved" });
  expect(fetchMock).toHaveBeenCalledWith("/api/plates/import", {
    method: "POST",
    body: "{}",
  });
});

it("explains recoverable failures without treating HTTP errors as success", async () => {
  vi.stubGlobal(
    "fetch",
    vi
      .fn()
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({ error: "Models must fit together on one plate" }),
          { status: 400 },
        ),
      )
      .mockResolvedValueOnce(new Response("busy", { status: 409 }))
      .mockResolvedValueOnce(new Response("disabled", { status: 503 }))
      .mockResolvedValueOnce(new Response("timeout", { status: 504 }))
      .mockRejectedValueOnce(new TypeError("Failed to fetch")),
  );
  await expect(request("/api/plates/id/slice")).rejects.toThrow(
    "1枚のプレートに収まりません",
  );
  await expect(request("/api/plates/id/slice")).rejects.toThrow("再試行");
  await expect(request("/api/scad/models")).rejects.toThrow("モデルの取得先");
  await expect(request("/api/plates/id/slice")).rejects.toThrow("時間の上限");
  await expect(request("/api/plates")).rejects.toThrow("接続できません");
});
