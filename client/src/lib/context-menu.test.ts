import { afterEach, expect, it, vi } from "vitest";
import { contextMenu } from "./context-menu";

afterEach(() => vi.useRealTimers());
it("shares keyboard/right click and long press without stealing taps or scrolling", () => {
  vi.useFakeTimers();
  const row = document.createElement("button");
  const open = vi.fn();
  const action = contextMenu(row, open);
  const send = (name: string, fields = {}) => {
    const event = new Event(name, { cancelable: true });
    Object.assign(event, fields);
    row.dispatchEvent(event);
    return event;
  };
  expect(send("contextmenu").defaultPrevented).toBe(true);
  send("keydown", { key: "ContextMenu" });
  send("keydown", { key: "F10", shiftKey: true });
  expect(open).toHaveBeenCalledTimes(3);
  send("keydown", { key: "F10" });
  send("pointerdown", { pointerType: "mouse" });
  vi.advanceTimersByTime(600);
  expect(send("click").defaultPrevented).toBe(false);
  expect(open).toHaveBeenCalledTimes(3);
  send("pointerdown", { pointerType: "touch", clientX: 20, clientY: 20 });
  vi.advanceTimersByTime(499);
  expect(open).toHaveBeenCalledTimes(3);
  vi.advanceTimersByTime(1);
  expect(open).toHaveBeenLastCalledWith(row);
  expect(open).toHaveBeenCalledTimes(4);
  send("pointerup");
  expect(send("click").defaultPrevented).toBe(true);
  for (const cancel of [
    "pointermove",
    "pointercancel",
    "pointerleave",
    "pointerup",
  ]) {
    send("pointerdown", { pointerType: "pen", clientX: 20, clientY: 20 });
    send(cancel, { clientX: 20, clientY: 50 });
    vi.advanceTimersByTime(600);
  }
  expect(open).toHaveBeenCalledTimes(4);
  send("pointerdown", { pointerType: "touch", clientX: 20, clientY: 20 });
  action.destroy();
  vi.advanceTimersByTime(600);
  send("contextmenu");
  expect(open).toHaveBeenCalledTimes(4);
});
