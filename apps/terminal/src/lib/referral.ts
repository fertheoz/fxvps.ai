import { clientApi, ClientApiError } from '../api/clientApi';

const REF_KEY = 'fxvps.ref';

/** Remembers `?ref=CODE` from the landing URL until the client decides on it. */
export function captureReferral(): void {
  try {
    const code = new URLSearchParams(window.location.search).get('ref');
    if (code && /^[A-Za-z0-9-]{3,20}$/.test(code)) localStorage.setItem(REF_KEY, code.toUpperCase());
  } catch {
    /* storage unavailable */
  }
}

/** The remembered referral code waiting for the client's answer, if any. */
export function pendingReferral(): string | null {
  try {
    return localStorage.getItem(REF_KEY);
  } catch {
    return null;
  }
}

/** The client said no: forget the code. */
export function dismissReferral(): void {
  try {
    localStorage.removeItem(REF_KEY);
  } catch {
    /* storage unavailable */
  }
}

/**
 * Links `account` to the remembered IB. Called only from the client's explicit
 * confirmation (the link is permanent). The code is forgotten once the server
 * has decided (linked, already linked or refused) and kept when there was no
 * answer or the session expired, so the client can try again.
 */
export async function acceptReferral(account: string): Promise<'linked' | 'already'> {
  const code = pendingReferral();
  if (!code) throw new Error('no referral code');
  try {
    const r = await clientApi.ibLink(account, code);
    dismissReferral();
    return r.already ? 'already' : 'linked';
  } catch (e) {
    if (e instanceof ClientApiError && e.status >= 400 && e.status < 500 && e.status !== 401) dismissReferral();
    throw e;
  }
}
