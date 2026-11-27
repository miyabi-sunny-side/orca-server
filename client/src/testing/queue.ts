import { server } from "./server";

const machine = "Bambu Lab P1S 0.4 nozzle";

/**
 * One printer printing, eight jobs waiting in assorted states, and the queue
 * API answering as the server does: a stale generation is refused, a move moves.
 */
export function queueFixture() {
  const jobs = Array.from({ length: 9 }, (_, i) => ({
    id: `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
    plate_id: `10000000-0000-4000-8000-${String(i).padStart(12, "0")}`,
    name:
      i === 0
        ? "現在の印刷"
        : `${i}. ${i === 1 ? "長い名前のプレート・前側ケースと追加の取り付け部品" : "取り付け部品 " + i}`,
    state: i === 0 ? "printing" : "queued",
    filament_id: "petg",
    required_machine_profile_key: machine,
    process_profile_key: "0.20mm Standard",
    bed_type: "Cool Plate",
    ams_slot_id: "slot",
    attempt_id: null,
    artifact_path: null,
    last_error: null as string | null,
    hold_reason:
      i === 3 ? "No confirmed AMS slot contains the plate material" : null,
    estimate: {
      state: i === 2 ? "failed" : i === 4 ? "calculating" : "ready",
      seconds: i === 2 || i === 4 ? null : 4800,
      error:
        i === 2
          ? "Selected build plate temperature is missing or zero for this material"
          : null,
    },
  }));
  const q: any = {
    epoch: "fixture-epoch",
    generation: 0,
    request_id: "20000000-0000-4000-8000-000000000000",
    current: jobs[0],
    waiting: jobs.slice(1),
    allowed: { next: false, retry: false, discard: false },
    printer: {
      connection: "connected",
      synchronized: true,
      ready_to_print: false,
      print: {
        state: "RUNNING",
        percent: 35,
        remaining_minutes: 52,
        error: null,
      },
      ams: null,
    },
  };
  const commands: any[] = [];
  const api = server(({ method, path, body }) => {
    if (path === "/api/printers")
      return {
        json: [{ id: "p1", name: "P1S", machine_profile_key: machine }],
      };
    if (path === "/api/filaments")
      return {
        json: [
          {
            id: "petg",
            name: "PETG-GF 黒",
            material: "PETG-GF",
            vendor: "Fixture",
          },
        ],
      };
    if (path === "/api/printers/p1/ams")
      return { json: { slots: [{ id: "slot", ams_id: 1, slot_index: 0 }] } };
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
