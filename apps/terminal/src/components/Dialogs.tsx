import type { ReactNode } from 'react';
import { Command } from 'cmdk';
import { useT } from '../hooks';
import { useTerminal } from '../store/terminal';
import { getApi } from '../store/api';
import { SHORTCUTS } from '../shortcuts';
import { OrderTicket } from './OrderTicket';

export function Modal({ title, onClose, children, width = 'w-[420px]' }: { title: string; onClose: () => void; children: ReactNode; width?: string }) {
  return (
    <div className="fixed inset-0 z-50 grid place-items-start justify-center pt-[10vh] bg-black/40" onMouseDown={onClose}>
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className={`${width} max-w-[95vw] bg-panel border border-line rounded-lg shadow-2xl`}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-3 h-9 border-b border-line">
          <h2 className="font-semibold">{title}</h2>
          <button onClick={onClose} aria-label="Close" className="text-muted hover:text-fg">✕</button>
        </div>
        {children}
      </div>
    </div>
  );
}

export function TicketDialog() {
  const t = useT();
  const ticket = useTerminal((s) => s.ticket);
  const close = useTerminal((s) => s.closeTicket);
  if (!ticket) return null;
  return (
    <Modal title={`${t('ticket.title')} — ${ticket.symbol}`} onClose={close}>
      <OrderTicket key={`${ticket.symbol}-${ticket.side}-${ticket.type}`} preset={ticket} onDone={close} autoFocus />
    </Modal>
  );
}

export function ShortcutsDialog() {
  const t = useT();
  const open = useTerminal((s) => s.shortcutsOpen);
  const setOpen = useTerminal((s) => s.setShortcutsOpen);
  if (!open) return null;
  return (
    <Modal title={t('sc.title')} onClose={() => setOpen(false)}>
      <table className="w-full m-0 text-[12px]">
        <tbody>
          {SHORTCUTS.map((s) => (
            <tr key={s.keys} className="border-b border-line/50">
              <td className="px-3 py-1.5"><kbd className="num px-1.5 py-0.5 rounded bg-panel-2 border border-line">{s.keys}</kbd></td>
              <td className="px-3 py-1.5">{t(s.desc)}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </Modal>
  );
}

export function CommandPalette() {
  const t = useT();
  const s = useTerminal();
  if (!s.paletteOpen) return null;
  const run = (fn: () => void) => () => {
    fn();
    s.setPaletteOpen(false);
  };
  const item = 'px-3 py-1.5 rounded cursor-pointer flex justify-between';
  return (
    <div className="fixed inset-0 z-50 grid place-items-start justify-center pt-[12vh] bg-black/40" onMouseDown={() => s.setPaletteOpen(false)}>
      <div onMouseDown={(e) => e.stopPropagation()} className="w-[520px] max-w-[95vw]">
        <Command label={t('top.palette')} className="bg-panel border border-line rounded-lg shadow-2xl overflow-hidden">
          <Command.Input autoFocus placeholder={t('cmd.placeholder')} className="w-full px-3 h-11 bg-transparent border-b border-line outline-none text-[14px]" />
          <Command.List className="max-h-[50vh] overflow-auto p-1">
            <Command.Empty className="p-3 text-muted">{t('cmd.empty')}</Command.Empty>
            <Command.Group heading={t('cmd.actions')} className="text-muted [&_[cmdk-item]]:text-fg">
              <Command.Item className={item} onSelect={run(() => s.openTicket())}>{t('cmd.newOrder')}<kbd className="num text-muted">F9</kbd></Command.Item>
              <Command.Item className={item} onSelect={run(s.toggleTheme)}>{t('cmd.toggleTheme')}<kbd className="num text-muted">Alt+T</kbd></Command.Item>
              <Command.Item className={item} onSelect={run(() => s.setLayout(1))}>{t('cmd.layout1')}</Command.Item>
              <Command.Item className={item} onSelect={run(() => s.setLayout(2))}>{t('cmd.layout2')}</Command.Item>
              <Command.Item className={item} onSelect={run(() => s.setLayout(4))}>{t('cmd.layout4')}</Command.Item>
              <Command.Item className={item} onSelect={run(() => s.setLang(s.lang === 'en' ? 'tr' : 'en'))}>{t('cmd.lang')}</Command.Item>
              <Command.Item className={item} onSelect={run(() => s.setShortcutsOpen(true))}>{t('cmd.shortcuts')}<kbd className="num text-muted">?</kbd></Command.Item>
              <Command.Item
                className={item}
                onSelect={run(() => {
                  const acc = s.activeAccountId;
                  if (acc) (s.positions[acc] ?? []).forEach((p) => void getApi().closePosition(acc, p.id));
                })}
              >
                {t('cmd.closeAll')}
              </Command.Item>
            </Command.Group>
            <Command.Group heading={t('cmd.symbols')} className="text-muted [&_[cmdk-item]]:text-fg">
              {s.symbolOrder.map((sym) => (
                <Command.Item key={sym} value={`${sym} ${s.symbols[sym]?.description ?? ''}`} className={item} onSelect={run(() => s.setChartSymbol(sym))}>
                  <span>{sym}</span>
                  <span className="text-muted">{s.symbols[sym]?.description}</span>
                </Command.Item>
              ))}
            </Command.Group>
          </Command.List>
        </Command>
      </div>
    </div>
  );
}

export function Toasts() {
  const toasts = useTerminal((s) => s.toasts);
  const dismiss = useTerminal((s) => s.dismissToast);
  return (
    <div className="fixed bottom-3 right-3 z-50 flex flex-col gap-2" aria-live="polite">
      {toasts.map((x) => (
        <button
          key={x.id}
          onClick={() => dismiss(x.id)}
          className={`text-left px-3 py-2 rounded shadow-lg border bg-panel ${x.kind === 'ok' ? 'border-up/50' : 'border-down/50'}`}
          data-testid="toast"
        >
          <span className={x.kind === 'ok' ? 'text-up' : 'text-down'}>●</span> {x.text}
        </button>
      ))}
    </div>
  );
}
