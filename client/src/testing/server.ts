import { vi } from "vitest";

/** One request the page sent. `body` is parsed JSON when it was JSON. */
export type Sent = {
  method: string;
  path: string;
  url: URL;
  body: any;
};
/** `abort` makes the request fail as if the server could not be reached. */
export type Reply = { status?: number; json?: unknown; abort?: boolean };
/** Answers a request, or returns undefined to leave it to the routes added before. */
export type Route = (
  request: Sent,
) => Reply | undefined | Promise<Reply | undefined>;

/**
 * The server, faked in place of `fetch`. The newest route answers first and may
 * leave a request to the older ones; a request no route answers gets an empty list.
 */
export function server(...routes: Route[]) {
  const sent: Sent[] = [];
  const stack = [...routes];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(
        input instanceof Request ? input.url : String(input),
        location.origin,
      );
      const method = (
        init?.method ?? (input instanceof Request ? input.method : "GET")
      ).toUpperCase();
      const request: Sent = {
        method,
        path: url.pathname,
        url,
        body: parse(init?.body),
      };
      sent.push(request);
      for (const route of [...stack].reverse()) {
        const reply = await route(request);
        if (reply === undefined) continue;
        if (reply.abort) throw new TypeError("Failed to fetch");
        const status = reply.status ?? 200;
        return new Response(
          status === 204 ? null : JSON.stringify(reply.json ?? null),
          { status, headers: { "content-type": "application/json" } },
        );
      }
      return new Response("[]", {
        headers: { "content-type": "application/json" },
      });
    }),
  );
  return {
    sent,
    /** Adds a route that answers before the earlier ones. */
    route(route: Route) {
      stack.push(route);
      return () => stack.splice(stack.indexOf(route), 1);
    },
    /** The requests sent with this method to this path. */
    to(method: string, path: string | RegExp) {
      return sent.filter(
        (r) =>
          r.method === method &&
          (typeof path === "string" ? r.path === path : path.test(r.path)),
      );
    },
  };
}

function parse(body: BodyInit | null | undefined) {
  if (typeof body !== "string") return body;
  try {
    return JSON.parse(body);
  } catch {
    return body;
  }
}

/** A promise that stays pending until `release` is called. */
export function gate() {
  let release!: () => void;
  const held = new Promise<void>((resolve) => (release = resolve));
  return { held, release };
}
