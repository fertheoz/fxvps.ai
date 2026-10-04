import { Group, Panel, Separator } from 'react-resizable-panels';
import { useT } from '../hooks';
import { useTerminal, type ChartLayout, type Indicators } from '../store/terminal';
import { ChartPanel } from './ChartPanel';

const IND: { key: keyof Indicators; label: string }[] = [
  { key: 'sma', label: 'SMA 20' },
  { key: 'ema', label: 'EMA 50' },
  { key: 'bollinger', label: 'BB 20,2' },
  { key: 'rsi', label: 'RSI 14' },
  { key: 'volume', label: 'Vol' },
];

export function ChartGrid() {
  const t = useT();
  const layout = useTerminal((s) => s.layout);
  const setLayout = useTerminal((s) => s.setLayout);
  const indicators = useTerminal((s) => s.indicators);
  const toggle = useTerminal((s) => s.toggleIndicator);

  return (
    <section className="flex flex-col h-full">
      <div className="flex items-center gap-3 h-8 px-2 bg-panel border-b border-line shrink-0">
        <div className="flex items-center gap-1">
          <span className="text-muted">{t('chart.layout')}</span>
          {([1, 2, 4] as ChartLayout[]).map((l) => (
            <button
              key={l}
              aria-label={`${t('chart.layout')} ${l}`}
              className={`w-6 h-5 rounded text-[11px] ${layout === l ? 'bg-accent text-white' : 'border border-line text-muted hover:text-fg'}`}
              onClick={() => setLayout(l)}
            >
              {l}
            </button>
          ))}
        </div>
        <div className="flex items-center gap-1">
          <span className="text-muted">{t('chart.indicators')}</span>
          {IND.map((i) => (
            <button
              key={i.key}
              aria-pressed={indicators[i.key]}
              className={`px-1.5 h-5 rounded text-[11px] ${indicators[i.key] ? 'bg-accent/20 text-accent border border-accent/40' : 'border border-line text-muted hover:text-fg'}`}
              onClick={() => toggle(i.key)}
            >
              {i.label}
            </button>
          ))}
        </div>
      </div>
      <div className="flex-1 min-h-0">
        {layout === 1 && <ChartPanel index={0} />}
        {layout === 2 && (
          <Group orientation="horizontal">
            <Panel minSize="20"><ChartPanel index={0} /></Panel>
            <Separator />
            <Panel minSize="20"><ChartPanel index={1} /></Panel>
          </Group>
        )}
        {layout === 4 && (
          <Group orientation="vertical">
            <Panel minSize="20">
              <Group orientation="horizontal">
                <Panel minSize="20"><ChartPanel index={0} /></Panel>
                <Separator />
                <Panel minSize="20"><ChartPanel index={1} /></Panel>
              </Group>
            </Panel>
            <Separator />
            <Panel minSize="20">
              <Group orientation="horizontal">
                <Panel minSize="20"><ChartPanel index={2} /></Panel>
                <Separator />
                <Panel minSize="20"><ChartPanel index={3} /></Panel>
              </Group>
            </Panel>
          </Group>
        )}
      </div>
    </section>
  );
}
