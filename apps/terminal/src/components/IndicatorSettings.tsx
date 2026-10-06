import { useState } from 'react';
import type { ChartTemplate } from '@fxvps/trading-core';
import { useT } from '../hooks';
import { DEFAULT_INDICATOR_SETTINGS, useTerminal, type IndicatorSettings as Settings, type Indicators } from '../store/terminal';

const NO_TEMPLATES: ChartTemplate[] = [];
const KEYS: (keyof Settings)[] = ['sma', 'ema', 'bollinger', 'rsi'];
const LABEL: Record<keyof Settings, string> = { sma: 'SMA', ema: 'EMA', bollinger: 'Bollinger', rsi: 'RSI' };

/**
 * Indicator switches with their parameters (period, deviation, colour) and the
 * account's chart templates. `compact` = phone sheet (larger controls).
 */
export function IndicatorSettings({ compact = false }: { compact?: boolean }) {
  const t = useT();
  const indicators = useTerminal((s) => s.indicators);
  const toggle = useTerminal((s) => s.toggleIndicator);
  const settings = useTerminal((s) => s.indicatorSettings);
  const setSettings = useTerminal((s) => s.setIndicatorSettings);
  const accountId = useTerminal((s) => s.activeAccountId);
  // A stable fallback: a fresh [] per call would make the store selector loop.
  const templates = useTerminal((s) => (s.activeAccountId ? s.objects[s.activeAccountId]?.templates : undefined)) ?? NO_TEMPLATES;
  const saveTemplate = useTerminal((s) => s.saveTemplate);
  const applyTemplate = useTerminal((s) => s.applyTemplate);
  const deleteTemplate = useTerminal((s) => s.deleteTemplate);
  const [name, setName] = useState('');
  const h = compact ? 'h-10' : 'h-7';
  const num = `num ${h} w-16 rounded-lg bg-panel-2 border border-line/60 px-2 text-right outline-none focus:border-accent`;
  const btn = `${h} px-3 rounded-lg border border-line text-muted hover:text-fg`;
  const set = (k: keyof Settings, patch: Partial<Settings[keyof Settings]>) => setSettings({ [k]: { ...settings[k], ...patch } } as Partial<Settings>);
  const clamp = (v: string, min: number, max: number, fallback: number) => {
    const n = Number(v);
    return Number.isFinite(n) && n >= min && n <= max ? Math.round(n) : fallback;
  };
  return (
    <div className={`flex flex-col ${compact ? 'gap-3 text-[14px]' : 'gap-2 text-[12px]'}`} data-testid="indicator-settings">
      {KEYS.map((k) => {
        const s = settings[k];
        const on = indicators[k as keyof Indicators];
        return (
          <div key={k} className="flex items-center gap-2">
            <label className="flex items-center gap-2 w-24">
              <input type="checkbox" checked={on} onChange={() => toggle(k as keyof Indicators)} aria-label={LABEL[k]} data-testid={`ind-on-${k}`} />
              <span className="font-medium">{LABEL[k]}</span>
            </label>
            <input
              className={num}
              inputMode="numeric"
              value={s.period}
              onChange={(e) => set(k, { period: clamp(e.target.value, 1, 500, s.period) })}
              aria-label={`${LABEL[k]} ${t('ind.period')}`}
              data-testid={`ind-period-${k}`}
              title={t('ind.period')}
            />
            {k === 'bollinger' && (
              <input
                className={num}
                inputMode="decimal"
                value={(s as Settings['bollinger']).dev}
                onChange={(e) => {
                  const n = Number(e.target.value);
                  if (Number.isFinite(n) && n > 0 && n <= 5) set(k, { dev: n } as Partial<Settings['bollinger']>);
                }}
                aria-label={`${LABEL[k]} ${t('ind.dev')}`}
                title={t('ind.dev')}
              />
            )}
            <input
              type="color"
              value={s.color}
              onChange={(e) => set(k, { color: e.target.value })}
              aria-label={`${LABEL[k]} ${t('ind.color')}`}
              className={`${h} w-9 rounded-lg bg-transparent border border-line/60 p-0.5`}
            />
          </div>
        );
      })}
      <label className="flex items-center gap-2">
        <input type="checkbox" checked={indicators.volume} onChange={() => toggle('volume')} aria-label={t('chart.volume')} />
        <span className="font-medium">{t('chart.volume')}</span>
      </label>
      <button className={`${btn} self-start`} onClick={() => setSettings(DEFAULT_INDICATOR_SETTINGS)}>
        {t('ind.reset')}
      </button>

      <div className={`border-t border-line/50 pt-2 ${compact ? 'mt-1' : ''}`}>
        <div className="text-[11px] uppercase tracking-wider text-muted mb-1">{t('ind.templates')}</div>
        {!accountId && <div className="text-muted">{t('ind.noAccount')}</div>}
        {templates.map((tpl) => (
          <div key={tpl.id} className="flex items-center gap-2 py-1" data-testid={`tpl-${tpl.id}`}>
            <span className="flex-1 truncate">{tpl.name}</span>
            <button className={btn} onClick={() => applyTemplate(tpl.id)}>
              {t('ind.apply')}
            </button>
            <button className={`${btn} text-down`} onClick={() => deleteTemplate(tpl.id)} aria-label={`${t('obj.remove')} ${tpl.name}`}>
              ✕
            </button>
          </div>
        ))}
        {accountId && (
          <form
            className="flex items-center gap-2 pt-1"
            onSubmit={(e) => {
              e.preventDefault();
              if (name.trim()) {
                saveTemplate(name.trim());
                setName('');
              }
            }}
          >
            <input className={`${h} flex-1 min-w-0 rounded-lg bg-panel-2 border border-line/60 px-2 outline-none focus:border-accent`} value={name} onChange={(e) => setName(e.target.value)} placeholder={t('ind.templateName')} aria-label={t('ind.templateName')} data-testid="tpl-name" />
            <button type="submit" className={`${h} px-3 rounded-lg bg-accent text-white font-medium disabled:opacity-40`} disabled={!name.trim()} data-testid="tpl-save">
              {t('ind.save')}
            </button>
          </form>
        )}
      </div>
    </div>
  );
}
