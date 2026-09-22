import { afterEach, expect, it, vi } from "vitest";
import { request } from "./api";

afterEach(() => vi.unstubAllGlobals());

it("duplication failures identify the name or storage without blaming the slicer", async () => {
  for (const [status, message] of [
    [400, "プレート名"],
    [404, "複製元"],
    [503, "保存先"],
  ] as const) {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(new Response("{}", { status })),
    );
    await expect(
      request("/api/plates/id/duplicate", { method: "POST" }),
    ).rejects.toThrow(message);
  }
});

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

it("keeps queue conflicts distinguishable from an unknown network result", async () => {
  vi.stubGlobal(
    "fetch",
    vi
      .fn()
      .mockResolvedValueOnce(
        new Response(JSON.stringify({ error: "Queue changed" }), {
          status: 409,
        }),
      )
      .mockResolvedValueOnce(new Response("unavailable", { status: 503 })),
  );
  await expect(request("/api/queue", { method: "POST" })).rejects.toMatchObject(
    { status: 409, message: expect.stringContaining("キュー") },
  );
  await expect(request("/api/queue")).rejects.toMatchObject({
    status: 503,
    message: expect.stringContaining("キュー"),
  });
});

it("explains material references and stale AMS mappings in the correct context", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(new Response("conflict", { status: 409 })),
  );
  await expect(
    request("/api/filaments/a", { method: "DELETE" }),
  ).rejects.toThrow("参照");
  await expect(
    request("/api/printers/p/ams/s", { method: "PUT" }),
  ).rejects.toThrow("AMS");
});

it("default selection failures identify the missing printer", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(new Response("missing", { status: 404 })),
  );
  await expect(
    request("/api/default-settings", { method: "PUT" }),
  ).rejects.toThrow("プリンターが見つかりません");
});

it("explains how to recover when a selected process has no brim width", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      new Response(
        JSON.stringify({
          error:
            "Selected process has no valid brim width; choose another process or disable brim",
        }),
        { status: 400 },
      ),
    ),
  );
  await expect(request("/api/plates/id", { method: "PUT" })).rejects.toThrow(
    "別の工程を選ぶか「ブリムを付ける」をOFF",
  );
});

it("plate material search failures describe material storage, not slicing", async () => {
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(new Response("unavailable", { status: 503 })),
  );
  await expect(request("/api/plate-filaments?q=PLA")).rejects.toMatchObject({
    status: 503,
    message: expect.stringContaining("材料の保存先"),
  });
});
