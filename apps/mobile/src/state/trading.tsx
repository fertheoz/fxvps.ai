import {
  computeAccountMetrics,
  MockTradingApi,
  type Account,
  type AccountMetrics,
  type ConnectionState,
  type Position,
  type Quote,
  type SymbolSpec,
  type TradingApi,
} from '@fxvps/trading-core';
import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';

interface TradingValue {
  api: TradingApi;
  connection: ConnectionState;
  account: Account | undefined;
  specs: Record<string, SymbolSpec>;
  symbols: string[];
  quotes: Record<string, Quote>;
  positions: Position[];
  metrics: AccountMetrics | undefined;
}

const TradingContext = createContext<TradingValue | null>(null);

/** Quote updates are coalesced to this interval to keep RN renders cheap. */
const FLUSH_MS = 250;

export function TradingProvider({ children, api: injected }: { children: ReactNode; api?: TradingApi }) {
  const api = useMemo<TradingApi>(() => injected ?? new MockTradingApi({ seed: 7 }), [injected]);
  const [connection, setConnection] = useState<ConnectionState>('connecting');
  const [account, setAccount] = useState<Account>();
  const [specs, setSpecs] = useState<Record<string, SymbolSpec>>({});
  const [quotes, setQuotes] = useState<Record<string, Quote>>({});
  const [positions, setPositions] = useState<Position[]>([]);
  const pending = useRef<Record<string, Quote>>({});

  useEffect(() => {
    let alive = true;
    let unsubQuotes: (() => void) | undefined;
    const accountIdRef = { current: '' };
    const unsubEvents = api.onEvent((e) => {
      if (e.type === 'connection') setConnection(e.state);
      else if (e.type === 'account') setAccount((a) => (a && a.id !== e.account.id ? a : e.account));
      else if (e.type === 'positions') setPositions((p) => (e.accountId === accountIdRef.current ? e.positions : p));
    });
    const timer = setInterval(() => {
      const batch = pending.current;
      if (Object.keys(batch).length === 0) return;
      pending.current = {};
      setQuotes((q) => ({ ...q, ...batch }));
    }, FLUSH_MS);

    (async () => {
      await api.connect();
      const [accounts, symbolSpecs] = await Promise.all([api.getAccounts(), api.getSymbols()]);
      if (!alive) return;
      const first = accounts[0];
      if (first) {
        accountIdRef.current = first.id;
        setAccount(first);
      }
      setSpecs(Object.fromEntries(symbolSpecs.map((s) => [s.name, s])));
      setConnection('connected');
      unsubQuotes = api.subscribeQuotes(
        symbolSpecs.map((s) => s.name),
        (qs) => {
          for (const q of qs) pending.current[q.symbol] = q;
        },
      );
    })().catch(() => setConnection('disconnected'));

    return () => {
      alive = false;
      unsubEvents();
      unsubQuotes?.();
      clearInterval(timer);
      api.disconnect();
    };
  }, [api]);

  const value = useMemo<TradingValue>(() => {
    const metrics = account ? computeAccountMetrics(account, positions, quotes, specs) : undefined;
    return { api, connection, account, specs, symbols: Object.keys(specs), quotes, positions, metrics };
  }, [api, connection, account, specs, quotes, positions]);

  return <TradingContext.Provider value={value}>{children}</TradingContext.Provider>;
}

export function useTrading(): TradingValue {
  const v = useContext(TradingContext);
  if (!v) throw new Error('useTrading outside TradingProvider');
  return v;
}
