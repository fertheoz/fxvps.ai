import { createJSONStorage } from 'zustand/middleware';
import { useTerminal } from '../store/terminal';
import type { ChartView } from './view';

let prepared = false;
/**
 * A detached window must not overwrite the main window's persisted layout, so its
 * store persists to sessionStorage (per webview) from here on.
 */
export function prepareChartWindow(view: ChartView): void {
  if (prepared) return;
  prepared = true;
  useTerminal.persist.setOptions({ storage: createJSONStorage(() => sessionStorage) });
  useTerminal.setState({ layout: 1, activeChart: 0, charts: [{ symbol: view.symbol, timeframe: view.timeframe }] });
}
