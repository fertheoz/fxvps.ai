export function formatTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

/** Parse a user-typed decimal ("1,2345" or "1.2345"); undefined for empty/invalid. */
export function parseDecimal(s: string): number | undefined {
  const t = s.trim().replace(',', '.');
  if (!t || !/^-?\d*(\.\d*)?$/.test(t) || t === '.' || t === '-') return undefined;
  const n = Number(t);
  return Number.isFinite(n) ? n : undefined;
}

export function formatTimeShort(ms: number): string {
  return formatTime(ms).slice(5);
}
