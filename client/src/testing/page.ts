import {
  cleanup,
  fireEvent,
  getByRole,
  render,
  waitFor,
} from "@testing-library/svelte";
import { expect, vi } from "vitest";
import App from "../App.svelte";

/** Where the page asked the browser to go with `location.assign` or `replace`. */
export const went: string[] = [];

/**
 * Opens the whole page at `path`, as a browser would load it. Navigation the
 * page starts is recorded in `went`; the test opens the next page itself.
 */
export function visit(path: string) {
  cleanup();
  history.replaceState(null, "", path);
  const real = window.location;
  const fake = {
    assign: (url: string | URL) => went.push(String(url)),
    replace: (url: string | URL) => went.push(String(url)),
    reload: () => went.push(real.pathname),
    toString: () => real.href,
  };
  for (const key of [
    "href",
    "origin",
    "protocol",
    "host",
    "hostname",
    "port",
    "pathname",
    "search",
    "hash",
  ] as const)
    Object.defineProperty(fake, key, { get: () => real[key] });
  vi.stubGlobal("location", fake);
  return render(App);
}

/** Opens the current address again, like the browser's reload. */
export function reload() {
  return visit(window.location.pathname + window.location.search);
}

/** Presses a key on the element, or on the focused element. */
export async function press(
  key: string,
  target: Element | null = document.activeElement,
  init: KeyboardEventInit = {},
) {
  await fireEvent.keyDown(target ?? document.body, { key, ...init });
}

export function reset() {
  cleanup();
  went.length = 0;
  vi.unstubAllGlobals();
  localStorage.clear();
}

/** Waits until the button is there and enabled, as a person would, and presses it. */
export async function click(
  name: string | RegExp,
  within: HTMLElement = document.body,
) {
  const button = await waitFor(() => {
    const found = getByRole<HTMLButtonElement>(within, "button", { name });
    expect(found.disabled).toBe(false);
    return found;
  });
  await fireEvent.click(button);
  return button;
}
