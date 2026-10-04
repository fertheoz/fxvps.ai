import { useEffect, useState } from 'react';
import { Group, Panel, Separator } from 'react-resizable-panels';
import type { TradingApi } from './api/types';
import { bootstrap } from './store/api';
import { useTerminal } from './store/terminal';
import { useKeyboardShortcuts } from './shortcuts';
import { useT } from './hooks';
import { TopBar } from './components/TopBar';
import { MarketWatch } from './components/MarketWatch';
import { ChartGrid } from './components/ChartGrid';
import { OrderTicket } from './components/OrderTicket';
import { DepthOfMarket } from './components/DepthOfMarket';
import { Toolbox } from './components/Toolbox';
import { CommandPalette, ShortcutsDialog, TicketDialog, Toasts } from './components/Dialogs';

// One bootstrap per API instance for the app's lifetime (StrictMode mounts effects twice).
const started = new WeakMap<TradingApi, Promise<() => void>>();
function startOnce(api: TradingApi): Promise<() => void> {
  let p = started.get(api);
  if (!p) {
    p = bootstrap(api);
    started.set(api, p);
  }
  return p;
}

export function App({ api }: { api: TradingApi }) {
  const t = useT();
  const theme = useTerminal((s) => s.theme);
  const lang = useTerminal((s) => s.lang);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useKeyboardShortcuts();

  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);
  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  useEffect(() => {
    let cancelled = false;
    startOnce(api)
      .then(() => !cancelled && setReady(true))
      .catch((e: Error) => !cancelled && setError(e.message));
    return () => {
      cancelled = true;
    };
  }, [api]);

  if (error) return <div className="p-6 text-down">{error}</div>;
  if (!ready) return <div className="h-full grid place-items-center text-muted">{t('conn.connecting')}…</div>;

  return (
    <div className="flex flex-col h-full">
      <TopBar />
      <div className="flex-1 min-h-0">
        <Group orientation="horizontal" id="main-h">
          <Panel id="mw" defaultSize="21" minSize="14">
            <MarketWatch />
          </Panel>
          <Separator />
          <Panel id="center" minSize="35">
            <Group orientation="vertical" id="center-v">
              <Panel id="charts" defaultSize="68" minSize="25">
                <ChartGrid />
              </Panel>
              <Separator />
              <Panel id="toolbox" defaultSize="32" minSize="12">
                <Toolbox />
              </Panel>
            </Group>
          </Panel>
          <Separator />
          <Panel id="side" defaultSize="20" minSize="15">
            <Group orientation="vertical" id="side-v">
              <Panel id="ticket" defaultSize="58" minSize="25">
                <div className="h-full overflow-auto bg-panel">
                  <h2 className="px-2 h-8 flex items-center font-semibold border-b border-line">{t('ticket.title')}</h2>
                  <OrderTicket followChart />
                </div>
              </Panel>
              <Separator />
              <Panel id="dom" defaultSize="42" minSize="15">
                <DepthOfMarket />
              </Panel>
            </Group>
          </Panel>
        </Group>
      </div>
      <TicketDialog />
      <CommandPalette />
      <ShortcutsDialog />
      <Toasts />
    </div>
  );
}
