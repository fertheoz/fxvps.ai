import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ACCOUNT_KEY, decodeJwt, sessionToken, useSession } from './session';
import { bufToB64url, b64urlToBuf, CSRF_HEADER, IdentityClient, IdentityError, isMfa } from '../auth/identity';

function b64url(s: string): string {
  return btoa(String.fromCharCode(...new TextEncoder().encode(s))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function jwt(claims: Record<string, unknown>): string {
  return `${b64url('{"alg":"RS256","kid":"k1"}')}.${b64url(JSON.stringify(claims))}.sig`;
}

function tokenResp(accounts: string[], ttl = 300, sub = 'u1') {
  const now = Math.floor(Date.now() / 1000);
  return {
    access_token: jwt({ sub, email: 'a@example.com', exp: now + ttl, iat: now, accounts, roles: ['client'], amr: ['pwd'] }),
    token_type: 'Bearer' as const,
    expires_in: ttl,
    refresh_expires_in: 3600,
    accounts,
    roles: ['client'],
  };
}

type Call = { url: string; init: RequestInit };

function fakeFetch(handler: (url: string, init: RequestInit) => { status: number; body?: unknown }) {
  const calls: Call[] = [];
  const f = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    calls.push({ url, init: init ?? {} });
    const r = handler(url, init ?? {});
    return new Response(r.body === undefined ? null : JSON.stringify(r.body), { status: r.status });
  });
  return { f: f as unknown as typeof fetch, calls };
}

beforeEach(() => {
  sessionStorage.clear();
  vi.useFakeTimers();
});
afterEach(() => {
  useSession.getState().configure(null);
  vi.useRealTimers();
});

describe('decodeJwt', () => {
  it('reads claims and rejects garbage', () => {
    const c = decodeJwt(jwt({ sub: 'u', exp: 10, accounts: ['A'], roles: ['admin'], amr: ['pwd', 'otp'], email: 'ü@x.io' }));
    expect(c).toEqual({ sub: 'u', exp: 10, iat: undefined, accounts: ['A'], roles: ['admin'], amr: ['pwd', 'otp'], email: 'ü@x.io' });
    expect(decodeJwt('nope')).toBeNull();
    expect(decodeJwt(`a.${b64url('{"exp":1}')}.c`)).toBeNull();
  });
});

describe('base64url', () => {
  it('round-trips', () => {
    const b = new Uint8Array([0, 255, 62, 63, 1, 2, 3]);
    expect(new Uint8Array(b64urlToBuf(bufToB64url(b)))).toEqual(b);
    expect(bufToB64url(new Uint8Array([251, 255]))).toBe('-_8');
  });
});

describe('session store', () => {
  it('restores from the refresh cookie and auto-selects a single account', async () => {
    const { f, calls } = fakeFetch(() => ({ status: 200, body: tokenResp(['ACC-1']) }));
    useSession.getState().configure('http://id.test/', f);
    expect(await useSession.getState().restore()).toBe(true);
    const s = useSession.getState();
    expect(s.status).toBe('authenticated');
    expect(s.claims?.accounts).toEqual(['ACC-1']);
    expect(s.account).toBe('ACC-1');
    expect(sessionToken()).toBe(s.accessToken);
    expect(calls[0]!.url).toBe('http://id.test/v1/token/refresh');
    expect(calls[0]!.init.credentials).toBe('include');
    expect((calls[0]!.init.headers as Record<string, string>)[CSRF_HEADER]).toBe('1');
  });

  it('stays anonymous when there is no session', async () => {
    const { f } = fakeFetch(() => ({ status: 401, body: { error: 'unauthenticated', message: 'x' } }));
    useSession.getState().configure('http://id.test', f);
    expect(await useSession.getState().restore()).toBe(false);
    expect(useSession.getState().status).toBe('anonymous');
    expect(useSession.getState().error).toBeNull();
    expect(sessionToken()).toBeUndefined();
  });

  it('requires a pick with several accounts and remembers it', async () => {
    const { f } = fakeFetch(() => ({ status: 200, body: tokenResp(['A', 'B']) }));
    useSession.getState().configure('http://id.test', f);
    await useSession.getState().restore();
    expect(useSession.getState().account).toBeNull();
    useSession.getState().selectAccount('B');
    expect(sessionStorage.getItem(ACCOUNT_KEY)).toBe('B');
    // a later token keeps the choice while the account is still granted
    useSession.getState().accept(tokenResp(['A', 'B']));
    expect(useSession.getState().account).toBe('B');
    useSession.getState().accept(tokenResp(['A', 'C']));
    expect(useSession.getState().account).toBeNull();
  });

  it('refreshes silently before expiry and rotates the token', async () => {
    let n = 0;
    const { f, calls } = fakeFetch(() => {
      n++;
      return { status: 200, body: tokenResp(['A'], 300, `u${n}`) };
    });
    useSession.getState().configure('http://id.test', f);
    await useSession.getState().restore();
    const first = useSession.getState().accessToken;
    // 80% of a 300 s lifetime = 240 s
    await vi.advanceTimersByTimeAsync(239_000);
    expect(calls).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(2_000);
    expect(calls).toHaveLength(2);
    expect(useSession.getState().accessToken).not.toBe(first);
    expect(useSession.getState().claims?.sub).toBe('u2');
  });

  it('shares one in-flight refresh between concurrent callers', async () => {
    let resolve!: (r: Response) => void;
    const f = vi.fn(() => new Promise<Response>((r) => (resolve = r)));
    useSession.getState().configure('http://id.test', f as unknown as typeof fetch);
    const a = useSession.getState().refresh();
    const b = useSession.getState().refresh();
    resolve(new Response(JSON.stringify(tokenResp(['A'])), { status: 200 }));
    expect(await a).toBe(true);
    expect(await b).toBe(true);
    expect(f).toHaveBeenCalledTimes(1);
  });

  it('ends the session when refresh is rejected, keeps it on network errors', async () => {
    let mode: 'ok' | 'net' | 'reject' = 'ok';
    const f = vi.fn(async () => {
      if (mode === 'net') throw new TypeError('Failed to fetch');
      if (mode === 'reject') return new Response(JSON.stringify({ error: 'invalid_grant', message: 'reused' }), { status: 401 });
      return new Response(JSON.stringify(tokenResp(['A'])), { status: 200 });
    });
    useSession.getState().configure('http://id.test', f as unknown as typeof fetch);
    await useSession.getState().restore();
    mode = 'net';
    expect(await useSession.getState().refresh()).toBe(true);
    expect(useSession.getState().status).toBe('authenticated');
    mode = 'reject';
    expect(await useSession.getState().refresh()).toBe(false);
    const s = useSession.getState();
    expect(s.status).toBe('anonymous');
    expect(s.accessToken).toBeNull();
    expect(s.error).toBe('reused');
  });

  it('logout revokes the cookie session and forgets the account', async () => {
    const { f, calls } = fakeFetch((url) => (url.endsWith('/revoke') ? { status: 204 } : { status: 200, body: tokenResp(['A']) }));
    useSession.getState().configure('http://id.test', f);
    await useSession.getState().restore();
    await useSession.getState().logout();
    expect(useSession.getState().status).toBe('anonymous');
    expect(sessionStorage.getItem(ACCOUNT_KEY)).toBeNull();
    expect(calls.at(-1)!.url).toBe('http://id.test/v1/token/revoke');
    // no refresh timer left running
    await vi.advanceTimersByTimeAsync(600_000);
    expect(calls).toHaveLength(2);
  });
});

describe('IdentityClient', () => {
  it('maps errors and the MFA challenge', async () => {
    const { f, calls } = fakeFetch((url) =>
      url.endsWith('/v1/login')
        ? { status: 200, body: { mfa_required: true, mfa_token: 'm', methods: ['totp'] } }
        : { status: 423, body: { error: 'account_locked', message: 'locked' } },
    );
    const c = new IdentityClient('http://id.test', f);
    const r = await c.login('a@example.com', 'pw');
    expect(isMfa(r)).toBe(true);
    expect(JSON.parse(String(calls[0]!.init.body))).toEqual({ email: 'a@example.com', password: 'pw', session: 'cookie' });
    const e = await c.login2fa('m', { code: '1' }).catch((x: unknown) => x);
    expect(e).toBeInstanceOf(IdentityError);
    expect((e as IdentityError).code).toBe('account_locked');
    expect((e as IdentityError).status).toBe(423);
  });
});
