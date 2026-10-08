import { useState } from 'react';
import { useT } from '../hooks';
import { useTerminal } from '../store/terminal';
import { acceptReferral, dismissReferral, pendingReferral } from '../lib/referral';

/**
 * The client opened an IB's referral link: ask before attaching the account to
 * that IB (permanent) instead of linking it silently. Nothing is sent until
 * the client presses the button.
 */
export function ReferralBanner({ account }: { account: string }) {
  const t = useT();
  const toast = useTerminal((s) => s.toast);
  const [code, setCode] = useState(pendingReferral);
  const [busy, setBusy] = useState(false);
  if (!code) return null;
  const accept = async () => {
    setBusy(true);
    try {
      const r = await acceptReferral(account);
      toast('ok', t(r === 'already' ? 'ref.already' : 'ref.linked'));
    } catch (e) {
      toast('error', (e as Error).message);
    } finally {
      setBusy(false);
      setCode(pendingReferral());
    }
  };
  return (
    <div className="grid gap-2 border border-accent/60 rounded p-2" data-testid="ref-banner">
      <div>{t('ref.ask', { code, account })}</div>
      <div className="text-muted text-[11px]">{t('ref.permanent')}</div>
      <div className="flex gap-2">
        <button className="px-3 h-8 rounded bg-accent text-white disabled:opacity-40 text-[12px]" disabled={busy} onClick={() => void accept()} data-testid="ref-accept">
          {t('ref.accept')}
        </button>
        <button
          className="px-2 h-8 rounded border border-line hover:bg-hover disabled:opacity-40 text-[12px]"
          disabled={busy}
          onClick={() => {
            dismissReferral();
            setCode(null);
          }}
          data-testid="ref-dismiss"
        >
          {t('ref.dismiss')}
        </button>
      </div>
    </div>
  );
}
