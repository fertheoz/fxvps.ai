/** Optional Market Watch columns (MT5 / cTrader set); the first four are the default. */
export type MwColumnId = 'bid' | 'ask' | 'spread' | 'change' | 'changePts' | 'time' | 'open' | 'high' | 'low' | 'myLots';

export interface MwColumnDef {
  id: MwColumnId;
  /** CSS grid track. */
  width: string;
  /** i18n key of the header. */
  label: `mw.col.${MwColumnId}`;
}

export const MW_COLUMNS: MwColumnDef[] = [
  { id: 'bid', width: '62px', label: 'mw.col.bid' },
  { id: 'ask', width: '62px', label: 'mw.col.ask' },
  { id: 'spread', width: '30px', label: 'mw.col.spread' },
  { id: 'change', width: '44px', label: 'mw.col.change' },
  { id: 'changePts', width: '48px', label: 'mw.col.changePts' },
  { id: 'time', width: '58px', label: 'mw.col.time' },
  { id: 'open', width: '74px', label: 'mw.col.open' },
  { id: 'high', width: '74px', label: 'mw.col.high' },
  { id: 'low', width: '74px', label: 'mw.col.low' },
  { id: 'myLots', width: '48px', label: 'mw.col.myLots' },
];

export const DEFAULT_MW_COLUMNS: MwColumnId[] = ['bid', 'ask', 'spread', 'change'];

/** Grid template for the star, the symbol and the chosen columns, in catalogue order. */
export function mwGridTemplate(visible: MwColumnId[]): string {
  const tracks = MW_COLUMNS.filter((c) => visible.includes(c.id)).map((c) => c.width);
  // symbol column sized to its content (≈ 7 chars), not to the leftover width
  return `14px minmax(48px, 60px) ${tracks.join(' ')}`;
}
