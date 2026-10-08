import { useMemo, useState } from 'react';
import { useT } from '../hooks';
import { useTerminal } from '../store/terminal';
import { useEcon, useEconEvents, useNow } from '../store/econ';
import { IMPACT_COLOR, eventsFor, groupByDay, localDay, symbolCurrencies, type EconImpact } from '../lib/econCalendar';

const DAY = 86_400_000;
const CHART = '__chart__';

/** Toolbox "Calendar": economic releases by day, impact colour, currency filter (the active chart's pair or one currency). */
export function CalendarTab() {
  const t = useT();
  const lang = useTerminal((s) => s.lang);
  const events = useEconEvents();
  const now = useNow();
  const error = useEcon((s) => s.error);
  const loadedAt = useEcon((s) => s.loadedAt);
  const chartSpec = useTerminal((s) => {
    const slot = s.charts[s.activeChart];
    return slot ? s.symbols[slot.symbol] : undefined;
  });
  const [ccy, setCcy] = useState('');
  const [minImpact, setMinImpact] = useState<EconImpact>('low');
  // from the start of yesterday (local) on: recent results stay visible next to what is coming
  const [since] = useState(() => {
    const d = new Date(Date.now() - DAY);
    d.setHours(0, 0, 0, 0);
    return d.getTime();
  });
  const currencies = useMemo(() => [...new Set(events.map((e) => e.currency))].sort(), [events]);
  const filter = ccy === CHART ? (chartSpec ? symbolCurrencies(chartSpec) : []) : ccy ? [ccy] : currencies;
  const shown = eventsFor(events, filter, minImpact).filter((e) => e.time >= since);
  const days = groupByDay(shown);
  const today = localDay(now);
  const locale = lang === 'tr' ? 'tr-TR' : 'en-US';
  const dayFmt = new Intl.DateTimeFormat(locale, { weekday: 'long', day: 'numeric', month: 'long' });
  const timeFmt = new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' });
  const sel = 'bg-panel-2 border border-line rounded px-1 h-6 text-[12px]';
  return (
    <div className="h-full flex flex-col text-[12px]" data-testid="calendar-tab">
      <div className="flex flex-wrap items-center gap-2 px-2 h-8 border-b border-line shrink-0">
        <select className={sel} value={ccy} onChange={(e) => setCcy(e.target.value)} aria-label={t('cal.currency')} data-testid="calendar-currency">
          <option value="">{t('cal.allCurrencies')}</option>
          {chartSpec && <option value={CHART}>{t('cal.chartPair', { symbol: chartSpec.name })}</option>}
          {currencies.map((c) => (
            <option key={c} value={c}>
              {c}
            </option>
          ))}
        </select>
        <select className={sel} value={minImpact} onChange={(e) => setMinImpact(e.target.value as EconImpact)} aria-label={t('cal.impact')} data-testid="calendar-impact">
          <option value="low">{t('cal.impactAll')}</option>
          <option value="medium">{t('cal.impactMedium')}</option>
          <option value="high">{t('cal.impactHigh')}</option>
        </select>
        {error && <span className="text-down truncate" title={error}>{t('cal.error')}</span>}
      </div>
      <div className="grid grid-cols-[40px_10px_48px_minmax(120px,1fr)_64px_64px_64px] gap-2 px-2 h-5 items-center border-b border-line text-[10px] text-muted shrink-0">
        <span>{t('cal.time')}</span>
        <span />
        <span>{t('cal.currency')}</span>
        <span>{t('cal.event')}</span>
        <span className="text-right">{t('cal.actual')}</span>
        <span className="text-right">{t('cal.forecast')}</span>
        <span className="text-right">{t('cal.previous')}</span>
      </div>
      <div className="flex-1 min-h-0 overflow-auto">
        {loadedAt > 0 && days.length === 0 && <div className="p-3 text-muted">{t('cal.none')}</div>}
        {days.map(([day, list]) => (
          <div key={day}>
            <div className={`sticky top-0 z-[1] px-2 h-6 flex items-center bg-panel-2 border-b border-line text-[11px] font-semibold ${day === today ? 'text-accent' : 'text-muted'}`} data-testid={`calendar-day-${day}`}>
              {dayFmt.format(new Date(list[0]!.time))}
              {day === today && <span className="ml-2 font-normal">· {t('cal.today')}</span>}
            </div>
            {list.map((e) => (
              <div
                key={e.id}
                className={`grid grid-cols-[40px_10px_48px_minmax(120px,1fr)_64px_64px_64px] items-center gap-2 px-2 h-6 border-b border-line/50 hover:bg-hover ${e.time < now ? 'text-muted' : ''}`}
                data-testid={`calendar-event-${e.id}`}
                data-impact={e.impact}
              >
                <span className="num">{timeFmt.format(new Date(e.time))}</span>
                <span className="w-2 h-2 rounded-full" style={{ background: IMPACT_COLOR[e.impact] }} title={t(`cal.impact.${e.impact}`)} aria-label={t(`cal.impact.${e.impact}`)} />
                <span className="font-semibold">{e.currency}</span>
                <span className="truncate" title={e.title}>
                  {e.title}
                </span>
                <span className="num text-right text-fg" title={t('cal.actual')}>
                  {e.actual ?? ''}
                </span>
                <span className="num text-right" title={t('cal.forecast')}>
                  {e.forecast ?? ''}
                </span>
                <span className="num text-right text-muted" title={t('cal.previous')}>
                  {e.previous ?? ''}
                </span>
              </div>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}
