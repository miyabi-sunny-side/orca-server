// What every document of the page tests (tests/page.rs) runs first: the stand-in for the API of
// the server, so that a test can show the page in states only a printer could bring about.
// A test lays its answers in sessionStorage under `test:api` (JSON), keyed by "METHOD /path?query",
// "METHOD /path", "/path?query" or "/path", the first that is there; an answer is
// `{status, json}`, `{bytes: [...]}` for bytes, `{abort: true}` for a server that cannot be reached,
// or `{hold: "name"}` for one that answers once the test adds the name to `test:released`.
// Every request is written down in `test:sent`; one the page gave up on, in `test:aborted`;
// an error the page did not catch, in `test:errors`.
// A request for no key is answered with an empty list. Without `test:api` the server answers.
{
  const fetchOfPage = window.fetch.bind(window);
  const read = (key, empty) => {
    try {
      return JSON.parse(sessionStorage.getItem(key)) ?? empty;
    } catch {
      return empty;
    }
  };
  const note = (key, entry) =>
    sessionStorage.setItem(key, JSON.stringify([...read(key, []), entry]));
  addEventListener("error", (event) => note("test:errors", event.message));
  addEventListener("unhandledrejection", (event) =>
    note("test:errors", String(event.reason)),
  );
  window.fetch = async (input, init = {}) => {
    const routes = read("test:api", null);
    const asked = new URL(
      input instanceof Request ? input.url : String(input),
      location.href,
    );
    if (routes === null || !asked.pathname.startsWith("/api/"))
      return fetchOfPage(input, init);
    const method = (init.method ?? "GET").toUpperCase();
    const where = asked.pathname + asked.search;
    let body = typeof init.body === "string" ? init.body : null;
    try {
      body = JSON.parse(body);
    } catch {}
    note("test:sent", { method, path: where, body });
    const key = [
      `${method} ${where}`,
      `${method} ${asked.pathname}`,
      where,
      asked.pathname,
    ].find((name) => name in routes);
    const answer = key === undefined ? { json: [] } : routes[key];
    if (answer.hold)
      await new Promise((resolve, reject) => {
        const signal = init.signal;
        const look = setInterval(() => {
          if (signal?.aborted) {
            clearInterval(look);
            note("test:aborted", where);
            reject(new DOMException("The request was aborted", "AbortError"));
          } else if (read("test:released", []).includes(answer.hold)) {
            clearInterval(look);
            resolve();
          }
        }, 10);
      });
    if (answer.abort) throw new TypeError("Failed to fetch");
    const status = answer.status ?? 200;
    if (answer.bytes !== undefined)
      return new Response(new Uint8Array(answer.bytes), {
        status,
        headers: { "Content-Type": "application/octet-stream" },
      });
    return new Response(
      status === 204 || status === 202 ? null : JSON.stringify(answer.json ?? null),
      { status, headers: { "Content-Type": "application/json" } },
    );
  };
}
