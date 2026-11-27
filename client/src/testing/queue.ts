import fixture from "./queue.json";
import { server } from "./server";

/**
 * One printer printing, eight jobs waiting in assorted states (queue.json, which
 * the page tests in the browser show as well), and the queue API answering as
 * the server does: a stale generation is refused, a move moves.
 */
export function queueFixture() {
  const q: any = structuredClone(fixture.queue);
  const commands: any[] = [];
  const api = server(({ method, path, body }) => {
    if (path === "/api/printers") return { json: fixture.printers };
    if (path === "/api/filaments") return { json: fixture.filaments };
    if (path === "/api/printers/p1/ams") return { json: fixture.ams };
    if (path === "/api/queue") {
      if (method === "POST") {
        commands.push(body);
        if (body.generation !== q.generation)
          return { status: 409, json: { error: "Queue changed" } };
        if (body.action.type === "move") {
          const at = q.waiting.findIndex(
            (j: any) => j.id === body.action.job_id,
          );
          const [job] = q.waiting.splice(at, 1);
          q.waiting.splice(body.action.index, 0, job);
        }
        q.generation++;
      }
      return { json: structuredClone(q) };
    }
  });
  return { q, commands, api };
}
