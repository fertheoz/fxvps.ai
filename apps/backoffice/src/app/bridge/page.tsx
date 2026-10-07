"use client";
import * as React from "react";
import { Copy, KeyRound, Play, Plus, Rocket, Trash2 } from "lucide-react";
import { Badge, Button, Card, CardContent, CardHeader, CardTitle, Input, Label, PageHeader } from "@/components/ui/primitives";
import { useToast } from "@/components/shell/providers";
import { api, useApiMutation, useApiQuery, useMfaOk } from "@/lib/queries";
import { useActor, useT } from "@/lib/hooks";
import type { Institution } from "@/lib/api/types";
import type { Group } from "@/lib/schemas";

/** MT5 plugin bridge: institutions, live plugin sessions, demo setup, in-browser test. */
export default function BridgePage() {
  const t = useT();
  const actor = useActor();
  const { data } = useApiQuery("listInstitutions", [], { live: 2000 });
  const [shownKey, setShownKey] = React.useState<{ id: string; key: string } | null>(null);
  const list = data?.institutions ?? [];
  const endpoint = data?.endpoint ?? "wss://trade.fxvps.ai/bridge";
  return (
    <div data-testid="page-bridge" className="space-y-4">
      <PageHeader title={t("bridge.title")} />
      <p className="text-sm text-muted-foreground">
        {t("bridge.intro")} <code className="rounded bg-muted px-1">{endpoint}</code>
      </p>
      {shownKey && <KeyCard id={shownKey.id} keyText={shownKey.key} endpoint={endpoint} onClose={() => setShownKey(null)} />}
      {actor.can("lp.manage") && !list.some((i) => i.id.startsWith("mishov")) && <DemoCard onKey={setShownKey} />}
      <InstitutionTable list={list} onKey={setShownKey} />
      {actor.can("lp.manage") && <AddCard onKey={setShownKey} />}
      <TestCard key={shownKey?.key ?? "none"} endpoint={endpoint} list={list} lastKey={shownKey} />
    </div>
  );
}

function KeyCard({ id, keyText, endpoint, onClose }: { id: string; keyText: string; endpoint: string; onClose: () => void }) {
  const t = useT();
  const toast = useToast();
  const conf = `institution = ${id}\nkey = ${keyText}\nendpoints = ${endpoint}\ngroups = real\\*\nfallback = reject`;
  return (
    <Card className="border-amber-500/60" data-testid="bridge-key">
      <CardHeader><CardTitle className="flex items-center gap-2"><KeyRound className="h-4 w-4" />{t("bridge.keyTitle")}</CardTitle></CardHeader>
      <CardContent className="space-y-2 text-sm">
        <p className="text-amber-600 dark:text-amber-400">{t("bridge.keyOnce")}</p>
        <pre className="overflow-x-auto rounded bg-muted p-3 font-mono text-xs">{conf}</pre>
        <div className="flex gap-2">
          <Button size="sm" variant="outline" onClick={() => { void navigator.clipboard.writeText(conf); toast(t("bridge.copied")); }}><Copy className="h-3 w-3" />{t("bridge.copy")}</Button>
          <Button size="sm" variant="outline" onClick={onClose}>{t("bridge.keySaved")}</Button>
        </div>
      </CardContent>
    </Card>
  );
}

const ago = (ms: number) => {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
  return s < 60 ? `${s}s` : s < 3600 ? `${Math.round(s / 60)}m` : `${Math.round(s / 3600)}h`;
};

function InstitutionTable({ list, onKey }: { list: Institution[]; onKey: (k: { id: string; key: string }) => void }) {
  const t = useT();
  const actor = useActor();
  const mfa = useMfaOk();
  const rotate = useApiMutation((id: string) => api().rotateInstitutionKey(id, actor).then((r) => ({ id, key: r.key })), onKey);
  const del = useApiMutation((id: string) => api().deleteInstitution(id, actor));
  return (
    <Card className="overflow-x-auto">
      <CardHeader><CardTitle>{t("bridge.institutions")}</CardTitle></CardHeader>
      <table className="w-full text-sm" data-testid="bridge-table">
        <thead className="bg-muted/50 text-xs text-muted-foreground">
          <tr>{[t("bridge.id"), t("bridge.account"), t("bridge.rate"), t("bridge.status"), t("bridge.server"), t("bridge.orders"), t("bridge.fills"), t("bridge.rejects"), t("bridge.reconcile"), ""].map((h, i) => <th key={i} className="whitespace-nowrap px-3 py-2 text-left font-medium">{h}</th>)}</tr>
        </thead>
        <tbody>
          {list.length === 0 && <tr><td colSpan={10} className="px-3 py-6 text-center text-muted-foreground">{t("bridge.none")}</td></tr>}
          {list.map((i) => {
            const s = i.sessions[0];
            const live = i.sessions.length > 0;
            return (
              <tr key={i.id} className="border-t border-border tabular-nums">
                <td className="px-3 py-2"><div className="font-medium">{i.id}</div><div className="text-xs text-muted-foreground">{i.name}</div></td>
                <td className="px-3 font-mono">{i.account}</td>
                <td className="px-3">{i.ordersPerSec}/s</td>
                <td className="px-3">{live ? <Badge tone="success">{t("bridge.online")} · {ago(s!.since_ms)}</Badge> : <Badge tone="muted">{t("bridge.offline")}</Badge>}</td>
                <td className="px-3 text-xs">{s ? <>{s.server} <span className="text-muted-foreground">v{s.plugin} · {s.ip}</span></> : "—"}</td>
                <td className="px-3">{s?.orders ?? "—"}</td>
                <td className="px-3">{s?.fills ?? "—"}</td>
                <td className={`px-3 ${s && s.rejects > 0 ? "text-red-600 dark:text-red-400" : ""}`}>{s?.rejects ?? "—"}</td>
                <td className="px-3">{s?.reconcile_ok == null ? "—" : s.reconcile_ok ? <Badge tone="success">OK</Badge> : <Badge tone="danger">{t("bridge.diff")}</Badge>}</td>
                <td className="px-3 whitespace-nowrap">
                  {actor.can("lp.manage") && (
                    <>
                      <Button size="sm" variant="outline" disabled={!mfa || rotate.isPending} onClick={() => rotate.mutate(i.id)}><KeyRound className="h-3 w-3" />{t("bridge.rotate")}</Button>{" "}
                      <Button size="sm" variant="outline" disabled={!mfa || del.isPending} onClick={() => { if (window.confirm(t("bridge.confirmDelete"))) del.mutate(i.id); }}><Trash2 className="h-3 w-3" /></Button>
                    </>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </Card>
  );
}

function AddCard({ onKey }: { onKey: (k: { id: string; key: string }) => void }) {
  const t = useT();
  const actor = useActor();
  const mfa = useMfaOk();
  const [f, setF] = React.useState({ id: "", name: "", account: "", rate: 100, ips: "" });
  const add = useApiMutation(
    () => api().createInstitution({ id: f.id, name: f.name, account: f.account, ordersPerSec: f.rate, ips: f.ips.split(",").map((x) => x.trim()).filter(Boolean) }, actor).then((r) => ({ id: r.institution.id, key: r.key })),
    (k) => { onKey(k); setF({ id: "", name: "", account: "", rate: 100, ips: "" }); },
  );
  return (
    <Card>
      <CardHeader><CardTitle className="flex items-center gap-2"><Plus className="h-4 w-4" />{t("bridge.add")}</CardTitle></CardHeader>
      <CardContent className="grid gap-3 sm:grid-cols-5">
        <Field label={t("bridge.id")}><Input value={f.id} onChange={(e) => setF({ ...f, id: e.target.value })} placeholder="kurum1" /></Field>
        <Field label={t("bridge.name")}><Input value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} /></Field>
        <Field label={t("bridge.account")}><Input value={f.account} onChange={(e) => setF({ ...f, account: e.target.value })} placeholder="100012" /></Field>
        <Field label={t("bridge.rate")}><Input type="number" value={f.rate} onChange={(e) => setF({ ...f, rate: Number(e.target.value) })} /></Field>
        <Field label={t("bridge.ips")}><Input value={f.ips} onChange={(e) => setF({ ...f, ips: e.target.value })} placeholder={t("bridge.ipsHint")} /></Field>
        <div className="sm:col-span-5">
          <Button disabled={!mfa || add.isPending || !f.id || !f.account} onClick={() => add.mutate(undefined)}><Plus className="h-3 w-3" />{t("bridge.create")}</Button>
          <span className="ml-3 text-xs text-muted-foreground">{t("bridge.nettingHint")}</span>
        </div>
      </CardContent>
    </Card>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return <div className="space-y-1"><Label>{label}</Label>{children}</div>;
}

const DEMO_GROUP = "kurum-demo";

/** One click: netting A-book group, the demo account, 1M deposit, institution + key. */
function DemoCard({ onKey }: { onKey: (k: { id: string; key: string }) => void }) {
  const t = useT();
  const actor = useActor();
  const mfa = useMfaOk();
  const toast = useToast();
  const [name, setName] = React.useState("MISHOV Demo");
  const [email, setEmail] = React.useState("demo@mishov.example");
  const [amount, setAmount] = React.useState(1_000_000);
  const [steps, setSteps] = React.useState<string[]>([]);
  const [busy, setBusy] = React.useState(false);
  const log = (s: string) => setSteps((x) => [...x, s]);
  const run = async () => {
    setBusy(true);
    setSteps([]);
    try {
      const a = api();
      const symbols = (await a.listSymbols()).map((s) => s.name);
      const group: Group = {
        id: DEMO_GROUP, name: DEMO_GROUP, currency: "USD", leverage: 100, marginMode: "retail_netting",
        marginCallPct: 100, stopOutPct: 50, commissionType: "symbol", commissionValue: 0, markupPoints: 0,
        swapMultiplier: 1, book: "A", symbols, esma: "none", partialFill: "retry", maxAttempts: 5,
        markupBidPoints: null, markupAskPoints: null, symbolMarkups: {}, maxSlippagePoints: null,
        passPriceImprovement: true, weekendLeverage: null,
      };
      if (!(await a.listGroups()).some((g) => g.id === DEMO_GROUP)) {
        await a.saveGroup(group, actor);
        log(t("bridge.demo.group"));
      } else log(t("bridge.demo.groupExists"));
      const acc = await a.openAccount({ name, email, group: DEMO_GROUP }, actor);
      log(`${t("bridge.demo.account")} ${acc.id}`);
      const dep = await a.balanceOp({ clientId: acc.id, type: "deposit", amount: Math.round(amount * 100), currency: "USD", reason: "Partner demo funding (bridge)", idempotencyKey: crypto.randomUUID() }, actor);
      log(dep.status === "pending_approval" ? t("bridge.demo.depositPending") : t("bridge.demo.deposit"));
      const id = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "").slice(0, 40) || "demo";
      const r = await a.createInstitution({ id, name, account: acc.id, ordersPerSec: 100 }, actor);
      log(`${t("bridge.demo.institution")} ${r.institution.id}`);
      onKey({ id: r.institution.id, key: r.key });
      toast(t("bridge.demo.done"));
    } catch (e) {
      log(`✗ ${(e as Error).message}`);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Card className="border-primary/50" data-testid="bridge-demo">
      <CardHeader><CardTitle className="flex items-center gap-2"><Rocket className="h-4 w-4" />{t("bridge.demo.title")}</CardTitle></CardHeader>
      <CardContent className="space-y-3 text-sm">
        <p className="text-muted-foreground">{t("bridge.demo.desc")}</p>
        <div className="grid gap-3 sm:grid-cols-3">
          <Field label={t("bridge.name")}><Input value={name} onChange={(e) => setName(e.target.value)} /></Field>
          <Field label="E-mail"><Input value={email} onChange={(e) => setEmail(e.target.value)} /></Field>
          <Field label={t("bridge.demo.amount")}><Input type="number" value={amount} onChange={(e) => setAmount(Number(e.target.value))} /></Field>
        </div>
        <Button disabled={!mfa || busy || name.trim().length < 2} onClick={() => void run()} data-testid="bridge-demo-run"><Rocket className="h-3 w-3" />{t("bridge.demo.run")}</Button>
        {!mfa && <span className="ml-3 text-xs text-amber-600">{t("bridge.mfa")}</span>}
        {steps.length > 0 && <ul className="space-y-1 font-mono text-xs">{steps.map((s, i) => <li key={i}>{s.startsWith("✗") ? s : `✓ ${s}`}</li>)}</ul>}
      </CardContent>
    </Card>
  );
}

/** Runs a fake MT5 server in the browser against the live bridge. */
function TestCard({ endpoint, list, lastKey }: { endpoint: string; list: Institution[]; lastKey: { id: string; key: string } | null }) {
  const t = useT();
  const [inst, setInst] = React.useState(lastKey?.id ?? list[0]?.id ?? "");
  const [key, setKey] = React.useState(lastKey?.key ?? "");
  const [symbol, setSymbol] = React.useState("EURUSD");
  const [lots, setLots] = React.useState("0.01");
  const [count, setCount] = React.useState(2);
  const [lines, setLines] = React.useState<string[]>([]);
  const [running, setRunning] = React.useState(false);
  const run = () => {
    setLines([]);
    setRunning(true);
    const out = (s: string) => setLines((x) => [...x.slice(-200), `${new Date().toISOString().slice(11, 23)}  ${s}`]);
    const ws = new WebSocket(endpoint);
    const t0 = performance.now();
    const sent = new Map<string, number>();
    let quotes = 0, done = 0;
    let lastQuote = "";
    const finish = () => { setRunning(false); ws.close(); };
    ws.onopen = () => {
      out(`→ connect ${endpoint}`);
      ws.send(JSON.stringify({ t: "hello", v: 1, institution: inst, key, server: "Console test (fake MT5)", plugin: "console" }));
    };
    ws.onmessage = (ev) => {
      const m = JSON.parse(String(ev.data));
      if (m.t === "quote") {
        quotes++;
        if (m.s === symbol) lastQuote = `${m.b} / ${m.a}`;
        return;
      }
      if (m.t === "welcome") {
        out(`← welcome: account ${m.account}, ${m.symbols.length} symbols (${Math.round(performance.now() - t0)} ms)`);
        setTimeout(() => {
          out(`  ${symbol} ${lastQuote || "…"}  (${quotes} quotes)`);
          for (let k = 0; k < count; k++) {
            const id = `console-${Date.now()}-${k}`;
            const side = k % 2 === 0 ? "buy" : "sell";
            sent.set(id, performance.now());
            out(`→ order ${id.slice(-6)} ${side} ${lots} ${symbol}`);
            ws.send(JSON.stringify({ t: "order", id, login: 1, group: "test", symbol, side, lots, kind: "market" }));
          }
        }, 800);
      } else if (m.t === "fill") {
        const ms = Math.round(performance.now() - (sent.get(m.id) ?? 0));
        out(`← fill ${m.id.slice(-6)} ${m.filled} @ ${m.avg}${m.done ? "" : " (partial)"}  ${ms} ms`);
        if (m.done) done++;
      } else if (m.t === "reject") {
        done++;
        out(`← reject ${m.id.slice(-6)} ${m.code}: ${m.text}`);
      } else if (m.t === "reconcile_result") {
        out(`← reconcile ${m.ok ? "OK" : "DIFF " + JSON.stringify(m.diff)}  account net ${JSON.stringify(m.ours)}`);
        finish();
      } else if (m.t === "error") {
        out(`✗ ${m.code}: ${m.text}`);
        finish();
      }
      if (done === count && count > 0 && m.t !== "reconcile_result") {
        done = -1;
        ws.send(JSON.stringify({ t: "reconcile", net: {} }));
        out(`→ reconcile`);
      }
    };
    ws.onerror = () => { out("✗ connection error"); finish(); };
    ws.onclose = () => setRunning(false);
    setTimeout(() => { if (ws.readyState === WebSocket.OPEN) { out("✗ timeout"); finish(); } }, 20000);
  };
  return (
    <Card data-testid="bridge-test">
      <CardHeader><CardTitle className="flex items-center gap-2"><Play className="h-4 w-4" />{t("bridge.test.title")}</CardTitle></CardHeader>
      <CardContent className="space-y-3 text-sm">
        <p className="text-muted-foreground">{t("bridge.test.desc")}</p>
        <div className="grid gap-3 sm:grid-cols-5">
          <Field label={t("bridge.id")}><Input value={inst} onChange={(e) => setInst(e.target.value)} list="bridge-ids" /></Field>
          <datalist id="bridge-ids">{list.map((i) => <option key={i.id} value={i.id} />)}</datalist>
          <Field label={t("bridge.key")}><Input type="password" value={key} onChange={(e) => setKey(e.target.value)} placeholder="fxk_…" /></Field>
          <Field label="Symbol"><Input value={symbol} onChange={(e) => setSymbol(e.target.value.toUpperCase())} /></Field>
          <Field label="Lots"><Input value={lots} onChange={(e) => setLots(e.target.value)} /></Field>
          <Field label={t("bridge.test.count")}><Input type="number" min={1} max={20} value={count} onChange={(e) => setCount(Math.max(1, Math.min(20, Number(e.target.value))))} /></Field>
        </div>
        <Button disabled={running || !inst || !key} onClick={run} data-testid="bridge-test-run"><Play className="h-3 w-3" />{running ? t("bridge.test.running") : t("bridge.test.run")}</Button>
        <span className="ml-3 text-xs text-muted-foreground">{t("bridge.test.note")}</span>
        {lines.length > 0 && <pre className="max-h-80 overflow-auto rounded bg-muted p-3 font-mono text-xs" data-testid="bridge-test-log">{lines.join("\n")}</pre>}
      </CardContent>
    </Card>
  );
}
