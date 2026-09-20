export async function checkHealth(signal?: AbortSignal): Promise<void> {
  const response = await fetch("/api/health", { signal });
  if (!response.ok || (await response.json()).status !== "ok") {
    throw new Error("Service unavailable");
  }
}
