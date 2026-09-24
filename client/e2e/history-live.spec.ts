import { test, expect } from "@playwright/test";
import { writeFileSync } from "node:fs";

test("history re-add uses current inputs, repairs admission and retries one request without starting", async ({
  page,
  request,
}) => {
  test.setTimeout(180_000);
  const context = JSON.parse(process.env.E2E_HISTORY_CONTEXT!);
  const path = `/api/plates/${context.plate.id}`;
  const queue = async () =>
    (await request.get("/api/queue?printer_id=p1")).json();
  const history = async () => (await request.get("/api/history")).json();
  const edit = async (material: string | null) => {
    const body = await (await request.get(path)).json();
    delete body.id;
    delete body.imported;
    delete body.roles;
    for (const model of body.models) delete model.roles;
    body.conditions.filament_id = material;
    expect((await request.put(path, { data: body })).ok()).toBe(true);
  };
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto("/history");
  const row = page.locator(".history-row").first(),
    dialog = page.getByRole("dialog");
  const add = dialog.getByRole("button", { name: "キューに追加", exact: true });
  await expect(row).toContainText(context.history.items[0].name);
  const before = await queue();
  await edit(null);
  await row.click({ button: "right" });
  await expect(add).toBeDisabled();
  await expect(
    dialog.getByRole("link", { name: "プレート編集" }),
  ).toHaveAttribute("href", `/plates/${context.plate.id}?edit=1`);
  await page.keyboard.press("Escape");
  await edit(context.plate.conditions.filament_id);
  await row.press("Shift+F10");
  await expect(add).toBeEnabled();
  let lose = true;
  const commands: unknown[] = [];
  await page.route("**/api/queue?*", async (route) => {
    if (route.request().method() === "POST") {
      commands.push(route.request().postDataJSON());
      if (lose) {
        lose = false;
        const response = await route.fetch();
        expect(response.ok()).toBe(true);
        return route.abort("failed");
      }
    }
    await route.continue();
  });
  await add.evaluate((button: HTMLButtonElement) => {
    button.click();
    button.click();
  });
  const retry = dialog.getByRole("button", { name: "同じ要求を再確認" });
  await expect(retry).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();
  // A reload preserves the original request ID and original printer.
  await page.reload();
  await row.click({ button: "right" });
  await expect(retry).toBeVisible();
  await retry.click();
  await expect(dialog).toHaveCount(0);
  await expect(row).toBeFocused();
  expect(commands).toHaveLength(2);
  expect(commands[0]).toEqual(commands[1]);
  const once = await queue();
  expect(once.waiting).toHaveLength(before.waiting.length + 1);
  expect(once.waiting[0].id).toBe(before.waiting[0].id);
  await row.click({ button: "right" });
  await expect(add).toBeEnabled();
  await add.click();
  await expect(dialog).toHaveCount(0);
  const twice = await queue();
  expect(twice.waiting).toHaveLength(before.waiting.length + 2);
  expect(commands[2]).not.toEqual(commands[0]);
  expect(await history()).toEqual(context.history);
  expect(twice.current).toBeNull();
  for (const job of twice.waiting.slice(-2)) {
    expect(job.plate_id).toBe(context.plate.id);
    expect(job.state).toBe("queued");
    expect(job.attempt_id).toBeNull();
    expect(job.artifact_path).toBeNull();
  }
  const peer = await (
    await request.get(process.env.E2E_PRINTER_CONTROL!)
  ).json();
  expect(peer.prints).toHaveLength(1);
  expect(peer.uploads).toHaveLength(1);
  await page.getByRole("link", { name: "キューを見る" }).click();
  await expect(page).toHaveURL(/\/queue\?printer_id=p1$/);
  await page.goBack();
  await expect(row).toContainText(context.history.items[0].name);
  await page.screenshot({
    path: `${process.env.E2E_EVIDENCE_DIR}/history-live.png`,
  });
  writeFileSync(
    `${process.env.E2E_EVIDENCE_DIR}/history-result.json`,
    JSON.stringify(
      { before, once, twice, history: await history(), commands, peer },
      null,
      2,
    ),
  );
});
