import type { AdminApi, Actor } from "./types";

/**
 * HTTP adapter stub for the future Rust `backoffice-api` (axum). Endpoints are
 * documented in API.md. Auth: OIDC bearer token; the actor is derived
 * server-side from the token, the `actor` argument is only sent as a hint
 * header for local development.
 */
export function createHttpApi(baseUrl: string, getToken: () => Promise<string | null>): AdminApi {
  const call = async <T,>(method: string, path: string, body?: unknown, actor?: Actor, headers: Record<string, string> = {}): Promise<T> => {
    const token = await getToken();
    const res = await fetch(`${baseUrl}${path}`, {
      method,
      headers: {
        "content-type": "application/json",
        ...(token ? { authorization: `Bearer ${token}` } : {}),
        ...(actor ? { "x-actor-hint": actor.name } : {}),
        ...headers,
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!res.ok) throw new Error(`${method} ${path} → ${res.status}`);
    return (await res.json()) as T;
  };
  const enc = encodeURIComponent;

  return {
    dashboard: () => call("GET", "/v1/dashboard"),
    exposure: () => call("GET", "/v1/exposure"),
    listClients: (q) => call("GET", `/v1/accounts${q?.search ? `?search=${enc(q.search)}` : ""}`),
    getClient: (id) => call("GET", `/v1/accounts/${enc(id)}`),
    balanceOp: (req, actor) =>
      call("POST", `/v1/accounts/${enc(req.clientId)}/balance-ops`, req, actor, { "idempotency-key": req.idempotencyKey }),
    setKyc: (id, kyc, actor) => call("PATCH", `/v1/accounts/${enc(id)}/kyc`, { kyc }, actor),
    listGroups: () => call("GET", "/v1/groups"),
    saveGroup: (g, actor) => call("PUT", `/v1/groups/${enc(g.id)}`, g, actor),
    listSymbols: () => call("GET", "/v1/symbols"),
    saveSymbol: (s, actor) => call("PUT", `/v1/symbols/${enc(s.name)}`, s, actor),
    listPositions: () => call("GET", "/v1/positions"),
    listOrders: () => call("GET", "/v1/orders"),
    forceClose: (ids, actor) => call("POST", "/v1/positions/force-close", { positionIds: ids }, actor),
    marginCalls: () => call("GET", "/v1/risk/margin-calls"),
    esmaPresets: () => call("GET", "/v1/risk/presets"),
    applyPreset: (presetId, groupId, actor) => call("POST", `/v1/groups/${enc(groupId)}/apply-preset`, { presetId }, actor),
    listFixSessions: () => call("GET", "/v1/lp/sessions"),
    reconnect: (id, actor) => call("POST", `/v1/lp/sessions/${enc(id)}/reconnect`, {}, actor),
    listTrades: () => call("GET", "/v1/reports/trades"),
    statements: () => call("GET", "/v1/reports/statements"),
    listAudit: () => call("GET", "/v1/audit"),
    listUsers: () => call("GET", "/v1/admin-users"),
    saveUser: (u, actor) => call("PUT", `/v1/admin-users/${enc(u.id)}`, u, actor),
    getSettings: () => call("GET", "/v1/settings"),
    saveSettings: (s, actor) => call("PUT", "/v1/settings", s, actor),
  };
}
