import { Badge } from "@/components/ui/primitives";
import type { Client, FixSession } from "@/lib/schemas";

export function KycBadge({ kyc }: { kyc: Client["kyc"] }) {
  const tone = kyc === "approved" ? "success" : kyc === "pending" ? "warning" : kyc === "rejected" ? "danger" : "muted";
  return <Badge tone={tone}>{kyc}</Badge>;
}
export function StatusBadge({ status }: { status: Client["status"] }) {
  return <Badge tone={status === "active" ? "success" : status === "readonly" ? "info" : "muted"}>{status}</Badge>;
}
export function BookBadge({ book }: { book: "A" | "B" }) {
  return <Badge tone={book === "A" ? "info" : "warning"}>{book}-book</Badge>;
}
export function SideBadge({ side }: { side: "buy" | "sell" }) {
  return <Badge tone={side === "buy" ? "success" : "danger"}>{side.toUpperCase()}</Badge>;
}
export function FixBadge({ status }: { status: FixSession["status"] }) {
  return <Badge tone={status === "logged_on" ? "success" : status === "connecting" ? "warning" : "danger"}>{status.replace("_", " ")}</Badge>;
}
export function marginLevel(equity: number, margin: number): number | null {
  return margin > 0 ? (equity / margin) * 100 : null;
}

/** 0–100 toxic-flow score: green < 30, amber < 60, red otherwise. */
export function ToxicityBadge({ score }: { score: number }) {
  const tone = score >= 60 ? "danger" : score >= 30 ? "warning" : "success";
  return <Badge tone={tone} data-testid="toxicity">{score}</Badge>;
}
