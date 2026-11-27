import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { click, press, reload, reset, visit, went } from "../testing/page";
import { queueFixture } from "../testing/queue";
import { server } from "../testing/server";

afterEach(reset);

const reason =
  "The printer reported that the storage is full and the uploaded project could not be opened for printing ".repeat(
    3,
  ) + `/sdcard/${"x".repeat(120)}`;
const $ = <T extends Element = HTMLElement>(selector: string) =>
  document.querySelector<T>(selector);
const $$ = (selector: string) => [...document.querySelectorAll(selector)];
const found = <T extends Element = HTMLElement>(selector: string) =>
  waitFor(() => {
    const element = $<T>(selector);
    expect(element, selector).toBeTruthy();
    return element!;
  });
/** Not inside a closed disclosure, where the browser draws nothing. */
const shown = (element: Element) => {
  for (let at = element.parentElement; at; at = at.parentElement)
    if (
      at instanceof HTMLDetailsElement &&
      !at.open &&
      !element.closest("summary")
    )
      return false;
  return !element.closest("[hidden]");
};
/** Long enough for the page's two-second queue polling. */
const polled = { timeout: 5000 };
const retryButton = () =>
  screen.getByRole<HTMLButtonElement>("button", { name: "再印刷" });

it("saved failure code and long reason stay readable with recovery", async () => {
  const { q } = queueFixture();
  visit("/");
  await found(".current-job");
  expect($(".device-failure")).toBeNull();
  Object.assign(q.current, {
    state: "needs_attention",
    last_error: "Printer rejected the start request",
    failure: { kind: "rejected", field: "err_code", code: "0500-4003", reason },
  });
  Object.assign(q.printer.print, { state: "IDLE", error: 0x05004003 });
  Object.assign(q, {
    allowed: { next: false, retry: true, discard: true },
    recovery: { retry_reason: null, discard_reason: null },
  });
  reload();
  const failure = await found(".device-failure");
  const summary = $(".current-job summary")!;
  expect(summary.textContent).toContain("要確認");
  expect(summary.textContent).not.toContain("0500-4003");
  expect(summary.parentElement).toHaveProperty("open", false);
  expect(failure.textContent).toContain("現在");
  expect(failure.querySelector("strong")!.textContent).toBe("0500-4003");
  expect(failure.textContent).not.toContain(reason.trim());
  q.printer.print.error = 0;
  reload();
  await waitFor(() =>
    expect($(".device-failure")!.textContent).toContain("前回"),
  );
  expect($(".device-failure")!.textContent).toContain("0500-4003");
  expect($(".device-failure")!.textContent).not.toContain(reason.trim());
  await fireEvent.click($(".current-job summary")!);
  const details = $(".current-job .job-details")!;
  expect(details.textContent).toContain("コード: err_code 0500-4003");
  expect(details.textContent).toContain(`理由: ${reason.trim()}`);
  expect(retryButton().disabled).toBe(false);
  Object.assign(q.current, {
    failure: { kind: "stopped", state: "FAILED" },
    last_error: "Print stopped; inspect the printer",
  });
  reload();
  await waitFor(() =>
    expect($(".device-failure")!.textContent).toContain("印刷停止"),
  );
  expect($(".current-job summary")!.textContent).toContain("要確認");
});

it("an attempt that never started says why next to the retry", async () => {
  const { q, commands } = queueFixture();
  Object.assign(q.current, {
    state: "needs_attention",
    last_error:
      "Printer status or selected AMS changed during transfer; no start command was sent",
    failure: null,
  });
  Object.assign(q.printer, { ready_to_print: true });
  Object.assign(q.printer.print, { state: "IDLE", error: 0 });
  Object.assign(q, {
    waiting: [],
    allowed: { next: false, retry: true, discard: true },
    recovery: { retry_reason: null, discard_reason: null },
  });
  visit("/");
  const note =
    "転送中に機器またはAMSの状態が変わりました。開始命令は送っていません。";
  expect(await screen.findAllByText(note)).toHaveLength(1);
  expect($(".current-job")).toHaveProperty("open", false);
  expect(shown(screen.getByText(note))).toBe(true);
  expect(retryButton().disabled).toBe(false);
  expect(shown(retryButton())).toBe(true);
  expect($$(".btn.primary").filter(shown)).toHaveLength(1);
  // Opening the job does not say it a second time.
  await fireEvent.click($(".current-job > summary")!);
  expect($(".current-job")).toHaveProperty("open", true);
  expect(screen.getAllByText(note)).toHaveLength(1);
  expect(commands).toHaveLength(0);
});

it("stopped recovery stays visible and takes one keyboard action", async () => {
  const { q, commands } = queueFixture();
  q.current.state = "needs_attention";
  q.printer.print.state = "FAILED";
  q.allowed.retry = true;
  q.allowed.discard = true;
  Object.assign(q, { recovery: { retry_reason: null, discard_reason: null } });
  visit("/");
  await found(".current-job");
  expect($$(".job-details").filter(shown)).toHaveLength(0);
  const retry = await waitFor(() => {
    expect(retryButton().disabled).toBe(false);
    return retryButton();
  });
  expect(shown(retry)).toBe(true);
  expect(screen.queryByRole("checkbox")).toBeNull();
  retry.focus();
  expect(document.activeElement).toBe(retry);
  await fireEvent.click(retry);
  await waitFor(() => expect(commands).toHaveLength(1));
  expect(commands[0].action).toEqual({
    type: "retry",
    expected_job: q.current.id,
    cleared: true,
  });
  q.allowed.retry = false;
  q.allowed.discard = false;
  const changed = "Printer report does not match the recovery target";
  Object.assign(q, {
    recovery: { retry_reason: changed, discard_reason: changed },
  });
  reload();
  await waitFor(() => expect(retryButton().disabled).toBe(true));
  expect(
    screen.getByText(
      "本体のジョブが変わっています。印刷状況を確認してください。",
    ),
  ).toBeTruthy();
  expect(commands).toHaveLength(1);
});

it("paused printer explains the error without expanding the job", async () => {
  const { q, commands } = queueFixture();
  Object.assign(q.current, {
    state: "needs_attention",
    last_error: "Printer reported an error; inspect the printer",
    failure: {
      kind: "device_error",
      field: "print_error",
      code: "0300-8010",
      state: "PAUSE",
    },
  });
  Object.assign(q.printer.print, { state: "PAUSE", error: 0x03008010 });
  Object.assign(q.printer, {
    descriptions: { "0300-8010": "冷却ファンの回転速度は異常があります。" },
  });
  const previous = "Wait for a matching terminal report for the previous start";
  Object.assign(q, {
    waiting: [],
    recovery: { retry_reason: previous, discard_reason: previous },
  });
  visit("/");
  const failure = await found(".device-failure");
  expect($(".current-job")).toHaveProperty("open", false);
  expect(shown(failure)).toBe(true);
  expect(failure.textContent).toContain("0300-8010");
  expect(failure.textContent).toContain(
    "冷却ファンの回転速度は異常があります。",
  );
  expect(failure.textContent).toContain("現在");
  expect(failure.textContent).not.toContain("PAUSE");
  expect(screen.queryByText(/前の開始結果が不明です/)).toBeNull();
  const help = within(failure).getByRole<HTMLAnchorElement>("link", {
    name: /公式/,
  });
  const url = new URL(help.href);
  expect(url.hostname).toBe("e.bambulab.com");
  expect(url.searchParams.get("e")).toBe("03008010");
  // The browser opens it in a new tab; the page stays.
  expect(help.target).toBe("_blank");
  help.focus();
  expect(document.activeElement).toBe(help);
  await fireEvent.click($(".current-job > summary")!);
  expect($(".current-job")!.textContent).toContain("本体の状態: PAUSE");
  reload();
  await waitFor(() =>
    expect($(".device-failure")!.textContent).toContain("0300-8010"),
  );
  expect($(".current-job")).toHaveProperty("open", false);
  expect(retryButton().disabled).toBe(true);
  Object.assign(q.current, {
    state: "printing",
    failure: null,
    last_error: null,
  });
  Object.assign(q.printer.print, { state: "RUNNING", error: 0 });
  await waitFor(() => expect($(".device-failure")).toBeNull(), polled);
  expect(commands).toHaveLength(0);
});

it("unmanaged and unlisted errors remain visible, while stale reports are labelled", async () => {
  const { q, commands } = queueFixture();
  const job = q.current;
  Object.assign(q, { current: null, waiting: [] });
  Object.assign(q.printer.print, { state: "PAUSE", error: 0xffff1234 });
  visit("/");
  const failure = await found(".device-failure");
  expect(failure.querySelector("strong")!.textContent).toBe("FFFF-1234");
  expect(within(failure).getByRole("link", { name: /公式/ })).toBeTruthy();
  Object.assign(job, {
    state: "needs_attention",
    failure: { kind: "stopped", state: "FAILED" },
  });
  Object.assign(q, { current: job });
  Object.assign(q.printer, { connection: "disconnected", synchronized: false });
  await waitFor(
    () => expect($(".device-failure")!.textContent).toContain("前回"),
    polled,
  );
  const stale = $(".device-failure")!;
  expect(stale.textContent).toContain("印刷停止");
  expect(stale.textContent).not.toContain("FFFF-1234");
  expect(within(stale).queryByRole("link")).toBeNull();
  expect(commands).toHaveLength(0);
});

it("two lines, density and stable detail/keyboard state", async () => {
  const { q, commands, api } = queueFixture();
  visit("/");
  await waitFor(() => expect($$(".job-summary")).toHaveLength(9));
  expect(
    screen
      .getByRole("link", { name: "OrcaServer" })
      .getAttribute("aria-current"),
  ).toBe("page");
  expect(
    screen.getByRole("link", { name: "プレート" }).getAttribute("href"),
  ).toBe("/plates");
  expect(screen.queryByRole("button", { name: "前へ" })).toBeNull();
  expect($$(".job-details").filter(shown)).toHaveLength(0);
  for (const row of $$(".waiting-job"))
    expect(row.querySelector(".job-lines")!.children).toHaveLength(2);
  const first = $$(".waiting-job")[0] as HTMLElement;
  await fireEvent.click(first.querySelector("summary")!);
  expect(within(first).getByText(/PETG-GF 黒/)).toBeTruthy();
  const handle = within(first).getByRole("button", { name: /並べ替え/ });
  handle.focus();
  const reads = api.to("GET", "/api/queue").length;
  await waitFor(
    () => expect(api.to("GET", "/api/queue").length).toBeGreaterThan(reads),
    polled,
  );
  await waitFor(() => expect(document.activeElement).toBe(handle));
  expect(first.querySelector("details")).toHaveProperty("open", true);
  const id = q.waiting[0].id;
  await press(" ", handle);
  await press("ArrowDown", handle);
  await press("Enter", handle);
  await waitFor(() => expect(commands).toHaveLength(1));
  expect(commands[0].action).toEqual({ type: "move", job_id: id, index: 1 });
  await waitFor(() =>
    expect($$(".waiting-job")[1].querySelector("details")).toHaveProperty(
      "open",
      true,
    ),
  );
  await waitFor(() =>
    expect(document.activeElement).toBe(
      document.getElementById(`handle-${id}`),
    ),
  );
});

it("home shows each printer and keeps legacy printer filtering", async () => {
  const { q, api } = queueFixture();
  api.route(({ path, url }) => {
    if (path === "/api/printers")
      return {
        json: [
          {
            id: "p1",
            name: "P1S",
            machine_profile_key: "Bambu Lab P1S 0.4 nozzle",
          },
          {
            id: "p2",
            name: "Second P1S",
            machine_profile_key: "Bambu Lab P1S 0.4 nozzle",
          },
        ],
      };
    if (path === "/api/queue" && url.searchParams.get("printer_id") === "p2")
      return { json: { ...structuredClone(q), current: null, waiting: [] } };
    if (path === "/api/printers/p2/ams") return { json: { slots: [] } };
  });
  visit("/");
  await waitFor(() => expect($$(".printer-queue")).toHaveLength(2));
  await fireEvent.change(screen.getByLabelText("表示するプリンター"), {
    target: { value: "p2" },
  });
  await waitFor(() => expect($$(".printer-queue")).toHaveLength(1));
  expect(screen.getByRole("heading", { name: "Second P1S" })).toBeTruthy();
  visit("/queue?printer_id=p1");
  await waitFor(() => expect($$(".job-summary")).toHaveLength(9));
  expect($$(".printer-queue")).toHaveLength(1);
});

it("detail adds directly and preserves an unknown request through reload", async () => {
  const machine = "Bambu Lab P1S 0.4 nozzle",
    mini = "Bambu Lab A1 mini 0.2 nozzle";
  const id = "11111111-1111-4111-8111-111111111111";
  const plate = {
    id,
    version: 1,
    name: "繰り返し印刷するプレート",
    models: [
      { id: "item", name: "part.stl", source: "part.stl", quantity: 10 },
    ],
    conditions: { filament_id: "white" },
  };
  const printers = [{ id: "p1", name: "P1S", machine_profile_key: machine }];
  const requests: any[] = [],
    waiting: any[] = [];
  let reads = 0,
    fail = true;
  server(({ method, path, body }) => {
    if (path === `/api/plates/${id}`) return { json: plate };
    if (path === "/api/printers") return { json: printers };
    if (path === "/api/filaments")
      return {
        json: [
          { id: "white", name: "PLA 白", vendor: "Fixture", material: "PLA" },
        ],
      };
    if (path === "/api/queue") {
      if (method === "POST") {
        requests.push(body);
        if (!waiting.length) waiting.push({ id: "job", plate_id: id });
        if (fail) {
          fail = false;
          return { abort: true };
        }
      }
      reads++;
      return {
        json: {
          epoch: "epoch",
          generation: waiting.length,
          request_id: `request-${reads}`,
          waiting,
          current: null,
          allowed: { next: false },
          admission: { allowed: true, plate_version: 1, reason: null },
        },
      };
    }
  });
  visit(`/plates/${id}`);
  const add = await waitFor(() => {
    const button = screen.getByRole<HTMLButtonElement>("button", {
      name: "印刷キューへ",
    });
    expect(button.disabled).toBe(false);
    return button;
  });
  expect(screen.queryByLabelText("追加先のプリンター")).toBeNull();
  add.click();
  add.click();
  expect(await screen.findByText("追加の結果が不明です")).toBeTruthy();
  expect(requests).toHaveLength(1);
  expect(requests[0].action).toEqual({
    type: "add",
    plate_id: id,
    plate_version: 1,
  });
  reload();
  await waitFor(() =>
    expect(
      screen.getByRole<HTMLButtonElement>("button", { name: "印刷キューへ" })
        .disabled,
    ).toBe(true),
  );
  await click("結果を再確認");
  await waitFor(() =>
    expect(
      screen
        .getAllByRole("status")
        .some((s) => s.textContent?.includes("キューに追加しました")),
    ).toBe(true),
  );
  expect(requests).toHaveLength(2);
  expect(requests[1]).toEqual(requests[0]);
  expect(waiting).toHaveLength(1);
  expect(went).toEqual([]);
  expect(screen.queryByLabelText("使用するAMSスロット")).toBeNull();
  // Every registered printer is a destination; remember only a valid choice.
  printers.push(
    ...[3, 1, 2].map((n) => ({
      id: `a${n}`,
      name: `A1 mini ${n}`,
      machine_profile_key: mini,
    })),
  );
  const destination = () =>
    screen.findByLabelText<HTMLSelectElement>("追加先のプリンター");
  reload();
  await waitFor(async () => expect((await destination()).value).toBe("a1"));
  expect((await destination()).options).toHaveLength(4);
  await fireEvent.change(await destination(), { target: { value: "a2" } });
  reload();
  await waitFor(async () => expect((await destination()).value).toBe("a2"));
  printers.splice(
    printers.findIndex((p) => p.id === "a2"),
    1,
  );
  reload();
  await waitFor(async () => expect((await destination()).value).toBe("a1"));
});

it("after unplugging the AMS the queue prints from the external spool without any feed choice", async () => {
  const { q, commands, api } = queueFixture();
  Object.assign(q.current, {
    state: "needs_attention",
    failure: { kind: "stopped", state: "FAILED" },
  });
  q.waiting[0].feed = "external";
  Object.assign(q.printer.print, { state: "FAILED", error: 0 });
  Object.assign(q, {
    allowed: { next: false, retry: true, discard: true },
    recovery: { retry_reason: null, discard_reason: null },
  });
  visit("/");
  await waitFor(() =>
    expect($(".current-job summary")?.textContent).toContain("要確認"),
  );
  expect(screen.queryByText(/AMS slot|AMSを確認/)).toBeNull();
  expect(screen.queryByLabelText("給材")).toBeNull();
  const waiting = $$(".waiting-job details")[0] as HTMLDetailsElement;
  await fireEvent.click(waiting.querySelector("summary")!);
  expect(within(waiting).getByText(/外部スプール ·/)).toBeTruthy();
  await click("再印刷");
  await waitFor(() => expect(commands).toHaveLength(1));
  expect(commands[0].action.type).toBe("retry");
  api.route(({ path }) => {
    if (path === "/api/printers/p1/ams")
      return { json: { current: true, slots: [] } };
  });
  visit("/printers/p1/ams");
  expect(
    await screen.findByText("AMS未接続 · 外部スプールで印刷します"),
  ).toBeTruthy();
});

it("stopped jobs recover through editing, retry, removal and next", async () => {
  const { q, commands } = queueFixture();
  Object.assign(q.current, {
    state: "needs_attention",
    failure: {
      kind: "device_error",
      field: "print_error",
      code: "0300-4000",
      state: "RUNNING",
      description: "Z軸の原点復帰に失敗しました。タスクは停止されました。",
    },
  });
  Object.assign(q.printer.print, { state: "IDLE", error: 0 });
  Object.assign(q, {
    allowed: { next: false, retry: true, discard: true },
    recovery: { retry_reason: null, discard_reason: null },
  });
  visit("/queue?printer_id=p1");
  const failure = await found(".device-failure");
  expect($(".current-job")).toHaveProperty("open", false);
  expect(failure.textContent).toContain("前回");
  expect(failure.querySelector("strong")!.textContent).toBe(
    "0300-4000 Z軸の原点復帰に失敗しました。タスクは停止されました。",
  );
  await waitFor(() => expect(retryButton().disabled).toBe(false));
  expect(
    screen.getByRole<HTMLButtonElement>("button", { name: "除外" }).disabled,
  ).toBe(false);
  expect(screen.queryByText(/前の開始結果が不明です/)).toBeNull();
  expect($$(".btn.primary").filter(shown)).toHaveLength(1);
  Object.assign(q.current, { failure: { kind: "stopped", state: "FAILED" } });
  reload();
  await waitFor(() =>
    expect($(".device-failure")?.textContent).toContain("印刷停止"),
  );
  expect($(".current-job summary")!.textContent).toContain("要確認");
  await fireEvent.click($(".current-job summary")!);
  expect(
    within($(".current-job")!)
      .getByRole("link", { name: "プレートの条件を編集" })
      .getAttribute("href"),
  ).toBe(`/plates/${q.current.plate_id}?edit=1`);
  await click("除外");
  await waitFor(() => expect(commands).toHaveLength(1));
  expect(commands[0].action).toEqual({
    type: "discard",
    expected_job: q.current.id,
    cleared: true,
  });
  Object.assign(q, {
    current: null,
    allowed: { next: true, retry: false, discard: false },
  });
  q.printer.ready_to_print = false;
  reload();
  await click("印刷");
  await waitFor(() => expect(commands).toHaveLength(2));
  expect(commands[1].action).toMatchObject({
    type: "next",
    expected_job: q.waiting[0].id,
  });
});
