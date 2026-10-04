import { beforeEach, describe, expect, it } from 'vitest';
import { loadGateway, resolveGateway, saveGateway } from './connection';

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
});
