import { useEffect, useState } from 'react';
import { formatMoney } from '@fxvps/trading-core';
import { useT } from '../hooks';
import { useTerminal } from '../store/terminal';
import { clientApi, clientApiBase, type ClientMe, type FundingKind, type FundingMethod } from '../api/clientApi';
import { Modal } from './Dialogs';
import { StatementTab } from './StatementTab';
import { CopyTab } from './CopyTab';
import { IbTab } from './IbTab';
import { ReferralBanner } from './ReferralBanner';

type Tab = 'funding' | 'verification' | 'copy' | 'ib' | 'statement';
const DOC_KINDS = ['id_front', 'id_back', 'proof_of_address', 'selfie', 'other'] as const;

/** Funding (deposit / withdrawal requests) and verification (KYC documents) for the signed-in client. */
export function AccountDialog() {
  const t = useT();
  const open = useTerminal((s) => s.accountOpen);
  const setOpen = useTerminal((s) => s.setAccountOpen);
  const toast = useTerminal((s) => s.toast);
  const [tab, setTab] = useState<Tab>('funding');
  const [me, setMe] = useState<ClientMe | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [account, setAccount] = useState('');
  const [kind, setKind] = useState<FundingKind>('deposit');
  const [method, setMethod] = useState<FundingMethod>('usdt_trc20');
  const [amount, setAmount] = useState('');
  const [details, setDetails] = useState('');
  const [docKind, setDocKind] = useState<(typeof DOC_KINDS)[number]>('id_front');
  const reload = async () => {
    try {
      const m = await clientApi.me();
      setMe(m);
      setError(null);
      if (!account && m.accounts[0]) setAccount(m.accounts[0].externalId);
    } catch (e) {
      setError((e as Error).message);
    }
  };
  useEffect(() => {
    if (!open) return;
    // load after mount (not synchronously inside the effect)
    const id = setTimeout(() => void reload(), 0);
    return () => clearTimeout(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);
  if (!open) return null;
  const close = () => setOpen(false);
  if (!clientApiBase()) {
    return (
      <Modal title={t('acct.title')} onClose={close}>
        <div className="p-4 text-[12px] text-muted">{t('acct.noGateway')}</div>
      </Modal>
    );
  }
  const acc = me?.accounts.find((a) => a.externalId === account) ?? me?.accounts[0];
  // USDT deposits go through the hazine card, for USD accounts only
  const card = !!me?.cryptoCard && kind === 'deposit' && method === 'usdt_trc20';
  const cardUsd = acc?.currency === 'USD';
  const minor =(v: string) => Math.round(Number(v.replace(',', '.')) * 100);
  const submitFunding = async () => {
    if (!acc) return;
    const m = minor(amount);
    if (!Number.isFinite(m) || m <= 0) return toast('error', t('acct.badAmount'));
    setBusy(true);
    try {
      const fr = await clientApi.requestFunding({ account: acc.externalId, kind, method, amount: m, details: card ? '' : details });
      toast('ok', t('acct.requested'));
      // USDT deposit: the hazine card takes the payment (wallet connect / QR)
      if (fr.payUrl) window.open(fr.payUrl, '_blank', 'noopener');
      setAmount('');
      setDetails('');
      await reload();
    } catch (e) {
      toast('error', (e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const upload = async (file: File | undefined) => {
    if (!file || !acc) return;
    setBusy(true);
    try {
      await clientApi.uploadDocument(acc.externalId, docKind, file);
      toast('ok', t('acct.uploaded'));
      await reload();
    } catch (e) {
      toast('error', (e as Error).message);
    } finally {
      setBusy(false);
    }
  };
  const inp = 'bg-panel-2 border border-line rounded px-2 h-8 text-[12px] w-full';
  const btn = 'px-2 h-8 rounded border border-line hover:bg-hover disabled:opacity-40 text-[12px]';
  const tabBtn = (v: Tab, label: string) => (
    <button key={v} className={`px-3 h-8 text-[12px] border-b-2 ${tab === v ? 'border-accent text-fg' : 'border-transparent text-muted hover:text-fg'}`} onClick={() => setTab(v)} data-testid={`acct-tab-${v}`}>
      {label}
    </button>
  );
  const ins = me?.instructions;
  return (
    <Modal title={t('acct.title')} onClose={close} width="w-[640px]">
      <div className="px-3 border-b border-line flex items-center gap-1">
        {tabBtn('funding', t('acct.funding'))}
        {tabBtn('verification', t('acct.verification'))}
        {tabBtn('statement', t('stmt.tab'))}
        {tabBtn('copy', t('copy.tab'))}
        {tabBtn('ib', t('ib.tab'))}
        {me && me.accounts.length > 1 && (
          <select className="ml-auto bg-panel-2 border border-line rounded px-2 h-7 text-[12px]" value={account} onChange={(e) => setAccount(e.target.value)} aria-label={t('top.account')}>
            {me.accounts.map((a) => <option key={a.externalId} value={a.externalId}>{a.externalId} · {a.currency}</option>)}
          </select>
        )}
      </div>
      <div className="p-3 grid gap-3 text-[12px]" data-testid="account-dialog">
        {error && <div className="text-down">{error}</div>}
        {!me && !error && <div className="text-muted">…</div>}
        {me && acc && (
          <div className="flex flex-wrap gap-x-4 gap-y-1 text-muted">
            <span>{acc.externalId} · {acc.group}</span>
            <span>{t('acct.balance')}: <span className="num text-fg">{formatMoney(acc.balance)} {acc.currency}</span></span>
            <span>{t('acct.kyc')}: <span className={acc.kyc === 'approved' ? 'text-up' : acc.kyc === 'rejected' ? 'text-down' : 'text-fg'}>{t(`acct.kyc.${acc.kyc}`)}</span></span>
          </div>
        )}
        {me && acc && <ReferralBanner key={acc.externalId} account={acc.externalId} />}

        {tab === 'funding' && me && acc && (
          <>
            <div className="grid gap-2 border border-line rounded p-2">
              <div className="text-[10px] uppercase text-muted">{t('acct.instructions')}</div>
              {me.cryptoCard && <div><span className="text-muted">USDT (TRC-20): </span>{t(cardUsd ? 'acct.usdtCard' : 'acct.usdtUsdOnly')}</div>}
              {!me.cryptoCard && ins?.usdtTrc20Address ? (
                <div><span className="text-muted">USDT (TRC-20): </span><code className="select-all break-all">{ins.usdtTrc20Address}</code></div>
              ) : null}
              {ins?.bankDetails ? <pre className="whitespace-pre-wrap font-sans text-[12px]">{ins.bankDetails}</pre> : null}
              {!me.cryptoCard && !ins?.usdtTrc20Address && !ins?.bankDetails && <div className="text-muted">{t('acct.noMethods')}</div>}
              <div className="text-muted text-[11px]">{t('acct.depositHint')}</div>
            </div>
            <div className="grid grid-cols-2 gap-2">
              <label className="grid gap-1">{t('acct.kind')}
                <select className={inp} value={kind} onChange={(e) => setKind(e.target.value as FundingKind)}>
                  <option value="deposit">{t('acct.deposit')}</option>
                  <option value="withdraw">{t('acct.withdraw')}</option>
                </select>
              </label>
              <label className="grid gap-1">{t('acct.method')}
                <select className={inp} value={method} onChange={(e) => setMethod(e.target.value as FundingMethod)}>
                  <option value="usdt_trc20">USDT TRC-20</option>
                  <option value="bank">{t('acct.bank')}</option>
                </select>
              </label>
              <label className="grid gap-1">{t('acct.amount')} ({acc.currency})
                <input className={`${inp} num`} value={amount} onChange={(e) => setAmount(e.target.value)} placeholder="100.00" data-testid="acct-amount" />
              </label>
              {/* the hazine card finds the payment itself: no reference to type */}
              {!card && (
                <label className="grid gap-1">{kind === 'deposit' ? t('acct.detailsDeposit') : t('acct.detailsWithdraw')}
                  <input className={inp} value={details} onChange={(e) => setDetails(e.target.value)} data-testid="acct-details" />
                </label>
              )}
            </div>
            <div className="flex items-center gap-2">
              <button className="px-3 h-8 rounded bg-accent text-white disabled:opacity-40 text-[12px]" disabled={busy} onClick={() => void submitFunding()} data-testid="acct-submit">{t('acct.submit')}</button>
              <span className="text-muted text-[11px]">{t('acct.reviewHint')}</span>
            </div>
            <div>
              <div className="text-[10px] uppercase text-muted mb-1">{t('acct.history')}</div>
              {me.funding.length === 0 && <div className="text-muted">{t('tb.empty')}</div>}
              {me.funding.slice().sort((a, b) => b.requestedAt - a.requestedAt).map((f) => (
                <div key={f.id}>
                <div className="flex items-center justify-between gap-2 border-t border-line/60 py-1">
                  <span>{f.id} · {t(f.kind === 'deposit' ? 'acct.deposit' : 'acct.withdraw')} · <span className="num">{formatMoney(f.amount)} {f.currency}</span> · {f.method === 'usdt_trc20' ? 'USDT' : t('acct.bank')}</span>
                  <span className={f.status === 'rejected' ? 'text-down' : f.status === 'requested' ? 'text-muted' : 'text-up'}>{t(`acct.status.${f.status}`)}{f.note ? ` · ${f.note}` : ''}</span>
                </div>
                {f.status === 'requested' && f.expectedMicro && (
                  <div className="text-[11px] pb-1" data-testid={`usdt-exact-${f.id}`}>
                    {f.txHash ? (
                      <span className="text-up">{t('acct.usdtSeen')}</span>
                    ) : f.payUrl ? (
                      <a className="text-accent underline" href={f.payUrl} target="_blank" rel="noopener noreferrer" data-testid={`usdt-pay-${f.id}`}>
                        {t('acct.usdtPay')} · {(f.expectedMicro / 1e6).toFixed(6)} USDT
                      </a>
                    ) : (
                      <>{t('acct.usdtExact')}: <code className="select-all num text-fg">{(f.expectedMicro / 1e6).toFixed(6)}</code> USDT</>
                    )}
                  </div>
                )}
                {f.status !== 'requested' && f.paidAfterDecision && (
                  <div className="text-[11px] pb-1 text-up" data-testid={`usdt-late-${f.id}`}>{t('acct.usdtLate')}</div>
                )}
                </div>
              ))}
            </div>
          </>
        )}

        {tab === 'copy' && me && acc && <CopyTab acc={acc} />}
        {tab === 'ib' && me && acc && <IbTab acc={acc} />}

        {/* keyed: another account starts with an empty statement, a late load for the old one is dropped */}
        {tab === 'statement' && me && acc && <StatementTab key={acc.externalId} acc={acc} />}

        {tab === 'verification' && me && acc && (
          <>
            <div className="text-muted">{t('acct.kycHint')}</div>
            <div className="flex flex-wrap items-end gap-2">
              <label className="grid gap-1">{t('acct.docKind')}
                <select className={inp} value={docKind} onChange={(e) => setDocKind(e.target.value as (typeof DOC_KINDS)[number])}>
                  {DOC_KINDS.map((k) => <option key={k} value={k}>{t(`acct.doc.${k}`)}</option>)}
                </select>
              </label>
              <label className={`${btn} cursor-pointer flex items-center`}>
                {t('acct.chooseFile')}
                <input type="file" className="hidden" accept="image/jpeg,image/png,image/webp,application/pdf" disabled={busy} onChange={(e) => void upload(e.target.files?.[0])} data-testid="acct-file" />
              </label>
              <span className="text-muted text-[11px]">JPEG / PNG / WebP / PDF · ≤ 6 MB</span>
            </div>
            <div>
              <div className="text-[10px] uppercase text-muted mb-1">{t('acct.documents')}</div>
              {me.documents.filter((d) => d.account === acc.login).length === 0 && <div className="text-muted">{t('tb.empty')}</div>}
              {me.documents.filter((d) => d.account === acc.login).map((d) => (
                <div key={d.id} className="flex items-center justify-between gap-2 border-t border-line/60 py-1">
                  <span>{t(`acct.doc.${d.kind}` as 'acct.doc.other')} · {d.filename} · {(d.size / 1024).toFixed(0)} KB</span>
                  <span className="text-muted">{new Date(d.uploadedAt).toLocaleString()}</span>
                </div>
              ))}
            </div>
          </>
        )}
      </div>
    </Modal>
  );
}
