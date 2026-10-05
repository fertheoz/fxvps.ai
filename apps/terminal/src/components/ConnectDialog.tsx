import { useState } from 'react';
import { useT } from '../hooks';
import { defaultGateway, isValidWsUrl, loadGateway, saveGateway } from '../store/connection';
import { Modal } from './Dialogs';

/** Reload without query params so the stored choice (or the mock) is used. */
function reload(): void {
  location.assign(location.pathname);
}

/** Dev login: gateway URL + token, kept in sessionStorage for this tab. */
export function ConnectDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const saved = loadGateway();
  const [url, setUrl] = useState(saved?.url ?? defaultGateway()?.url ?? 'ws://localhost:8080/ws');
  const [token, setToken] = useState(saved?.token ?? '');
  const valid = isValidWsUrl(url.trim());
  const input = 'w-full bg-panel-2 border border-line rounded px-2 py-1.5 text-[12px] num';
  return (
    <Modal title={t('gw.title')} onClose={onClose}>
      <form
        className="p-3 flex flex-col gap-3"
        data-testid="gateway-form"
        onSubmit={(e) => {
          e.preventDefault();
          if (!valid) return;
          saveGateway({ url: url.trim(), token: token.trim() });
          reload();
        }}
      >
        <label className="flex flex-col gap-1">
          <span className="text-muted text-[11px]">{t('gw.url')}</span>
          <input className={input} value={url} onChange={(e) => setUrl(e.target.value)} data-testid="gateway-url" autoFocus spellCheck={false} />
          {!valid && <span className="text-down text-[11px]">{t('gw.invalidUrl')}</span>}
        </label>
        <label className="flex flex-col gap-1">
          <span className="text-muted text-[11px]">{t('gw.token')}</span>
          <textarea
            className={`${input} h-20 resize-none break-all`}
            value={token}
            onChange={(e) => setToken(e.target.value)}
            data-testid="gateway-token"
            spellCheck={false}
            autoComplete="off"
          />
        </label>
        <p className="text-muted text-[11px] m-0">{t('gw.hint')}</p>
        <div className="flex gap-2 justify-end">
          <button
            type="button"
            className="px-3 py-1.5 rounded border border-line text-muted hover:text-fg"
            data-testid="gateway-mock"
            onClick={() => {
              saveGateway(null);
              reload();
            }}
          >
            {t('gw.useMock')}
          </button>
          <button type="submit" disabled={!valid} className="px-3 py-1.5 rounded bg-accent text-white disabled:opacity-50" data-testid="gateway-connect">
            {t('gw.connect')}
          </button>
        </div>
      </form>
    </Modal>
  );
}
