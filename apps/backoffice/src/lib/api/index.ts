import { createHttpApi } from "./http";
import { createMockApi } from "./mock";
import type { AdminApi } from "./types";
import { apiUrl, getToken, setToken } from "../auth";

export * from "./types";
export { ApiError } from "./http";

let instance: AdminApi | null = null;

/** NEXT_PUBLIC_API_URL set → HTTP adapter (live core-engine); otherwise the seeded in-browser mock. */
export function getApi(): AdminApi {
  if (!instance) {
    const url = apiUrl();
    instance = url ? createHttpApi(url, getToken, { onUnauthorized: () => setToken(null) }) : createMockApi();
  }
  return instance;
}
