"use client";
import { Bar, BarChart, CartesianGrid, Cell, Legend, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";

export const CHART = { a: "#3b82f6", b: "#f59e0b", up: "#10b981", down: "#ef4444", grid: "rgba(128,128,128,0.18)", axis: "#8b93a7" };
const tooltipStyle = { background: "var(--card)", border: "1px solid var(--border)", borderRadius: 6, fontSize: 12, color: "var(--foreground)" };

export function ExposureChart({ data, fmt }: { data: { symbol: string; notional: number }[]; fmt: (minor: number) => string }) {
  return (
    <ResponsiveContainer width="100%" height={Math.max(220, data.length * 26)}>
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
    <ResponsiveContainer width="100%" height={240}>
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

export function FlowChart({ data, fmt, labels }: { data: { day: string; deposits: number; withdrawals: number }[]; fmt: (minor: number) => string; labels: [string, string] }) {
  return (
    <ResponsiveContainer width="100%" height={240}>
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
