import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../api/clientApi', async (importOriginal) => {
  const real = await importOriginal<typeof import('../api/clientApi')>();
  return { ...real, clientApi: { ...real.clientApi, ibLink: vi.fn() } };
});

import { clientApi, ClientApiError } from '../api/clientApi';
import { acceptReferral, captureReferral, dismissReferral, pendingReferral } from './referral';

const ibLink = vi.mocked(clientApi.ibLink);

describe('referral', () => {
  beforeEach(() => {
    localStorage.clear();
    ibLink.mockReset();
  });

  it('remembers ?ref= from the landing URL but sends nothing by itself', () => {
    window.history.replaceState(null, '', '/?ref=sub-1');
    captureReferral();
    window.history.replaceState(null, '', '/');
    expect(pendingReferral()).toBe('SUB-1');
    expect(ibLink).not.toHaveBeenCalled();
  });

  it('links only when the client accepts, then forgets the code', async () => {
    localStorage.setItem('fxvps.ref', 'NINE');
    ibLink.mockResolvedValue({ ok: true });
    expect(await acceptReferral('DEMO-1')).toBe('linked');
    expect(ibLink).toHaveBeenCalledWith('DEMO-1', 'NINE');
    expect(pendingReferral()).toBeNull();
  });

  it('reports an account that already has a partner', async () => {
    localStorage.setItem('fxvps.ref', 'NINE');
    ibLink.mockResolvedValue({ ok: true, already: true });
    expect(await acceptReferral('DEMO-1')).toBe('already');
    expect(pendingReferral()).toBeNull();
  });

  it('keeps the code without an answer, forgets it once the server refused', async () => {
    localStorage.setItem('fxvps.ref', 'NINE');
    ibLink.mockRejectedValueOnce(new ClientApiError(0, 'no gateway'));
    await expect(acceptReferral('DEMO-1')).rejects.toThrow('no gateway');
    expect(pendingReferral()).toBe('NINE');
    ibLink.mockRejectedValueOnce(new ClientApiError(409, "a referral code can only be added before the account's first trade"));
    await expect(acceptReferral('DEMO-1')).rejects.toThrow(/first trade/);
    expect(pendingReferral()).toBeNull();
  });

  it('dismiss forgets the code without calling the server', () => {
    localStorage.setItem('fxvps.ref', 'NINE');
    dismissReferral();
    expect(pendingReferral()).toBeNull();
    expect(ibLink).not.toHaveBeenCalled();
  });
});
