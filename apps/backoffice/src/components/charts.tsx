"use client";
import { Bar, BarChart, CartesianGrid, Cell, Legend, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";

export const CHART = { a: "#3b82f6", b: "#f59e0b", up: "#10b981", down: "#ef4444", grid: "rgba(128,128,128,0.18)", axis: "#8b93a7" };
const tooltipStyle = { background: "var(--card)", border: "1px solid var(--border)", borderRadius: 6, fontSize: 12, color: "var(--foreground)" };

export function ExposureChart({ data, fmt }: { data: { symbol: string; notional: number }[]; fmt: (minor: number) => string }) {
  return (
    <ResponsiveContainer width="99%" height={Math.max(220, data.length * 26)}>
      <BarChart data={data} layout="vertical" margin={{ left: 8, right: 16 }}>
        <CartesianGrid stroke={CHART.grid} horizontal={false} />
        <XAxis type="number" tickFormatter={(v: number) => fmt(v)} stroke={CHART.axis} fontSize={11} />
        <YAxis type="category" dataKey="symbol" width={64} stroke={CHART.axis} fontSize={11} />
        <Tooltip formatter={(v) => fmt(Number(v))} contentStyle={tooltipStyle} cursor={{ fill: "rgba(128,128,128,0.08)" }} />
        <Bar dataKey="notional" name="Net notional" isAnimationActive={false}>
          {data.map((d) => <Cell key={d.symbol} fill={d.notional >= 0 ? CHART.up : CHART.down} />)}
        </Bar>
      </BarChart>
    </ResponsiveContainer>
  );
}

export function PnlChart({ data, fmt }: { data: { t: string; aBook: number; bBook: number }[]; fmt: (minor: number) => string }) {
  return (
    <ResponsiveContainer width="99%" height={240}>
      <LineChart data={data} margin={{ left: 8, right: 16 }}>
        <CartesianGrid stroke={CHART.grid} />
        <XAxis dataKey="t" stroke={CHART.axis} fontSize={11} interval={3} />
        <YAxis tickFormatter={(v: number) => fmt(v)} stroke={CHART.axis} fontSize={11} width={70} />
        <Tooltip formatter={(v) => fmt(Number(v))} contentStyle={tooltipStyle} />
        <Legend wrapperStyle={{ fontSize: 12 }} />
        <Line dataKey="aBook" name="A-book" stroke={CHART.a} dot={false} strokeWidth={2} isAnimationActive={false} />
        <Line dataKey="bBook" name="B-book" stroke={CHART.b} dot={false} strokeWidth={2} isAnimationActive={false} />
      </LineChart>
    </ResponsiveContainer>
  );
}

/** Revenue legs per bucket (stacked): markup, commission, B-book result. */
export function RevenueChart({ data, fmt, labels }: { data: { label: string; markup: number; commission: number; bBook: number }[]; fmt: (minor: number) => string; labels: [string, string, string] }) {
  return (
    <ResponsiveContainer width="99%" height={260}>
      <BarChart data={data} margin={{ left: 8, right: 16 }} stackOffset="sign">
        <CartesianGrid stroke={CHART.grid} vertical={false} />
        <XAxis dataKey="label" stroke={CHART.axis} fontSize={11} interval="preserveStartEnd" />
        <YAxis tickFormatter={(v: number) => fmt(v)} stroke={CHART.axis} fontSize={11} width={70} />
        <Tooltip formatter={(v) => fmt(Number(v))} contentStyle={tooltipStyle} cursor={{ fill: "rgba(128,128,128,0.08)" }} />
        <Legend wrapperStyle={{ fontSize: 12 }} />
        <Bar dataKey="markup" name={labels[0]} stackId="r" fill={CHART.a} isAnimationActive={false} />
        <Bar dataKey="commission" name={labels[1]} stackId="r" fill={CHART.up} isAnimationActive={false} />
        <Bar dataKey="bBook" name={labels[2]} stackId="r" fill={CHART.b} isAnimationActive={false} />
      </BarChart>
    </ResponsiveContainer>
  );
}

/** Traded lots per bucket with the order / reject counts on a second axis. */
export function VolumeChart({ data, labels }: { data: { label: string; lots: number; orders: number; rejects: number }[]; labels: [string, string, string] }) {
  return (
    <ResponsiveContainer width="99%" height={260}>
      <BarChart data={data} margin={{ left: 8, right: 8 }}>
        <CartesianGrid stroke={CHART.grid} vertical={false} />
        <XAxis dataKey="label" stroke={CHART.axis} fontSize={11} interval="preserveStartEnd" />
        <YAxis yAxisId="lots" stroke={CHART.axis} fontSize={11} width={48} />
        <YAxis yAxisId="n" orientation="right" stroke={CHART.axis} fontSize={11} width={32} allowDecimals={false} />
        <Tooltip contentStyle={tooltipStyle} cursor={{ fill: "rgba(128,128,128,0.08)" }} />
        <Legend wrapperStyle={{ fontSize: 12 }} />
        <Bar yAxisId="lots" dataKey="lots" name={labels[0]} fill={CHART.a} isAnimationActive={false} />
        <Line yAxisId="n" dataKey="orders" name={labels[1]} stroke={CHART.up} dot={false} strokeWidth={2} isAnimationActive={false} />
        <Line yAxisId="n" dataKey="rejects" name={labels[2]} stroke={CHART.down} dot={false} strokeWidth={2} isAnimationActive={false} />
      </BarChart>
    </ResponsiveContainer>
  );
}

export function FlowChart({ data, fmt, labels }: { data: { day: string; deposits: number; withdrawals: number }[]; fmt: (minor: number) => string; labels: [string, string] }) {
  return (
    <ResponsiveContainer width="99%" height={240}>
      <BarChart data={data} margin={{ left: 8, right: 16 }}>
        <CartesianGrid stroke={CHART.grid} vertical={false} />
        <XAxis dataKey="day" stroke={CHART.axis} fontSize={11} />
        <YAxis tickFormatter={(v: number) => fmt(v)} stroke={CHART.axis} fontSize={11} width={70} />
        <Tooltip formatter={(v) => fmt(Number(v))} contentStyle={tooltipStyle} cursor={{ fill: "rgba(128,128,128,0.08)" }} />
        <Legend wrapperStyle={{ fontSize: 12 }} />
        <Bar dataKey="deposits" name={labels[0]} fill={CHART.up} isAnimationActive={false} />
        <Bar dataKey="withdrawals" name={labels[1]} fill={CHART.down} isAnimationActive={false} />
      </BarChart>
    </ResponsiveContainer>
  );
}

/** Client slippage (points) and LP p95 latency (ms) per bucket on two axes. */
export function ExecutionChart({ data, labels }: { data: { label: string; avgSlipPts?: number; p95LatencyMs?: number }[]; labels: [string, string] }) {
  return (
    <ResponsiveContainer width="99%" height={240}>
      <LineChart data={data} margin={{ left: 8, right: 8 }}>
        <CartesianGrid stroke={CHART.grid} vertical={false} />
        <XAxis dataKey="label" stroke={CHART.axis} fontSize={11} interval="preserveStartEnd" />
        <YAxis yAxisId="slip" stroke={CHART.axis} fontSize={11} width={40} />
        <YAxis yAxisId="lat" orientation="right" stroke={CHART.axis} fontSize={11} width={44} allowDecimals={false} />
        <Tooltip contentStyle={tooltipStyle} />
        <Legend wrapperStyle={{ fontSize: 12 }} />
        <Line yAxisId="slip" dataKey="avgSlipPts" name={labels[0]} stroke={CHART.b} dot={false} strokeWidth={2} isAnimationActive={false} connectNulls />
        <Line yAxisId="lat" dataKey="p95LatencyMs" name={labels[1]} stroke={CHART.a} dot={false} strokeWidth={2} isAnimationActive={false} connectNulls />
      </LineChart>
    </ResponsiveContainer>
  );
}

/** SVG path of a sparkline through `points` (oldest first) in a `w` x `h` box. */
export function sparkPath(points: number[], w: number, h: number, pad = 2): string {
  if (points.length < 2) return "";
  const min = Math.min(...points);
  const max = Math.max(...points);
  const step = w / (points.length - 1);
  // a flat series sits in the middle instead of on an edge
  const y = (v: number) => (max === min ? h / 2 : pad + (h - 2 * pad) * (1 - (v - min) / (max - min)));
  return points.map((v, i) => `${i ? "L" : "M"}${(i * step).toFixed(1)},${y(v).toFixed(1)}`).join(" ");
}

/** Compact trend line for table cells: green when it ends at or above its start. */
export function Sparkline({ points, width = 120, height = 28, label }: { points: number[]; width?: number; height?: number; label?: string }) {
  const d = sparkPath(points, width, height);
  if (!d) return <span className="text-muted-foreground">—</span>;
  const up = (points[points.length - 1] ?? 0) >= (points[0] ?? 0);
  return (
    <svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} role="img" aria-label={label} data-testid="sparkline">
      <path d={d} fill="none" stroke={up ? CHART.up : CHART.down} strokeWidth={1.5} strokeLinejoin="round" />
    </svg>
  );
}
