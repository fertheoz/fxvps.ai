import type { AdminApi, Actor, ApprovalRequest, RoutingRule } from "./types";

/** Error returned by the admin API: `{ error: { code, message, permission? } }`. */

/** Wire form of a routing rule: lots travel as centi-lots. */
type WireRule = Omit<RoutingRule, "minLots" | "maxLots"> & { minCentilots: number | null; maxCentilots: number | null };
const toWireRule = (r: RoutingRule): WireRule => {
  const { minLots, maxLots, ...rest } = r;
  return { ...rest, minCentilots: minLots === null ? null : Math.round(minLots * 100), maxCentilots: maxLots === null ? null : Math.round(maxLots * 100) };
};
const fromWireRule = (w: WireRule): RoutingRule => {
  const { minCentilots, maxCentilots, ...rest } = w;
  return { ...rest, minLots: minCentilots === null ? null : minCentilots / 100, maxLots: maxCentilots === null ? null : maxCentilots / 100 };
};

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
    dashboardSeries: (range) => call("GET", `/v1/dashboard/series?range=${encodeURIComponent(range)}`),
    exposure: () => call("GET", "/v1/exposure"),
    listAlerts: () => call("GET", "/v1/alerts"),
    ackAlert: (id, actor) => call("POST", `/v1/alerts/${enc(id)}/ack`, {}, actor),
    hedgePolicy: () => call("GET", "/v1/risk/hedge"),
    // lots travel as raw 1e8 fixed-point on the wire (engine Qty)
    saveHedgePolicy: (p, actor) => call("PUT", "/v1/risk/hedge", {
      ...p,
      defaultSymbolLimit: p.defaultSymbolLimit == null ? null : Math.round(p.defaultSymbolLimit * 1e8),
      totalLimit: p.totalLimit == null ? null : Math.round(p.totalLimit * 1e8),
      accountLimit: p.accountLimit == null ? null : Math.round(p.accountLimit * 1e8),
      symbolLimits: Object.fromEntries(Object.entries(p.symbolLimits).map(([k, v]) => [k, Math.round(v * 1e8)])),
    }, actor),
    clientFlow: () => call("GET", "/v1/reports/clients"),
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
    setGroup: (id, group, actor) => call("PATCH", `/v1/accounts/${enc(id)}/group`, { group }, actor),
    setProfile: (id, p, actor) => call("PATCH", `/v1/accounts/${enc(id)}/profile`, p, actor),
    setIb: (id, p, actor) => call("PATCH", `/v1/accounts/${enc(id)}/ib`, p, actor),
    listKycDocs: (id) => call("GET", `/v1/accounts/${enc(id)}/kyc/documents`),
    kycDocBlob: async (id, doc) => {
      const token = await getToken();
      const res = await doFetch(`${base}/v1/accounts/${enc(id)}/kyc/documents/${enc(doc)}`, { headers: token ? { authorization: `Bearer ${token}` } : {} });
      if (!res.ok) throw new ApiError(res.status, "http_error", `document ${res.status}`);
      return await res.blob();
    },
    listFunding: (status = "open") => call("GET", `/v1/funding?status=${enc(status)}`),
    decideFunding: (id, decision, note, actor) => call("POST", `/v1/funding/${enc(id)}/decide`, { decision, note }, actor),
    ibReport: (from, to) => call("GET", `/v1/reports/ib${from || to ? `?from=${from ?? ""}&to=${to ?? ""}` : ""}`),
    transactions: (from, to) => call("GET", `/v1/reports/transactions${from || to ? `?from=${from ?? ""}&to=${to ?? ""}` : ""}`),
    bestExecution: (from, to) => call("GET", `/v1/reports/best-execution${from || to ? `?from=${from ?? ""}&to=${to ?? ""}` : ""}`),
    auditChain: () => call("GET", "/v1/audit/chain"),
    getAlertSettings: () => call("GET", "/v1/settings/alerts"),
    saveAlertSettings: (s, actor) => call("PUT", "/v1/settings/alerts", s, actor),
    getCalendar: () => call("GET", "/v1/settings/calendar"),
    saveCalendar: (c, actor) => call("PUT", "/v1/settings/calendar", c, actor),
    ruleVersions: () => call("GET", "/v1/rules/versions"),
    restoreRuleVersion: async (id, actor) => (await call<WireRule[]>("POST", `/v1/rules/versions/${enc(id)}/restore`, {}, actor)).map(fromWireRule),
    simState: () => call("GET", "/v1/lp/sim/state"),
    simShock: (symbol, pct, actor) => call("POST", "/v1/lp/sim/shock", { symbol, pct }, actor),
    simScenario: (s, actor) => call("POST", "/v1/lp/sim/scenario", s, actor),
    perf: () => call("GET", "/v1/perf"),
    listTenants: () => call("GET", "/v1/tenants"),
    saveTenants: (ts, actor) => call("PUT", "/v1/tenants", ts, actor),
    listGroups: () => call("GET", "/v1/groups"),
    listRules: async () => (await call<WireRule[]>("GET", "/v1/rules")).map(fromWireRule),
    saveRules: async (rules, actor) => (await call<WireRule[]>("PUT", "/v1/rules", rules.map(toWireRule), actor)).map(fromWireRule),
    rulesDryRun: () => call("GET", "/v1/rules/dry-run"),
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
    listInstitutions: () => call("GET", "/v1/bridge/institutions"),
    getAudit: () => call("GET", "/v1/denetim"),
    getPartnerOverview: () => call("GET", "/v1/partner/overview"),
    getCopyOverview: () => call("GET", "/v1/copy"),
    saveCopyStrategy: (account, s, actor) => call("PUT", `/v1/copy/strategies/${account}`, s, actor),
    copyUnsubscribe: (r, actor) => call("POST", "/v1/copy/unsubscribe", r, actor),
    copySettle: (provider, actor) => call("POST", "/v1/copy/settle", { provider }, actor),
    saveAuditSettings: (s, actor) => call("PUT", "/v1/denetim", s, actor),
    resetAudit: (actor) => call("POST", "/v1/denetim/reset", {}, actor),
    createInstitution: (r, actor) => call("POST", "/v1/bridge/institutions", r, actor),
    updateInstitution: (id, r, actor) => call("PUT", `/v1/bridge/institutions/${enc(id)}`, r, actor),
    rotateInstitutionKey: (id, actor) => call("POST", `/v1/bridge/institutions/${enc(id)}/rotate-key`, {}, actor),
    deleteInstitution: (id, actor) => call("DELETE", `/v1/bridge/institutions/${enc(id)}`, undefined, actor),
    reconnect: (id, actor) => call("POST", `/v1/lp/sessions/${enc(id)}/reconnect`, {}, actor),
    getLpConfig: () => call("GET", "/v1/lp/config"),
    saveLpConfig: (c, actor) => call("PUT", "/v1/lp/config", c, actor),
    getLpAggregation: () => call("GET", "/v1/lp/aggregation"),
    saveLpAggregation: (c, actor) => call("PUT", "/v1/lp/aggregation", c, actor),
    lpReport: () => call("GET", "/v1/reports/lp"),
    listTrades: () => call("GET", "/v1/reports/trades"),
    statements: () => call("GET", "/v1/reports/statements"),
    listLpExecutions: () => call("GET", "/v1/reports/lp-executions"),
    execution: () => call("GET", "/v1/reports/execution"),
    revenue: () => call("GET", "/v1/reports/revenue"),
    listAudit: () => call("GET", "/v1/audit"),
    listApprovals: (status = "pending_approval") => call<ApprovalRequest[]>("GET", `/v1/approvals?status=${enc(status)}`),
    approve: (id, actor) => call("POST", `/v1/approvals/${enc(id)}/approve`, {}, actor),
    reject: (id, reason, actor) => call("POST", `/v1/approvals/${enc(id)}/reject`, { reason }, actor),
    listUsers: () => call("GET", "/v1/admin-users"),
    saveUser: (u, actor) => call("PUT", `/v1/admin-users/${enc(u.id)}`, u, actor),
    getSettings: () => call("GET", "/v1/settings"),
    getSwapConfig: () => call("GET", "/v1/settings/swap"),
    saveSwapConfig: (c, actor) => call("PUT", "/v1/settings/swap", { enabled: c.enabled, rolloverHourUtc: c.rolloverHourUtc, skipWeekend: c.skipWeekend }, actor),
    runRollover: (actor) => call("POST", "/v1/settings/swap/rollover", {}, actor),
    getMe: () => call("GET", "/v1/me"),
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
