import { useState } from 'react';
import { Group, Panel, Separator } from 'react-resizable-panels';
import { ObjectList } from './ObjectList';
import { IndicatorSettings } from './IndicatorSettings';
import { useT } from '../hooks';
import { Fragment } from 'react';
import { useTerminal, type ChartLayout, type Indicators } from '../store/terminal';
import { ChartPanel } from './ChartPanel';

const IND: (keyof Indicators)[] = ['sma', 'ema', 'bollinger', 'rsi', 'volume'];

export function ChartGrid() {
  const t = useT();
  const layout = useTerminal((s) => s.layout);
  const hidden = useTerminal((s) => s.hiddenCharts);
  const charts = useTerminal((s) => s.charts);
  const restoreChart = useTerminal((s) => s.restoreChart);
  const hideChart = useTerminal((s) => s.hideChart);
  const maximized = useTerminal((s) => s.maximizedChart);
  // Slots still in the grid; the rest sit in the toolbar as chips (minimised) or are gone (closed).
  const visible = Array.from({ length: layout }, (_, i) => i).filter((i) => !hidden[i]);
  const minimized = Object.entries(hidden)
    .filter(([i, how]) => how === 'min' && Number(i) < layout)
    .map(([i]) => Number(i));
  // A row of up to three charts side by side.
  const row = (ids: number[]) =>
    ids.length === 1 ? (
      <ChartPanel index={ids[0]!} />
    ) : (
      <Group orientation="horizontal">
        {ids.map((i, k) => (
          <Fragment key={i}>
            {k > 0 && <Separator />}
            <Panel minSize="15"><ChartPanel index={i} /></Panel>
          </Fragment>
        ))}
      </Group>
    );
  const grid = () => {
    // one chart maximised over the whole chart area (title bar □ / double-click)
    if (maximized !== null && visible.includes(maximized)) return <ChartPanel index={maximized} />;
    if (!visible.length) return <div className="h-full grid place-items-center text-muted text-[12px]">{t('chart.allHidden')}</div>;
    if (visible.length <= 2) return row(visible);
    // 3–4 charts: two rows of two; 5–6: two rows of three.
    const perRow = visible.length > 4 ? 3 : 2;
    const rows = [visible.slice(0, perRow), visible.slice(perRow)];
    return (
      <Group orientation="vertical">
        <Panel minSize="20">{row(rows[0]!)}</Panel>
        <Separator />
        <Panel minSize="20">{row(rows[1]!)}</Panel>
      </Group>
    );
  };
  const setLayout = useTerminal((s) => s.setLayout);
  const indicators = useTerminal((s) => s.indicators);
  const toggle = useTerminal((s) => s.toggleIndicator);
  const cfg = useTerminal((s) => s.indicatorSettings);
  const [indOpen, setIndOpen] = useState(false);
  const label = (k: keyof Indicators) =>
    k === 'sma' ? `SMA ${cfg.sma.period}` : k === 'ema' ? `EMA ${cfg.ema.period}` : k === 'bollinger' ? `BB ${cfg.bollinger.period},${cfg.bollinger.dev}` : k === 'rsi' ? `RSI ${cfg.rsi.period}` : 'Vol';
  const askLine = useTerminal((s) => s.showAskLine);
  const toggleAsk = useTerminal((s) => s.toggleAskLine);
  const tool = useTerminal((s) => s.chartTool);
  const setTool = useTerminal((s) => s.setChartTool);
  const activeSymbol = useTerminal((s) => s.charts[s.activeChart]?.symbol ?? '');
  const [listOpen, setListOpen] = useState(false);
  const toolBtn = (kind: 'hline' | 'alert' | 'trend' | 'rect' | 'fib', label: string, icon: string) => (
    // Icon only: the bar has to fit a 1280px desktop next to the indicator toggles; the name is the tooltip.
    <button
      aria-pressed={tool === kind}
      aria-label={label}
      title={label}
      className={`w-6 h-5 rounded text-[12px] leading-none ${tool === kind ? 'bg-accent text-white' : 'border border-line text-muted hover:text-fg'}`}
      onClick={() => setTool(tool === kind ? null : kind)}
      data-testid={`tool-${kind}`}
    >
      {icon}
    </button>
  );

  return (
    <section className="flex flex-col h-full">
      <div className="flex items-center gap-3 h-8 px-2 bg-panel border-b border-line shrink-0 overflow-hidden whitespace-nowrap">
        <div className="flex items-center gap-1">
          <span className="text-muted">{t('chart.layout')}</span>
          {([1, 2, 4, 6] as ChartLayout[]).map((l) => (
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
        <div className="relative flex items-center gap-1" title={t('obj.tools')}>
          {toolBtn('hline', t('obj.hline'), '—')}
          {toolBtn('alert', t('obj.alert'), '🔔')}
          {toolBtn('trend', t('obj.trend'), '╱')}
          {toolBtn('rect', t('obj.rect'), '▭')}
          {toolBtn('fib', t('obj.fib'), '𝔽')}
          <button
            className="w-6 h-5 rounded text-[12px] leading-none border border-line text-muted hover:text-fg"
            onClick={() => setListOpen((o) => !o)}
            aria-expanded={listOpen}
            aria-label={t('obj.objects')}
            title={t('obj.objects')}
            data-testid="tool-list"
          >
            ≡
          </button>
          {listOpen && (
            <div className="absolute top-6 left-0 z-30 w-[320px] rounded-md border border-line bg-panel shadow-xl text-[12px]">
              <div className="px-3 py-1.5 border-b border-line font-semibold">
                {t('obj.objects')} · {activeSymbol}
              </div>
              <ObjectList symbol={activeSymbol} />
            </div>
          )}
        </div>
        <div className="relative flex items-center gap-1">
          <span className="text-muted">{t('chart.indicators')}</span>
          <button
            className="px-1.5 h-5 rounded text-[11px] border border-line text-muted hover:text-fg"
            onClick={() => setIndOpen((o) => !o)}
            aria-expanded={indOpen}
            title={t('ind.title')}
            data-testid="ind-settings"
          >
            ⚙
          </button>
          {indOpen && (
            <div className="absolute top-6 left-0 z-30 w-[360px] rounded-md border border-line bg-panel shadow-xl p-3">
              <IndicatorSettings />
            </div>
          )}
          <button
            aria-pressed={askLine}
            className={`px-1.5 h-5 rounded text-[11px] ${askLine ? 'bg-accent/20 text-accent border border-accent/40' : 'border border-line text-muted hover:text-fg'}`}
            onClick={toggleAsk}
            data-testid="toggle-ask-line"
          >
            {t('chart.askLine')}
          </button>
          {IND.map((k) => (
            <button
              key={k}
              aria-pressed={indicators[k]}
              className={`px-1.5 h-5 rounded text-[11px] ${indicators[k] ? 'bg-accent/20 text-accent border border-accent/40' : 'border border-line text-muted hover:text-fg'}`}
              onClick={() => toggle(k)}
            >
              {label(k)}
            </button>
          ))}
        </div>
      </div>
      <div className="flex-1 min-h-0">{grid()}</div>
      {minimized.length > 0 && (
        // MetaTrader-style strip of minimised charts under the chart area
        <div className="flex items-center gap-1 h-7 px-2 border-t border-line bg-panel shrink-0 overflow-x-auto whitespace-nowrap" data-testid="minimized-strip">
          {minimized.map((i) => (
            <span key={`min-${i}`} className="flex items-center h-5 rounded border border-line bg-panel-2 text-[11px]">
              <button
                className="px-2 h-full text-muted hover:text-fg"
                title={t('chart.restore')}
                onClick={() => restoreChart(i)}
                data-testid={`chart-restore-${i}`}
              >
                ▭ {charts[i]?.symbol},{charts[i]?.timeframe}
              </button>
              <button
                className="px-1 h-full text-muted hover:text-down border-l border-line"
                title={t('chart.close')}
                aria-label={t('chart.close')}
                onClick={() => hideChart(i, 'closed')}
              >
                ×
              </button>
            </span>
          ))}
        </div>
      )}
    </section>
  );
}
