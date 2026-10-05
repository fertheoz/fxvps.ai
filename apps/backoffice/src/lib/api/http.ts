import type { AdminApi, Actor, ApprovalRequest } from "./types";

/** Error returned by the admin API: `{ error: { code, message, permission? } }`. */
export class ApiError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
    public permission?: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

export interface HttpApiOptions {
  /** Called on 401 (missing/expired token), e.g. to show the login screen. */
  onUnauthorized?: () => void;
  fetchImpl?: typeof fetch;
  /** EventSource constructor (injectable for tests). */
  eventSource?: typeof EventSource;
}

/**
 * HTTP adapter for the core-engine admin API (`/v1/*`, see API.md). The actor
 * is derived server-side from the bearer token; the `actor` argument is only
 * sent as an `x-actor-hint` header for logs.
 */
export function createHttpApi(baseUrl: string, getToken: () => string | null | Promise<string | null>, opts: HttpApiOptions = {}): AdminApi {
  const base = baseUrl.replace(/\/+$/, "");
  const doFetch = opts.fetchImpl ?? ((...a: Parameters<typeof fetch>) => fetch(...a));

  const call = async <T,>(method: string, path: string, body?: unknown, actor?: Actor, headers: Record<string, string> = {}): Promise<T> => {
    const token = await getToken();
    const res = await doFetch(`${base}${path}`, {
      method,
      headers: {
        accept: "application/json",
        ...(body === undefined ? {} : { "content-type": "application/json" }),
        ...(token ? { authorization: `Bearer ${token}` } : {}),
        ...(actor ? { "x-actor-hint": encodeURIComponent(actor.name) } : {}),
        ...headers,
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!res.ok) {
      let code = "http_error";
      let message = `${method} ${path} → ${res.status}`;
      let permission: string | undefined;
      try {
        const e = ((await res.json()) as { error?: { code?: string; message?: string; permission?: string } }).error;
        if (e?.code) code = e.code;
        if (e?.message) message = e.message;
        permission = e?.permission;
      } catch {
        /* non-JSON error body */
      }
      if (res.status === 401) opts.onUnauthorized?.();
      throw new ApiError(res.status, code, message, permission);
    }
    if (res.status === 204) return undefined as T;
    return (await res.json()) as T;
  };
  const enc = encodeURIComponent;

  return {
    dashboard: () => call("GET", "/v1/dashboard"),
    exposure: () => call("GET", "/v1/exposure"),
    listClients: (q) => call("GET", `/v1/accounts${q?.search ? `?search=${enc(q.search)}` : ""}`),
    openAccount: (req, actor) => call("POST", "/v1/accounts", req, actor),
    getClient: async (id) => {
      try {
        return await call("GET", `/v1/accounts/${enc(id)}`);
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) return null;
        throw e;
      }
    },
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
    getLpConfig: () => call("GET", "/v1/lp/config"),
    saveLpConfig: (c, actor) => call("PUT", "/v1/lp/config", c, actor),
    listTrades: () => call("GET", "/v1/reports/trades"),
    statements: () => call("GET", "/v1/reports/statements"),
    listAudit: () => call("GET", "/v1/audit"),
    listApprovals: (status = "pending_approval") => call<ApprovalRequest[]>("GET", `/v1/approvals?status=${enc(status)}`),
    approve: (id, actor) => call("POST", `/v1/approvals/${enc(id)}/approve`, {}, actor),
    reject: (id, reason, actor) => call("POST", `/v1/approvals/${enc(id)}/reject`, { reason }, actor),
    listUsers: () => call("GET", "/v1/admin-users"),
    saveUser: (u, actor) => call("PUT", `/v1/admin-users/${enc(u.id)}`, u, actor),
    getSettings: () => call("GET", "/v1/settings"),
    saveSettings: (s, actor) => call("PUT", "/v1/settings", s, actor),

    subscribe(onTopics, onStatus) {
      const ES = opts.eventSource ?? (typeof EventSource === "undefined" ? undefined : EventSource);
      if (!ES) return () => {};
      let es: EventSource | null = null;
      let closed = false;
      let retry: ReturnType<typeof setTimeout> | undefined;
      const open = async () => {
        const token = await getToken();
        if (closed || !token) return;
        // EventSource cannot send headers: exchange the bearer token (header) for a
        // short-lived single-use ticket so the token never appears in a URL / log.
        let ticket: string;
        try {
          ticket = (await call<{ ticket: string }>("POST", "/v1/stream/ticket")).ticket;
        } catch {
          onStatus?.(false);
          if (!closed) retry = setTimeout(() => void open(), 3000);
          return;
        }
        if (closed) return;
        es = new ES(`${base}/v1/stream?ticket=${enc(ticket)}`);
        es.addEventListener("hello", () => onStatus?.(true));
        es.addEventListener("invalidate", (ev) => {
          try {
            const { topics } = JSON.parse((ev as MessageEvent<string>).data) as { topics: string[] };
            if (topics.length) onTopics(topics);
          } catch {
            onTopics(["*"]);
          }
        });
        es.onerror = () => {
          onStatus?.(false);
          es?.close();
          if (!closed) retry = setTimeout(() => void open(), 3000);
        };
      };
      void open();
      return () => {
        closed = true;
        clearTimeout(retry);
        es?.close();
        onStatus?.(false);
      };
    },
  };
}
