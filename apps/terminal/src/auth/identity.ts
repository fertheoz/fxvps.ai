/**
 * HTTP client for services/identity. Web mode: the refresh token lives in an
 * HttpOnly SameSite=Strict cookie scoped to /v1/token (never readable by JS); the
 * short-lived access token is kept in memory only. Cookie-authenticated calls carry
 * the CSRF header the service requires.
 */

export const CSRF_HEADER = 'X-Fxvps-Csrf';

export interface TokenResponse {
  access_token: string;
  token_type: 'Bearer';
  expires_in: number;
  refresh_expires_in: number;
  accounts: string[];
  roles: string[];
}

export interface MfaChallenge {
  mfa_required: true;
  mfa_token: string;
  methods: string[];
}

export type LoginResult = TokenResponse | MfaChallenge;

export function isMfa(r: LoginResult): r is MfaChallenge {
  return (r as MfaChallenge).mfa_required === true;
}

export interface Me {
  id: string;
  email: string;
  email_verified: boolean;
  roles: string[];
  accounts: string[];
  totp_enabled: boolean;
  recovery_codes_left: number;
  passkeys: { id: string; name: string; created_at: number }[];
}

export class IdentityError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = 'IdentityError';
  }
}

// ---- base64url <-> ArrayBuffer (WebAuthn JSON encoding)

export function b64urlToBuf(s: string): ArrayBuffer {
  const b64 = s.replace(/-/g, '+').replace(/_/g, '/') + '==='.slice((s.length + 3) % 4);
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

export function bufToB64url(b: ArrayBuffer | ArrayBufferView): string {
  const bytes = b instanceof ArrayBuffer ? new Uint8Array(b) : new Uint8Array(b.buffer, b.byteOffset, b.byteLength);
  let s = '';
  for (const x of bytes) s += String.fromCharCode(x);
  return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

/* eslint-disable @typescript-eslint/no-explicit-any */
function creationOptions(o: any): CredentialCreationOptions {
  const pk = o.publicKey;
  return {
    publicKey: {
      ...pk,
      challenge: b64urlToBuf(pk.challenge),
      user: { ...pk.user, id: b64urlToBuf(pk.user.id) },
      excludeCredentials: (pk.excludeCredentials ?? []).map((c: any) => ({ ...c, id: b64urlToBuf(c.id) })),
    },
  };
}

function requestOptions(o: any): CredentialRequestOptions {
  const pk = o.publicKey;
  return {
    publicKey: {
      ...pk,
      challenge: b64urlToBuf(pk.challenge),
      allowCredentials: (pk.allowCredentials ?? []).map((c: any) => ({ ...c, id: b64urlToBuf(c.id) })),
    },
  };
}

function credentialJson(c: PublicKeyCredential): any {
  const r = c.response as AuthenticatorAttestationResponse & AuthenticatorAssertionResponse;
  const response: Record<string, string | null> = { clientDataJSON: bufToB64url(r.clientDataJSON) };
  if ('attestationObject' in r && r.attestationObject) response.attestationObject = bufToB64url(r.attestationObject);
  if ('authenticatorData' in r && r.authenticatorData) {
    response.authenticatorData = bufToB64url(r.authenticatorData);
    response.signature = bufToB64url(r.signature);
    response.userHandle = r.userHandle ? bufToB64url(r.userHandle) : null;
  }
  return {
    id: c.id,
    rawId: bufToB64url(c.rawId),
    type: c.type,
    response,
    extensions: c.getClientExtensionResults?.() ?? {},
  };
}
/* eslint-enable @typescript-eslint/no-explicit-any */

export function passkeysSupported(): boolean {
  return typeof window !== 'undefined' && typeof window.PublicKeyCredential === 'function' && !!navigator.credentials;
}

export class IdentityClient {
  constructor(
    readonly baseUrl: string,
    private readonly fetchImpl: typeof fetch = (...a) => fetch(...a),
  ) {}

  private async call<T>(path: string, init: { method?: string; body?: unknown; token?: string; cookie?: boolean } = {}): Promise<T> {
    const headers: Record<string, string> = {};
    if (init.body !== undefined) headers['Content-Type'] = 'application/json';
    if (init.token) headers.Authorization = `Bearer ${init.token}`;
    if (init.cookie) headers[CSRF_HEADER] = '1';
    const r = await this.fetchImpl(`${this.baseUrl.replace(/\/$/, '')}${path}`, {
      method: init.method ?? (init.body !== undefined ? 'POST' : 'GET'),
      headers,
      body: init.body !== undefined ? JSON.stringify(init.body) : undefined,
      credentials: 'include',
    });
    if (r.status === 204) return undefined as T;
    const text = await r.text();
    let data: unknown;
    try {
      data = text ? JSON.parse(text) : undefined;
    } catch {
      data = undefined;
    }
    if (!r.ok) {
      const e = (data ?? {}) as { error?: string; message?: string };
      throw new IdentityError(r.status, e.error ?? 'http_error', e.message ?? `HTTP ${r.status}`);
    }
    return data as T;
  }

  register(email: string, password: string): Promise<void> {
    return this.call('/v1/register', { body: { email, password } });
  }
  verifyEmail(token: string): Promise<void> {
    return this.call('/v1/verify-email', { body: { token } });
  }
  resendVerification(email: string): Promise<void> {
    return this.call('/v1/verify-email/resend', { body: { email } });
  }
  forgotPassword(email: string): Promise<void> {
    return this.call('/v1/password/forgot', { body: { email } });
  }
  resetPassword(token: string, password: string): Promise<void> {
    return this.call('/v1/password/reset', { body: { token, password } });
  }
  login(email: string, password: string): Promise<LoginResult> {
    return this.call('/v1/login', { body: { email, password, session: 'cookie' } });
  }
  login2fa(mfaToken: string, code: { code?: string; recovery_code?: string }): Promise<TokenResponse> {
    return this.call('/v1/login/2fa', { body: { mfa_token: mfaToken, ...code, session: 'cookie' } });
  }
  /** Rotates the refresh cookie and returns a fresh access token. */
  refresh(): Promise<TokenResponse> {
    return this.call('/v1/token/refresh', { method: 'POST', cookie: true });
  }
  logout(): Promise<void> {
    return this.call('/v1/token/revoke', { method: 'POST', cookie: true });
  }
  me(token: string): Promise<Me> {
    return this.call('/v1/me', { token });
  }
  totpEnroll(token: string): Promise<{ secret: string; otpauth_url: string }> {
    return this.call('/v1/2fa/totp/enroll', { body: {}, token });
  }
  totpConfirm(token: string, code: string): Promise<{ recovery_codes: string[] }> {
    return this.call('/v1/2fa/totp/confirm', { body: { code }, token });
  }
  totpDisable(token: string, code: string): Promise<void> {
    return this.call('/v1/2fa/totp/disable', { body: { code }, token });
  }
  apiKeys(token: string): Promise<{ keys: ApiKeyInfo[] }> {
    return this.call('/v1/api-keys', { token });
  }
  /** The returned `secret` is shown once; the server keeps only its hash. */
  apiKeyCreate(token: string, name: string, scope: 'read' | 'trade', ips: string[]): Promise<ApiKeyInfo & { secret: string }> {
    return this.call('/v1/api-keys', { body: { name, scope, ips }, token });
  }
  apiKeyRevoke(token: string, id: string): Promise<void> {
    return this.call('/v1/api-keys/revoke', { body: { id }, token });
  }

  async registerPasskey(token: string, name: string): Promise<void> {
    const start = await this.call<{ registration_id: string; options: unknown }>('/v1/passkeys/register/start', { body: {}, token });
    const cred = (await navigator.credentials.create(creationOptions(start.options))) as PublicKeyCredential | null;
    if (!cred) throw new IdentityError(0, 'cancelled', 'Passkey creation cancelled');
    await this.call('/v1/passkeys/register/finish', {
      body: { registration_id: start.registration_id, credential: credentialJson(cred), name },
      token,
    });
  }

  async loginWithPasskey(email: string): Promise<TokenResponse> {
    const start = await this.call<{ authentication_id: string; options: unknown }>('/v1/passkeys/login/start', { body: { email } });
    const cred = (await navigator.credentials.get(requestOptions(start.options))) as PublicKeyCredential | null;
    if (!cred) throw new IdentityError(0, 'cancelled', 'Passkey sign-in cancelled');
    return this.call('/v1/passkeys/login/finish', {
      body: { authentication_id: start.authentication_id, credential: credentialJson(cred), session: 'cookie' },
    });
  }
}

export interface ApiKeyInfo {
  id: string;
  name: string;
  scope: 'read' | 'trade';
  ips: string[];
  created_at: number;
  last_used: number | null;
  revoked: boolean;
}
