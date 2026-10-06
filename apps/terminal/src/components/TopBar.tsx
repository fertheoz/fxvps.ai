import { useMetrics, useT } from '../hooks';
import { useTerminal } from '../store/terminal';
import { formatMoney } from '@fxvps/trading-core';
import type { ConnectionState } from '@fxvps/trading-core';
import { useState } from 'react';
import { isGatewayApi } from '../store/api';
import { defaultGateway, loadGateway } from '../store/connection';
import { UserMenu } from './Auth';
import { ConnectDialog } from './ConnectDialog';
import { promptInstall, useInstallPrompt } from '../lib/install';

const connColor: Record<ConnectionState, string> = {
  connected: 'bg-up',
  connecting: 'bg-amber-500',
  reconnecting: 'bg-amber-500',
  disconnected: 'bg-down',
};

function Stat({ label, value, tone, testId }: { label: string; value: string; tone?: 'up' | 'down'; testId?: string }) {
  return (
    <div className="flex flex-col leading-tight px-3 border-l border-line first:border-l-0">
      <span className="text-[10px] uppercase tracking-wide text-muted">{label}</span>
      <span data-testid={testId} className={`num text-[13px] ${tone === 'up' ? 'text-up' : tone === 'down' ? 'text-down' : ''}`}>
        {value}
      </span>
    </div>
  );
}

export function TopBar() {
  const t = useT();
  const accounts = useTerminal((s) => s.accounts);
  const activeId = useTerminal((s) => s.activeAccountId);
  const setActive = useTerminal((s) => s.setActiveAccount);
  const conn = useTerminal((s) => s.connection);
  const latency = useTerminal((s) => s.latencyMs);
  const theme = useTerminal((s) => s.theme);
  const toggleTheme = useTerminal((s) => s.toggleTheme);
  const lang = useTerminal((s) => s.lang);
  const setLang = useTerminal((s) => s.setLang);
  const setPalette = useTerminal((s) => s.setPaletteOpen);
  const setShortcuts = useTerminal((s) => s.setShortcutsOpen);
  const setAccountOpen = useTerminal((s) => s.setAccountOpen);
  const m = useMetrics();
  const [gwOpen, setGwOpen] = useState(false);
  const installable = useInstallPrompt();
  // The build-time default gateway is never written to sessionStorage.
  const gateway = isGatewayApi() ? (loadGateway() ?? defaultGateway()) : null;
  const active = accounts.find((a) => a.id === activeId);
  const cur = active?.currency ?? 'USD';

  return (
    <header className="flex items-center gap-3 h-12 px-3 bg-panel border-b border-line shrink-0">
      <div className="flex items-center gap-2 font-semibold text-[14px] tracking-tight">
        <span className="inline-block w-5 h-5 rounded bg-accent text-white text-[11px] grid place-items-center">fx</span>
        <span>fxvps.ai</span>
      </div>

      <label className="flex items-center gap-2 ml-2">
        <span className="sr-only">{t('top.account')}</span>
        <select
          aria-label={t('top.account')}
          className="bg-panel-2 border border-line rounded px-2 py-1 text-[12px]"
          value={activeId ?? ''}
          onChange={(e) => setActive(e.target.value)}
        >
          {accounts.map((a) => (
            <option key={a.id} value={a.id}>
              {a.parentId ? '↳ ' : ''}
              {a.id} · {a.name} · 1:{a.leverage}
              {a.marginMode ? ` · ${t(a.marginMode === 'hedging' ? 'top.hedging' : 'top.netting')}` : ''}
              {a.parentId ? ` (${t('top.sub')})` : ''}
            </option>
          ))}
        </select>
        {active?.isDemo && (
          <span className="text-[10px] font-semibold px-1.5 py-0.5 rounded bg-amber-500/15 text-amber-500">{t('top.demo')}</span>
        )}
      </label>

      <div className="flex items-center ml-2 overflow-x-auto">
        {m && (
          <>
            <Stat label={`${t('top.balance')} ${cur}`} value={formatMoney(m.balance)} testId="balance" />
            <Stat label={t('top.equity')} value={formatMoney(m.equity)} testId="equity" />
            <Stat
              label={t('top.floating')}
              value={formatMoney(m.floating)}
              tone={m.floating > 0 ? 'up' : m.floating < 0 ? 'down' : undefined}
            />
            <Stat label={t('top.margin')} value={formatMoney(m.margin)} testId="margin" />
            <Stat label={t('top.freeMargin')} value={formatMoney(m.freeMargin)} testId="free-margin" />
            <Stat
              label={t('top.marginLevel')}
              value={m.marginLevel ? `${m.marginLevel}%` : '—'}
              tone={m.marginLevel && Number(m.marginLevel) < 150 ? 'down' : undefined}
            />
          </>
        )}
      </div>

      <div className="ml-auto flex items-center gap-2">
        {installable && (
          <button
            className="flex items-center gap-1.5 px-2 py-1 rounded border border-accent/50 text-accent hover:bg-accent/10"
            onClick={() => void promptInstall()}
            title={t('top.install')}
            data-testid="install-app"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
              <path d="M12 3v12m0 0 4-4m-4 4-4-4M4 17v3h16v-3" />
            </svg>
            {t('top.install')}
          </button>
        )}
        <button
          className="px-2 py-1 rounded border border-line text-muted hover:text-fg"
          onClick={() => setAccountOpen(true)}
          title={t('acct.title')}
          data-testid="account-dialog-button"
        >
          {t('acct.short')}
        </button>
        <UserMenu />
        <button
          className="px-2 py-1 rounded border border-line text-muted hover:text-fg max-w-[220px] truncate"
          onClick={() => setGwOpen(true)}
          title={gateway?.url ?? t('gw.mock')}
          data-testid="gateway-button"
          data-api={gateway ? 'ws' : 'mock'}
        >
          {t('gw.button')}: {gateway ? new URL(gateway.url).host : t('gw.mock')}
        </button>
        {gwOpen && <ConnectDialog onClose={() => setGwOpen(false)} />}
        <button
          className="flex items-center gap-2 px-2 py-1 rounded border border-line text-muted hover:text-fg"
          onClick={() => setPalette(true)}
          title={t('top.palette')}
        >
          <span>{t('top.palette')}</span>
          <kbd className="num text-[10px] px-1 rounded bg-panel-2 border border-line">⌘K</kbd>
        </button>
        <button className="px-2 py-1 rounded border border-line text-muted hover:text-fg" onClick={() => setShortcuts(true)} title={t('top.shortcuts')}>
          ?
        </button>
        <button
          className="px-2 py-1 rounded border border-line text-muted hover:text-fg num"
          onClick={() => setLang(lang === 'en' ? 'tr' : 'en')}
          title={t('top.lang')}
          aria-label={t('top.lang')}
        >
          {lang.toUpperCase()}
        </button>
        <button
          className="px-2 py-1 rounded border border-line text-muted hover:text-fg"
          onClick={toggleTheme}
          title={t('top.theme')}
          aria-label={t('top.theme')}
          data-testid="theme-toggle"
        >
          {theme === 'dark' ? '☾' : '☀'}
        </button>
        <div className="flex items-center gap-1.5 pl-2" data-testid="connection" data-state={conn}>
          <span className={`w-2 h-2 rounded-full ${connColor[conn]}`} />
          <span className="text-muted">{t(`conn.${conn}`)}</span>
          {conn === 'connected' && latency !== undefined && <span className="num text-muted">{latency}ms</span>}
        </div>
      </div>
    </header>
  );
}
