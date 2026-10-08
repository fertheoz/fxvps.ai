import { clientApi } from '../api/clientApi';

const REF_KEY = 'fxvps.ref';

/** Remembers `?ref=CODE` from the landing URL until the client has an account. */
export function captureReferral(): void {
  try {
    const code = new URLSearchParams(window.location.search).get('ref');
    if (code && /^[A-Za-z0-9-]{3,20}$/.test(code)) localStorage.setItem(REF_KEY, code.toUpperCase());
  } catch {
    /* storage unavailable */
  }
}

/** Links the account to the remembered IB once; the server ignores repeats. */
export async function sendPendingReferral(account: string): Promise<void> {
  let code: string | null;
  try {
    code = localStorage.getItem(REF_KEY);
  } catch {
    return;
  }
  if (!code) return;
  try {
    await clientApi.ibLink(account, code);
    localStorage.removeItem(REF_KEY);
  } catch (e) {
    // unknown code: forget it; anything else: try again next time
    if ((e as Error).message.includes('referral')) {
      try {
        localStorage.removeItem(REF_KEY);
      } catch {
        /* ignore */
      }
    }
  }
}
