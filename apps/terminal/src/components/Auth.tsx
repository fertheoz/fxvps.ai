import { useEffect, useState, type FormEvent, type ReactNode } from 'react';
import { useT } from '../hooks';
import type { MessageKey } from '../i18n';
import { IdentityError, isMfa, passkeysSupported, type Me } from '../auth/identity';
import { useSession } from '../store/session';
import { useTerminal } from '../store/terminal';
import { saveGateway } from '../store/connection';
import { Modal } from './Dialogs';

const input = 'w-full bg-panel-2 border border-line rounded px-2 py-1.5 text-[12px]';
const primary = 'px-3 py-1.5 rounded bg-accent text-white disabled:opacity-50';
const secondary = 'px-3 py-1.5 rounded border border-line text-muted hover:text-fg disabled:opacity-50';
const linkBtn = 'text-accent hover:underline text-[11px] bg-transparent border-0 p-0 cursor-pointer';

const ERRORS: Record<string, MessageKey> = {
  invalid_credentials: 'auth.err.invalid_credentials',
  account_locked: 'auth.err.account_locked',
  email_not_verified: 'auth.err.email_not_verified',
  invalid_code: 'auth.err.invalid_code',
  invalid_mfa_token: 'auth.err.invalid_mfa_token',
  rate_limited: 'auth.err.rate_limited',
  weak_password: 'auth.err.weak_password',
  invalid_email: 'auth.err.invalid_email',
  invalid_token: 'auth.err.invalid_token',
  no_passkeys: 'auth.err.no_passkeys',
};

function useErrorText() {
  const t = useT();
  return (e: unknown) => {
    if (e instanceof IdentityError && ERRORS[e.code]) return t(ERRORS[e.code]!);
    return t('auth.err.generic', { message: e instanceof Error ? e.message : String(e) });
  };
}

type LinkParam = { kind: 'verify' | 'reset'; token: string } | null;

function peekLinkParam(): LinkParam {
  if (typeof location === 'undefined') return null;
  const p = new URLSearchParams(location.search);
  const v = p.get('verify_email');
  const r = p.get('reset_password');
  return v ? { kind: 'verify', token: v } : r ? { kind: 'reset', token: r } : null;
}

let linkParam: LinkParam | undefined;
let verifying: Promise<void> | null = null;

/** Verifies an emailed token once per page load (StrictMode runs effects twice). */
function verifyOnce(client: { verifyEmail(t: string): Promise<void> }, token: string): Promise<void> {
  verifying ??= client.verifyEmail(token);
  return verifying;
}

/** `?verify_email=` / `?reset_password=` from an emailed link, read once and stripped from the URL. */
function takeLinkParam(): LinkParam {
  if (linkParam !== undefined) return linkParam;
  linkParam = peekLinkParam();
  if (linkParam) {
    const p = new URLSearchParams(location.search);
    p.delete('verify_email');
    p.delete('reset_password');
    const q = p.toString();
    history.replaceState(history.state, '', `${location.pathname}${q ? `?${q}` : ''}${location.hash}`);
  }
  return linkParam;
}

type View = 'login' | 'register' | 'forgot' | 'reset' | 'mfa';

function Shell({ title, children }: { title: string; children: ReactNode }) {
  const t = useT();
  return (
    <div className="h-full grid place-items-center bg-bg p-4" data-testid="auth-screen">
      <div className="w-[380px] max-w-full bg-panel border border-line rounded-lg shadow-2xl">
        <div className="px-4 pt-4 pb-2">
          <div className="text-muted text-[11px]">{t('app.title')}</div>
          <h1 className="text-[16px] font-semibold m-0">{title}</h1>
        </div>
        <div className="px-4 pb-4">{children}</div>
      </div>
    </div>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="flex flex-col gap-1">
      <span className="text-muted text-[11px]">{label}</span>
      {children}
    </label>
  );
}

function Notice({ tone, children, testId }: { tone: 'ok' | 'error'; children: ReactNode; testId?: string }) {
  return (
    <div role={tone === 'error' ? 'alert' : 'status'} data-testid={testId} className={`text-[12px] ${tone === 'error' ? 'text-down' : 'text-up'}`}>
      {children}
    </div>
  );
}

export function LoginScreen() {
  const t = useT();
  const errText = useErrorText();
  const client = useSession((s) => s.client)!;
  const accept = useSession((s) => s.accept);
  const sessionError = useSession((s) => s.error);
  const link = takeLinkParam();
  const [view, setView] = useState<View>(link?.kind === 'reset' ? 'reset' : 'login');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [code, setCode] = useState('');
  const [recovery, setRecovery] = useState(false);
  const [mfaToken, setMfaToken] = useState('');
  const resetToken = link?.kind === 'reset' ? link.token : '';
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(sessionError ? t('auth.sessionExpired') : null);
  const [info, setInfo] = useState<string | null>(null);

  useEffect(() => {
    if (link?.kind !== 'verify') return;
    verifyOnce(client, link.token)
      .then(() => setInfo(t('auth.verified')))
      .catch((e: unknown) => setError(errText(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function run(f: () => Promise<void>) {
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      await f();
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  }

  const go = (v: View) => () => {
    setView(v);
    setError(null);
    setInfo(null);
  };

  const submit = (f: () => Promise<void>) => (e: FormEvent) => {
    e.preventDefault();
    void run(f);
  };

  const messages = (
    <>
      {error && <Notice tone="error" testId="auth-error">{error}</Notice>}
      {info && <Notice tone="ok" testId="auth-info">{info}</Notice>}
    </>
  );

  if (view === 'mfa')
    return (
      <Shell title={t('auth.mfaTitle')}>
        <form
          className="flex flex-col gap-3"
          data-testid="mfa-form"
          onSubmit={submit(async () => {
            const r = await client.login2fa(mfaToken, recovery ? { recovery_code: code } : { code });
            accept(r);
          })}
        >
          <p className="text-muted text-[12px] m-0">{recovery ? t('auth.recoveryHint') : t('auth.mfaHint')}</p>
          <Field label={recovery ? t('auth.recoveryCode') : t('auth.code')}>
            <input
              className={`${input} num tracking-widest`}
              value={code}
              onChange={(e) => setCode(e.target.value)}
              inputMode={recovery ? 'text' : 'numeric'}
              autoComplete="one-time-code"
              autoFocus
              data-testid="mfa-code"
            />
          </Field>
          {messages}
          <div className="flex items-center justify-between">
            <button
              type="button"
              className={linkBtn}
              onClick={() => {
                setRecovery(!recovery);
                setCode('');
              }}
            >
              {recovery ? t('auth.useTotp') : t('auth.useRecovery')}
            </button>
            <button type="submit" className={primary} disabled={busy || !code.trim()} data-testid="mfa-submit">
              {t('auth.verify')}
            </button>
          </div>
          <button type="button" className={linkBtn} onClick={go('login')}>
            {t('auth.back')}
          </button>
        </form>
      </Shell>
    );

  if (view === 'register')
    return (
      <Shell title={t('auth.createAccount')}>
        <form
          className="flex flex-col gap-3"
          data-testid="register-form"
          onSubmit={submit(async () => {
            await client.register(email.trim(), password);
            setPassword('');
            setView('login');
            setInfo(t('auth.registered'));
          })}
        >
          <Field label={t('auth.email')}>
            <input className={input} type="email" value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="email" required autoFocus data-testid="register-email" />
          </Field>
          <Field label={t('auth.password')}>
            <input
              className={input}
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="new-password"
              minLength={10}
              required
              data-testid="register-password"
            />
            <span className="text-muted text-[11px]">{t('auth.passwordHint')}</span>
          </Field>
          {messages}
          <div className="flex items-center justify-between">
            <button type="button" className={linkBtn} onClick={go('login')}>
              {t('auth.back')}
            </button>
            <button type="submit" className={primary} disabled={busy} data-testid="register-submit">
              {t('auth.register')}
            </button>
          </div>
        </form>
      </Shell>
    );

  if (view === 'forgot')
    return (
      <Shell title={t('auth.forgot')}>
        <form
          className="flex flex-col gap-3"
          onSubmit={submit(async () => {
            await client.forgotPassword(email.trim());
            setInfo(t('auth.resetSent'));
          })}
        >
          <Field label={t('auth.email')}>
            <input className={input} type="email" value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="email" required autoFocus />
          </Field>
          {messages}
          <div className="flex items-center justify-between">
            <button type="button" className={linkBtn} onClick={go('login')}>
              {t('auth.back')}
            </button>
            <button type="submit" className={primary} disabled={busy}>
              {t('auth.sendReset')}
            </button>
          </div>
        </form>
      </Shell>
    );

  if (view === 'reset')
    return (
      <Shell title={t('auth.newPassword')}>
        <form
          className="flex flex-col gap-3"
          onSubmit={submit(async () => {
            await client.resetPassword(resetToken, password);
            setPassword('');
            setView('login');
            setInfo(t('auth.passwordChanged'));
          })}
        >
          <Field label={t('auth.newPassword')}>
            <input
              className={input}
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="new-password"
              minLength={10}
              required
              autoFocus
            />
            <span className="text-muted text-[11px]">{t('auth.passwordHint')}</span>
          </Field>
          {messages}
          <div className="flex justify-end">
            <button type="submit" className={primary} disabled={busy}>
              {t('auth.setPassword')}
            </button>
          </div>
        </form>
      </Shell>
    );

  return (
    <Shell title={t('auth.title')}>
      <form
        className="flex flex-col gap-3"
        data-testid="login-form"
        onSubmit={submit(async () => {
          const r = await client.login(email.trim(), password);
          setPassword('');
          if (isMfa(r)) {
            setMfaToken(r.mfa_token);
            setCode('');
            setRecovery(false);
            setView('mfa');
          } else accept(r);
        })}
      >
        <Field label={t('auth.email')}>
          <input className={input} type="email" value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="username webauthn" required autoFocus data-testid="login-email" />
        </Field>
        <Field label={t('auth.password')}>
          <input className={input} type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" required data-testid="login-password" />
        </Field>
        {messages}
        {error && email && (
          <button type="button" className={`${linkBtn} self-start`} onClick={() => void run(async () => {
            await client.resendVerification(email.trim());
            setInfo(t('auth.registered'));
          })}>
            {t('auth.resend')}
          </button>
        )}
        <button type="submit" className={primary} disabled={busy} data-testid="login-submit">
          {t('auth.signIn')}
        </button>
        {passkeysSupported() && (
          <button
            type="button"
            className={secondary}
            disabled={busy || !email.trim()}
            data-testid="passkey-login"
            onClick={() => void run(async () => accept(await client.loginWithPasskey(email.trim())))}
          >
            {t('auth.passkey')}
          </button>
        )}
        <div className="flex items-center justify-between">
          <button type="button" className={linkBtn} onClick={go('forgot')} data-testid="forgot-link">
            {t('auth.forgot')}
          </button>
          <button type="button" className={linkBtn} onClick={go('register')} data-testid="register-link">
            {t('auth.createAccount')}
          </button>
        </div>
        <button
          type="button"
          className={`${linkBtn} self-center`}
          data-testid="auth-use-demo"
          onClick={() => {
            saveGateway(null);
            location.assign(location.pathname);
          }}
        >
          {t('auth.useDemo')}
        </button>
      </form>
    </Shell>
  );
}

export function AccountPicker() {
  const t = useT();
  const claims = useSession((s) => s.claims)!;
  const select = useSession((s) => s.selectAccount);
  const logout = useSession((s) => s.logout);
  const [choice, setChoice] = useState(claims.accounts[0] ?? '');
  return (
    <Shell title={t('auth.pickAccount')}>
      <div className="flex flex-col gap-3" data-testid="account-picker">
        <div className="text-muted text-[12px]">{claims.email}</div>
        {claims.accounts.length === 0 ? (
          <Notice tone="error" testId="no-accounts">
            {t('auth.noAccounts')}
          </Notice>
        ) : (
          <div className="flex flex-col gap-1" role="radiogroup">
            {claims.accounts.map((a) => (
              <label key={a} className="flex items-center gap-2 px-2 py-1.5 rounded border border-line cursor-pointer hover:bg-panel-2">
                <input type="radio" name="account" value={a} checked={choice === a} onChange={() => setChoice(a)} data-testid={`account-option-${a}`} />
                <span className="num">{a}</span>
              </label>
            ))}
          </div>
        )}
        <div className="flex justify-between">
          <button type="button" className={secondary} onClick={() => void logout()}>
            {t('auth.signOut')}
          </button>
          <button type="button" className={primary} disabled={!choice} onClick={() => select(choice)} data-testid="account-continue">
            {t('auth.continue')}
          </button>
        </div>
      </div>
    </Shell>
  );
}

/** Makes the account chosen at login the active terminal account once it is loaded. */
function AccountSync() {
  const account = useSession((s) => s.account);
  const accounts = useTerminal((s) => s.accounts);
  const setActive = useTerminal((s) => s.setActiveAccount);
  useEffect(() => {
    if (account && accounts.some((a) => a.id === account)) setActive(account);
  }, [account, accounts, setActive]);
  return null;
}

/**
 * Requires an identity session before rendering `children` (the terminal against a
 * gateway). Restores the session silently from the refresh cookie on load.
 */
export function AuthGate({ children }: { children: ReactNode }) {
  const t = useT();
  const status = useSession((s) => s.status);
  const account = useSession((s) => s.account);
  const restore = useSession((s) => s.restore);
  // Emailed links are handled by the login screen; skip the silent sign-in then.
  const [restored, setRestored] = useState(() => peekLinkParam() !== null);

  useEffect(() => {
    if (restored) return;
    let live = true;
    void restore().finally(() => live && setRestored(true));
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [restore]);

  if (!restored || status === 'checking' || status === 'idle')
    return (
      <div className="h-full grid place-items-center text-muted" data-testid="auth-checking">
        {t('auth.checking')}…
      </div>
    );
  if (status !== 'authenticated') return <LoginScreen />;
  if (!account) return <AccountPicker />;
  return (
    <>
      {children}
      <AccountSync />
    </>
  );
}

/** TOTP enrolment / passkey registration for the signed-in user. */
export function SecurityDialog({ onClose }: { onClose: () => void }) {
  const t = useT();
  const errText = useErrorText();
  const client = useSession((s) => s.client)!;
  const token = useSession((s) => s.accessToken);
  const [me, setMe] = useState<Me | null>(null);
  const [enroll, setEnroll] = useState<{ secret: string; otpauth_url: string } | null>(null);
  const [codes, setCodes] = useState<string[] | null>(null);
  const [code, setCode] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (token) client.me(token).then(setMe, (e: unknown) => setError(errText(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [client, token]);

  async function run(f: (tok: string) => Promise<void>) {
    if (!token) return;
    setBusy(true);
    setError(null);
    setInfo(null);
    try {
      await f(token);
      setMe(await client.me(token));
    } catch (e) {
      setError(errText(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal title={t('sec.title')} onClose={onClose} width="w-[460px]">
      <div className="p-3 flex flex-col gap-4" data-testid="security-dialog">
        <div className="text-muted text-[12px]">{me?.email}</div>
        <section className="flex flex-col gap-2">
          <div className="flex items-center justify-between">
            <h3 className="m-0 font-semibold text-[12px]">{t('sec.totp')}</h3>
            <span className={me?.totp_enabled ? 'text-up' : 'text-muted'} data-testid="totp-status">
              {me?.totp_enabled ? t('sec.totpOn') : t('sec.totpOff')}
            </span>
          </div>
          {codes ? (
            <div className="flex flex-col gap-2" data-testid="recovery-codes">
              <div className="font-semibold text-[12px]">{t('sec.recoveryTitle')}</div>
              <p className="text-muted text-[11px] m-0">{t('sec.recoveryHint')}</p>
              <pre className="num bg-panel-2 border border-line rounded p-2 m-0 grid grid-cols-2 gap-x-4">
                {codes.map((c) => (
                  <span key={c}>{c}</span>
                ))}
              </pre>
              <button type="button" className={`${primary} self-end`} onClick={() => setCodes(null)}>
                {t('sec.done')}
              </button>
            </div>
          ) : enroll ? (
            <form
              className="flex flex-col gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                void run(async (tok) => {
                  const r = await client.totpConfirm(tok, code.trim());
                  setEnroll(null);
                  setCode('');
                  setCodes(r.recovery_codes);
                });
              }}
            >
              <p className="text-muted text-[11px] m-0">{t('sec.scan')}</p>
              <Field label={t('sec.secret')}>
                <code className="num bg-panel-2 border border-line rounded px-2 py-1 break-all select-all" data-testid="totp-secret">
                  {enroll.secret}
                </code>
              </Field>
              <a className="text-accent text-[11px]" href={enroll.otpauth_url}>
                otpauth://
              </a>
              <Field label={t('auth.code')}>
                <input className={`${input} num`} value={code} onChange={(e) => setCode(e.target.value)} inputMode="numeric" autoComplete="one-time-code" data-testid="totp-code" />
              </Field>
              <button type="submit" className={`${primary} self-end`} disabled={busy || !code.trim()} data-testid="totp-confirm">
                {t('auth.verify')}
              </button>
            </form>
          ) : me?.totp_enabled ? (
            <form
              className="flex gap-2 items-end"
              onSubmit={(e) => {
                e.preventDefault();
                void run(async (tok) => {
                  await client.totpDisable(tok, code.trim());
                  setCode('');
                });
              }}
            >
              <Field label={t('auth.code')}>
                <input className={`${input} num`} value={code} onChange={(e) => setCode(e.target.value)} inputMode="numeric" autoComplete="one-time-code" />
              </Field>
              <button type="submit" className={secondary} disabled={busy || !code.trim()}>
                {t('sec.disable')}
              </button>
            </form>
          ) : (
            <button
              type="button"
              className={`${primary} self-start`}
              disabled={busy || !me}
              data-testid="totp-enable"
              onClick={() => void run(async (tok) => setEnroll(await client.totpEnroll(tok)))}
            >
              {t('sec.enable')}
            </button>
          )}
        </section>
        <section className="flex flex-col gap-2">
          <h3 className="m-0 font-semibold text-[12px]">{t('sec.passkeys')}</h3>
          {me && me.passkeys.length > 0 && (
            <ul className="m-0 pl-4 text-[12px]">
              {me.passkeys.map((p) => (
                <li key={p.id}>{p.name}</li>
              ))}
            </ul>
          )}
          <button
            type="button"
            className={`${secondary} self-start`}
            disabled={busy || !passkeysSupported()}
            data-testid="passkey-add"
            onClick={() =>
              void run(async (tok) => {
                await client.registerPasskey(tok, navigator.platform || 'passkey');
                setInfo(t('sec.passkeyAdded'));
              })
            }
          >
            {t('sec.addPasskey')}
          </button>
        </section>
        {error && <Notice tone="error">{error}</Notice>}
        {info && <Notice tone="ok">{info}</Notice>}
      </div>
    </Modal>
  );
}

/** Signed-in user chip for the top bar (identity sessions only). */
export function UserMenu() {
  const t = useT();
  const claims = useSession((s) => s.claims);
  const status = useSession((s) => s.status);
  const logout = useSession((s) => s.logout);
  const [open, setOpen] = useState(false);
  if (status !== 'authenticated' || !claims) return null;
  return (
    <>
      <button
        className="px-2 py-1 rounded border border-line text-muted hover:text-fg max-w-[200px] truncate"
        onClick={() => setOpen(true)}
        title={t('top.user')}
        data-testid="user-menu"
      >
        {claims.email}
      </button>
      <button
        className="px-2 py-1 rounded border border-line text-muted hover:text-fg"
        data-testid="sign-out"
        onClick={() => {
          // Keep the terminal mounted until the reload: showing the login form first
          // would let input typed into it be wiped by the reload.
          void logout({ keepState: true }).then(() => location.reload());
        }}
      >
        {t('auth.signOut')}
      </button>
      {open && <SecurityDialog onClose={() => setOpen(false)} />}
    </>
  );
}
