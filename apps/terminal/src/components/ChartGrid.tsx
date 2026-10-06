import { useState } from 'react';
import { Group, Panel, Separator } from 'react-resizable-panels';
import { ObjectList } from './ObjectList';
import { IndicatorSettings } from './IndicatorSettings';
import { useT } from '../hooks';
import { useTerminal, type ChartLayout, type Indicators } from '../store/terminal';
import { ChartPanel } from './ChartPanel';

const IND: (keyof Indicators)[] = ['sma', 'ema', 'bollinger', 'rsi', 'volume'];

export function ChartGrid() {
  const t = useT();
  const layout = useTerminal((s) => s.layout);
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
