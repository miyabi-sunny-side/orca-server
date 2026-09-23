import { test, expect } from "@playwright/test";
import { writeFileSync } from "node:fs";
test("queue copies preserve the plate and running attempt, survive source removal and never auto-start", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  const context = JSON.parse(process.env.E2E_QUEUE_MENU_CONTEXT!);
  const path = "/api/queue?printer_id=p1";
  const state = async () => (await request.get(path)).json();
  const peer = async () =>
    (await request.get(process.env.E2E_PRINTER_CONTROL!)).json();
  const report = async (name: string) => {
    expect(
      (
        await request.post(process.env.E2E_PRINTER_CONTROL!, {
          data: { state: name },
        })
      ).ok(),
    ).toBe(true);
  };
  const original = (await state()).waiting[0];
  const open = async (id: string) => {
    await page.locator(`#job-${id} > summary`).click({ button: "right" });
    await expect(page.getByRole("dialog")).toBeVisible();
  };
  const copy = async (id: string) => {
    const before = await state();
    await open(id);
    const button = page
      .getByRole("dialog")
      .getByRole("button", { name: "キュー複製" });
    await expect(button).toBeEnabled();
    await button.click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    const after = await state();
    expect(after.waiting).toHaveLength(before.waiting.length + 1);
    const added = after.waiting.at(-1);
    expect(before.waiting.map((j: any) => j.id)).toEqual(
      after.waiting.slice(0, -1).map((j: any) => j.id),
    );
    expect(added.plate_id).toBe(context.plate.id);
    expect(added.state).toBe("queued");
    expect(added.attempt_id).toBeNull();
    expect(added.artifact_path).toBeNull();
    return added;
  };
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto("/");
  const queuedCopy = await copy(original.id);
  expect((await peer()).prints).toHaveLength(0);
  await expect
    .poll(
      async () =>
        (await state()).waiting.find((j: any) => j.id === queuedCopy.id)
          .estimate.state,
      { timeout: 60_000 },
    )
    .toBe("ready");
  expect((await request.get(`/api/plates/${context.plate.id}`)).ok()).toBe(
    true,
  );
  const saved = await (
    await request.get(`/api/plates/${context.plate.id}`)
  ).json();
  expect(saved.models[0].quantity).toBe(10);
  await page.getByRole("button", { name: "空のプレートで印刷を開始" }).click();
  await expect
    .poll(async () => (await peer()).prints.length, { timeout: 60_000 })
    .toBe(1);
  await report("RUNNING");
  await expect.poll(async () => (await state()).current.state).toBe("printing");
  const frozen = (await state()).current;
  // Commit the copy on the real server, then lose only its response.
  let lose = true;
  const sent: any[] = [];
  await page.route("**/api/queue?*", async (route) => {
    if (route.request().method() === "POST") {
      sent.push(route.request().postDataJSON());
      if (lose) {
        lose = false;
        const response = await route.fetch();
        expect(response.ok()).toBe(true);
        return route.abort("failed");
      }
    }
    await route.continue();
  });
  await open(original.id);
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "キュー複製" })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "同じ要求を再確認" })
    .click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(sent).toHaveLength(2);
  expect(sent[0]).toEqual(sent[1]);
  let q = await state();
  expect(q.waiting).toHaveLength(2);
  const runningCopy = q.waiting.at(-1);
  expect(q.current.id).toBe(frozen.id);
  expect(q.current.attempt_id).toBe(frozen.attempt_id);
  expect(q.current.artifact_path).toBe(frozen.artifact_path);
  expect(q.current.state).toBe("printing");
  await report("FINISH");
  await expect
    .poll(async () => (await state()).current.state)
    .toBe("awaiting_removal");
  const completedCopy = await copy(original.id);
  expect((await peer()).prints).toHaveLength(1);
  q = await state();
  const discarded = await request.post(path, {
    data: {
      epoch: q.epoch,
      generation: q.generation,
      request_id: q.request_id,
      action: { type: "discard", expected_job: original.id, cleared: true },
    },
  });
  expect(discarded.ok()).toBe(true);
  await expect(page.locator(".current-job")).toHaveCount(0);
  q = await state();
  expect(q.waiting.map((j: any) => j.id)).toEqual([
    queuedCopy.id,
    runningCopy.id,
    completedCopy.id,
  ]);
  await open(queuedCopy.id);
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "キュー削除" })
    .click();
  await expect(page.locator(`#job-${queuedCopy.id}`)).toHaveCount(0);
  await expect
    .poll(
      async () =>
        (await state()).waiting.every((j: any) => j.estimate.state === "ready"),
      { timeout: 60_000 },
    )
    .toBe(true);
  expect((await state()).waiting.map((j: any) => j.id)).toEqual([
    runningCopy.id,
    completedCopy.id,
  ]);
  expect(
    await (await request.get(`/api/plates/${context.plate.id}`)).json(),
  ).toEqual(saved);
  expect((await peer()).prints).toHaveLength(1);
  await page.screenshot({
    path: `${process.env.E2E_EVIDENCE_DIR}/copies-after-original-removal.png`,
  });
  writeFileSync(
    `${process.env.E2E_EVIDENCE_DIR}/copy-result.json`,
    JSON.stringify(
      {
        original,
        queuedCopy,
        runningCopy,
        completedCopy,
        remaining: await state(),
        prints: (await peer()).prints.length,
      },
      null,
      2,
    ),
  );
});
