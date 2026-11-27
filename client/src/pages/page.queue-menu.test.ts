import { fireEvent, screen, waitFor, within } from "@testing-library/svelte";
import { afterEach, expect, it } from "vitest";
import { click, press, reset, visit, went } from "../testing/page";
import { gate } from "../testing/server";
import { queueFixture } from "../testing/queue";

afterEach(reset);

/** The queue answers adds, removes and admission like the server; `lose` drops one answer. */
function fixture() {
  const f = queueFixture();
  f.q.current.name = "bin · 1モデル10個";
  f.q.waiting[0].name = "セパレータ";
  let reason: string | null = null,
    lose = false,
    serial = 0,
    last = "";
  f.api.route(({ method, path, url, body }) => {
    if (path !== "/api/queue") return;
    const plate = url.searchParams.get("plate_id");
    if (method === "POST") {
      f.commands.push(body);
      if (JSON.stringify(body) !== last) {
        if (body.generation !== f.q.generation)
          return { status: 409, json: { error: "Queue changed; reload" } };
        if (body.action.type === "add") {
          const source = [f.q.current, ...f.q.waiting].find(
            (j: any) => j.plate_id === body.action.plate_id,
          )!;
          f.q.waiting.push({
            ...source,
            id: `copy-${++serial}`,
            state: "queued",
            attempt_id: null,
            artifact_path: null,
            estimate: { state: "pending", seconds: null, error: null },
          });
        } else if (body.action.type === "remove")
          f.q.waiting = f.q.waiting.filter(
            (j: any) => j.id !== body.action.job_id,
          );
        f.q.generation++;
        f.q.request_id = `request-${f.q.generation}`;
        last = JSON.stringify(body);
      }
      if (lose) {
        lose = false;
        return { abort: true };
      }
    }
    return {
      json: {
        ...structuredClone(f.q),
        admission: plate
          ? { plate_version: 7, allowed: !reason, reason }
          : null,
      },
    };
  });
  return {
    ...f,
    hold: (value: string | null) => (reason = value),
    lose: () => (lose = true),
  };
}
const polled = { timeout: 5000 };
const menu = () => screen.getByRole("dialog");
const row = (id: string) =>
  document.querySelector<HTMLElement>(`#job-${id} > summary`)!;
const button = (name: string) =>
  within(menu()).getByRole<HTMLButtonElement>("button", { name });
const duplicate = () => button("キュー複製");
const jobs = (selector: string) => document.querySelectorAll(selector);
async function open(id: string) {
  await waitFor(() => expect(row(id)).toBeTruthy());
  await fireEvent.contextMenu(row(id));
  await waitFor(() => expect(menu()).toBeTruthy());
}
const enabled = (find: () => HTMLButtonElement, options = {}) =>
  waitFor(() => expect(find().disabled).toBe(false), options);
const disabled = (find: () => HTMLButtonElement, options = {}) =>
  waitFor(() => expect(find().disabled).toBe(true), options);
const openDetails = () =>
  [...document.querySelectorAll("details")].filter(
    (d) => d.open && d.querySelector(".job-details"),
  );

it("duplicate current and waiting without expanding rows", async () => {
  const { q, commands } = fixture();
  visit("/");
  await waitFor(() => expect(jobs(".job-summary")).toHaveLength(9));
  await open(q.current.id);
  await enabled(() => button("プレート編集"));
  await enabled(duplicate);
  expect(button("キュー削除").disabled).toBe(true);
  expect(menu().textContent).toContain("印刷中");
  expect(menu().querySelectorAll(".queue-menu > button")).toHaveLength(3);
  await press("Escape");
  await waitFor(() => expect(document.activeElement).toBe(row(q.current.id)));
  for (const id of [q.waiting[0].id, q.current.id]) {
    await open(id);
    await enabled(duplicate);
    duplicate().click();
    duplicate().click();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    await waitFor(() =>
      expect(
        screen
          .getAllByRole("status")
          .some((s) => s.textContent?.includes("キューを複製しました")),
      ).toBe(true),
    );
    await waitFor(() => expect(document.activeElement).toBe(row(id)));
  }
  expect(commands).toHaveLength(2);
  expect(commands[0].action).toEqual({
    type: "add",
    plate_id: q.waiting[0].plate_id,
    plate_version: 7,
  });
  expect(commands[1].action).toEqual({
    type: "add",
    plate_id: q.current.plate_id,
    plate_version: 7,
  });
  await waitFor(() => expect(jobs(".waiting-job")).toHaveLength(10));
  expect(openDetails()).toHaveLength(0);
  expect(went).toEqual([]);
});

it("polling retains identity and reflects removal, deletion and admission", async () => {
  const { q, commands, hold } = fixture();
  visit("/");
  const source = q.waiting[0];
  await open(source.id);
  await enabled(duplicate);
  q.waiting.reverse();
  q.generation++;
  hold("No confirmed AMS slot contains the plate material");
  await waitFor(() => expect(menu().textContent).toContain("AMS"), polled);
  await disabled(duplicate);
  hold("Queue holds at most 100 waiting jobs");
  await waitFor(() => expect(menu().textContent).toContain("100件"), polled);
  source.plate_deleted = true;
  await disabled(() => button("プレート編集"), polled);
  await enabled(() => button("キュー削除"));
  delete source.plate_deleted;
  hold(null);
  await enabled(duplicate, polled);
  await fireEvent.click(duplicate());
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(commands[0].action.plate_id).toBe(source.plate_id);
  await open(source.id);
  q.waiting = q.waiting.filter((j: any) => j.id !== source.id);
  q.generation++;
  await waitFor(
    () => expect(menu().textContent).toContain("キューにありません"),
    polled,
  );
  for (const name of ["プレート編集", "キュー複製", "キュー削除"])
    expect(button(name).disabled).toBe(true);
  expect(commands).toHaveLength(1);
  await press("Escape");
  await waitFor(() =>
    expect(document.activeElement).toBe(row(q.waiting.at(-1)!.id)),
  );
}, 20_000);

it("lost response reuses one command; subsequent explicit duplication adds again", async () => {
  const f = fixture();
  visit("/");
  await open(f.q.current.id);
  await enabled(duplicate);
  f.lose();
  await fireEvent.click(duplicate());
  const retry = await within(menu()).findByRole("button", {
    name: "結果を再確認",
  });
  expect(duplicate().disabled).toBe(true);
  await fireEvent.click(retry);
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(f.commands).toHaveLength(2);
  expect(f.commands[0]).toEqual(f.commands[1]);
  await waitFor(() => expect(jobs(".waiting-job")).toHaveLength(9));
  await open(f.q.current.id);
  await enabled(duplicate);
  await fireEvent.click(duplicate());
  await waitFor(() => expect(jobs(".waiting-job")).toHaveLength(10));
  expect(f.commands[2].request_id).not.toBe(f.commands[0].request_id);
});

it("keyboard delete focuses adjacent row and edit opens the selected plate", async () => {
  const { q, commands } = fixture();
  visit("/");
  const [first, second] = q.waiting;
  await waitFor(() => expect(row(first.id)).toBeTruthy());
  row(first.id).focus();
  await press("F10", row(first.id), { shiftKey: true });
  await click("キュー削除", menu());
  await waitFor(() => expect(row(first.id)).toBeNull());
  await waitFor(() => expect(document.activeElement).toBe(row(second.id)));
  expect(commands[0].action).toEqual({ type: "remove", job_id: first.id });
  await press("ContextMenu");
  await click("プレート編集", menu());
  expect(went).toEqual([`/plates/${second.plate_id}?edit=1`]);
});

it("long press survives finger-up; preparing/removal/attention states cannot be deleted", async () => {
  const { q, commands } = fixture();
  visit("/");
  await waitFor(() => expect(row(q.current.id)).toBeTruthy());
  const touch = { pointerType: "touch", clientX: 140, clientY: 25 };
  await fireEvent.pointerDown(row(q.current.id), touch);
  await waitFor(() => expect(menu()).toBeTruthy());
  await fireEvent.pointerUp(row(q.current.id), touch);
  // The click that ends the long press neither closes the menu nor opens the row.
  await fireEvent.click(row(q.current.id));
  await enabled(duplicate);
  await press("Escape");
  expect(openDetails()).toHaveLength(0);
  for (const state of ["preparing", "awaiting_removal", "needs_attention"]) {
    q.current.state = state;
    q.generation++;
    await waitFor(
      () =>
        expect(
          document.querySelector(`#job-${q.current.id}`)!.className,
        ).toBeTruthy(),
      polled,
    );
    await open(q.current.id);
    await enabled(duplicate);
    await disabled(() => button("キュー削除"), polled);
    await press("Escape");
  }
  expect(commands).toHaveLength(0);
}, 20_000);

it("late admission for the previous menu cannot authorize or disable another job", async () => {
  const { q, commands, api } = fixture();
  let held = false;
  const late = gate();
  const [first, second] = q.waiting;
  api.route(async ({ method, path, url }) => {
    if (
      method !== "GET" ||
      path !== "/api/queue" ||
      url.searchParams.get("plate_id") !== first.plate_id ||
      held
    )
      return;
    held = true;
    await late.held;
    return {
      json: {
        ...structuredClone(q),
        admission: {
          plate_version: 2,
          allowed: false,
          reason: "old admission",
        },
      },
    };
  });
  visit("/");
  await open(first.id);
  await waitFor(() => expect(held).toBe(true));
  await press("Escape");
  await open(second.id);
  expect(duplicate().disabled).toBe(true);
  late.release();
  await enabled(duplicate);
  expect(menu().textContent).not.toContain("old admission");
  await fireEvent.click(duplicate());
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(commands[0].action).toEqual({
    type: "add",
    plate_id: second.plate_id,
    plate_version: 7,
  });
});

it("a selected waiting job that starts while the menu is open cannot be removed", async () => {
  const { q, commands } = fixture();
  visit("/");
  const source = q.waiting[0];
  await open(source.id);
  await enabled(() => button("キュー削除"));
  q.current = { ...source, state: "printing" };
  q.waiting = q.waiting.filter((j: any) => j.id !== source.id);
  q.generation++;
  await disabled(() => button("キュー削除"), polled);
  expect(menu().textContent).toContain("印刷中");
  expect(duplicate().disabled).toBe(false);
  expect(commands).toHaveLength(0);
});
