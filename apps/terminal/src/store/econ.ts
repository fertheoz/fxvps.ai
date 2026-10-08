import { useEffect, useState } from 'react';
import { create } from 'zustand';
import { clientApi, clientApiBase } from '../api/clientApi';
import { demoEvents, type EconEvent } from '../lib/econCalendar';

const DAY = 86_400_000;
const REFRESH_MS = 10 * 60_000;
const RETRY_MS = 60_000;

interface EconState {
  events: EconEvent[];
  /** When the list was last fetched (or last failed), epoch ms. */
  loadedAt: number;
  loading: boolean;
  error: string | null;
  /** Fetches the last 30 days to 14 days ahead; at most every 10 minutes unless forced. */
  load(force?: boolean): Promise<void>;
}

/** Economic calendar shared by the Calendar tab and every chart's pins (one fetch for all). */
export const useEcon = create<EconState>()((set, get) => ({
  events: [],
  loadedAt: 0,
  loading: false,
  error: null,
  async load(force = false) {
    const s = get();
    if (s.loading || (!force && Date.now() - s.loadedAt < REFRESH_MS)) return;
    set({ loading: true });
    const now = Date.now();
    try {
      // mock terminal (no gateway): a demo calendar so the tab and the pins have something to show
      const events = clientApiBase() ? (await clientApi.calendar(now - 30 * DAY, now + 14 * DAY)).events : demoEvents(now);
      set({ events, loadedAt: Date.now(), loading: false, error: null });
    } catch (e) {
      // keep the last list; try again in a minute
      set({ loadedAt: Date.now() - REFRESH_MS + RETRY_MS, loading: false, error: (e as Error).message });
    }
  },
}));

/** The calendar, loaded on mount and refreshed while the caller is mounted. */
export function useEconEvents(): EconEvent[] {
  const events = useEcon((s) => s.events);
  useEffect(() => {
    void useEcon.getState().load();
    const id = setInterval(() => void useEcon.getState().load(), RETRY_MS);
    return () => clearInterval(id);
  }, []);
  return events;
}

/** Wall clock for render (countdowns, past / upcoming styling), ticking every `everyMs`. */
export function useNow(everyMs = 30_000): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), everyMs);
    return () => clearInterval(id);
  }, [everyMs]);
  return now;
}
