/** SVG path of a sparkline through `points` (oldest first) in a `w` x `h` box. */
export function sparkPath(points: number[], w: number, h: number, pad = 2): string {
  if (points.length < 2) return '';
  const min = Math.min(...points);
  const max = Math.max(...points);
  const step = w / (points.length - 1);
  // a flat series sits in the middle instead of on an edge
  const y = (v: number) => (max === min ? h / 2 : pad + (h - 2 * pad) * (1 - (v - min) / (max - min)));
  return points.map((v, i) => `${i ? 'L' : 'M'}${(i * step).toFixed(1)},${y(v).toFixed(1)}`).join(' ');
}
