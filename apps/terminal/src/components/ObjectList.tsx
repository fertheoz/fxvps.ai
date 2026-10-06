import { useT } from '../hooks';
import { useTerminal } from '../store/terminal';
import { formatPrice } from '@fxvps/trading-core';

/** Lines and alerts of the active account on `symbol`, with removal. */
export function ObjectList({ symbol }: { symbol: string }) {
  const t = useT();
  const accountId = useTerminal((s) => s.activeAccountId);
  const objects = useTerminal((s) => (s.activeAccountId ? s.objects[s.activeAccountId] : undefined));
  const setObjects = useTerminal((s) => s.setObjects);
  const digits = useTerminal((s) => s.symbols[symbol]?.digits ?? 5);
  const lines = (objects?.lines ?? []).filter((l) => l.symbol === symbol);
  const alerts = (objects?.alerts ?? []).filter((a) => a.symbol === symbol);
  const shapes = (objects?.shapes ?? []).filter((x) => x.symbol === symbol);
  if (!accountId || !objects) return null;
  const remove = (kind: 'line' | 'alert' | 'shape', id: string) =>
    setObjects(
      accountId,
      kind === 'line'
        ? { ...objects, lines: objects.lines.filter((l) => l.id !== id) }
        : kind === 'alert'
          ? { ...objects, alerts: objects.alerts.filter((a) => a.id !== id) }
          : { ...objects, shapes: (objects.shapes ?? []).filter((x) => x.id !== id) },
    );
  if (!lines.length && !alerts.length && !shapes.length) return <div className="px-3 py-2 text-muted">{t('obj.none')}</div>;
  const row = (key: string, text: string, muted: boolean, onRemove: () => void) => (
    <div key={key} className={`flex items-center justify-between gap-2 px-3 py-2 border-b border-line/50 ${muted ? 'text-muted' : ''}`}>
      <span className="num">{text}</span>
      <button className="px-2 h-7 rounded-full border border-line text-muted hover:text-fg" onClick={onRemove} data-testid={`obj-remove-${key}`}>
        {t('obj.remove')}
      </button>
    </div>
  );
  return (
    <div data-testid="object-list">
      {shapes.map((x) => row(x.id, `${x.kind === 'trend' ? '╱' : x.kind === 'rect' ? '▭' : '𝔽'} ${t(`obj.${x.kind}`)} ${formatPrice(x.a.price, digits)} → ${formatPrice(x.b.price, digits)}`, false, () => remove('shape', x.id)))}
      {lines.map((l) => row(l.id, `— ${t('obj.hline')} ${formatPrice(l.price, digits)}`, false, () => remove('line', l.id)))}
      {alerts.map((a) =>
        row(a.id, `🔔 ${t(a.direction === 'above' ? 'obj.above' : 'obj.below')} ${formatPrice(a.price, digits)}${a.firedAt ? ` · ${t('obj.fired')}` : ''}`, !!a.firedAt, () => remove('alert', a.id)),
      )}
    </div>
  );
}
