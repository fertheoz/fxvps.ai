import type { z } from "zod";
import type { Session } from "./schemas";

type Sess = z.infer<typeof Session>;
const DAYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"] as const;

const toMin = (hhmm: string) => {
  const [h = "0", m = "0"] = hhmm.split(":");
  return Number(h) * 60 + Number(m);
};

/** Is the symbol tradable at the given instant (sessions are UTC)? */
export function isTradingAt(sessions: readonly Sess[], at: Date): boolean {
  const day = DAYS[at.getUTCDay()];
  const minute = at.getUTCHours() * 60 + at.getUTCMinutes();
  return sessions.some((s) => s.day === day && minute >= toMin(s.open) && minute < toMin(s.close));
}
