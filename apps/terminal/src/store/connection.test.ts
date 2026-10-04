import { beforeEach, describe, expect, it } from 'vitest';
import { isAllowedWsUrl, loadGateway, parseOrigins, resolveGateway, saveGateway, type WsPolicy } from './connection';

describe('gateway selection', () => {
  beforeEach(() => sessionStorage.clear());

  it('defaults to the mock', () => {
    expect(resolveGateway('').gateway).toBeNull();
  });

  it('reads ?api=ws params, persists them and strips the token from the URL', () => {
    const r = resolveGateway('?api=ws&url=ws%3A%2F%2F127.0.0.1%3A9000%2Fws&token=abc.def.ghi');
    expect(r.gateway).toEqual({ url: 'ws://127.0.0.1:9000/ws', token: 'abc.def.ghi' });
    expect(r.cleanedSearch).toBe('?api=ws&url=ws%3A%2F%2F127.0.0.1%3A9000%2Fws');
    expect(loadGateway()).toEqual(r.gateway);
    // A later plain load reuses the stored gateway.
    expect(resolveGateway('').gateway).toEqual(r.gateway);
  });

  it('ignores non-websocket URLs and resets on ?api=mock', () => {
    expect(resolveGateway('?api=ws&url=http://x/ws').gateway).toBeNull();
    saveGateway({ url: 'wss://gw.example/ws', token: 't' });
    expect(resolveGateway('?api=mock').gateway).toBeNull();
    expect(loadGateway()).toBeNull();
  });

  it('denies ?url= links to gateways outside the allow list (token exfiltration)', () => {
    const prod: WsPolicy = { allowed: ['wss://gw.fxvps.ai'], pageOrigin: 'https://terminal.fxvps.ai', dev: false };
    const r = resolveGateway('?api=ws&url=wss%3A%2F%2Fevil.example%2Fws', prod);
    expect(r.gateway).toBeNull();
    expect(loadGateway()).toBeNull();
    expect(resolveGateway('?api=ws&url=wss%3A%2F%2Fgw.fxvps.ai%2Fws', prod).gateway?.url).toBe('wss://gw.fxvps.ai/ws');
  });
});

describe('gateway allow list', () => {
  const prod: WsPolicy = { allowed: parseOrigins(' wss://gw.fxvps.ai/ , wss://gw2.fxvps.ai'), pageOrigin: 'https://terminal.fxvps.ai', dev: false };
  it('allows listed origins and the page origin only', () => {
    expect(isAllowedWsUrl('wss://gw.fxvps.ai/ws', prod)).toBe(true);
    expect(isAllowedWsUrl('wss://gw2.fxvps.ai/ws', prod)).toBe(true);
    expect(isAllowedWsUrl('wss://terminal.fxvps.ai/ws', prod)).toBe(true);
    expect(isAllowedWsUrl('ws://terminal.fxvps.ai/ws', prod)).toBe(false);
    expect(isAllowedWsUrl('wss://gw.fxvps.ai.evil.example/ws', prod)).toBe(false);
    expect(isAllowedWsUrl('wss://evil.example/ws', prod)).toBe(false);
    expect(isAllowedWsUrl('ws://127.0.0.1:9000/ws', prod)).toBe(false);
    expect(isAllowedWsUrl('https://gw.fxvps.ai/ws', prod)).toBe(false);
    expect(isAllowedWsUrl('not a url', prod)).toBe(false);
  });
  it('allows loopback gateways in dev builds or on loopback pages', () => {
    expect(isAllowedWsUrl('ws://127.0.0.1:9000/ws', { ...prod, dev: true })).toBe(true);
    expect(isAllowedWsUrl('ws://localhost:9000/ws', { allowed: [], pageOrigin: 'http://localhost:4173', dev: false })).toBe(true);
    expect(isAllowedWsUrl('wss://evil.example/ws', { allowed: [], pageOrigin: 'http://localhost:4173', dev: true })).toBe(false);
  });
});
