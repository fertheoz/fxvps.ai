/**
 * Coalesces high-frequency updates (ticks) into one flush per animation frame.
 * Later values for the same key overwrite earlier ones (conflation).
 */
export class RafBatcher<T> {
  private pending = new Map<string, T>();
  private scheduled = false;

  constructor(
    private readonly flush: (items: T[]) => void,
    private readonly schedule: (cb: () => void) => void = (cb) =>
      typeof requestAnimationFrame === 'function' ? requestAnimationFrame(() => cb()) : setTimeout(cb, 16),
  ) {}

  push(key: string, item: T): void {
    this.pending.set(key, item);
    if (!this.scheduled) {
      this.scheduled = true;
      this.schedule(() => this.run());
    }
  }

  private run(): void {
    this.scheduled = false;
    if (this.pending.size === 0) return;
    const items = [...this.pending.values()];
    this.pending.clear();
    this.flush(items);
  }
}
