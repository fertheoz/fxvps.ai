import { createHttpApi } from "./http";
import { createMockApi } from "./mock";
import type { AdminApi } from "./types";

export * from "./types";

let instance: AdminApi | null = null;

/** NEXT_PUBLIC_API_URL set → HTTP adapter; otherwise the seeded in-browser mock. */
export function getApi(): AdminApi {
  if (!instance) {
    const url = process.env.NEXT_PUBLIC_API_URL;
    instance = url ? createHttpApi(url, async () => null) : createMockApi();
  }
  return instance;
}
