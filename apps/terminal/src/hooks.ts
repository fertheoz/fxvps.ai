import { useCallback, useMemo } from 'react';
import { useTerminal, selectActiveAccount, selectPositions } from './store/terminal';
import { translate, type MessageKey } from './i18n';
import { buildRates, computeAccountMetrics, type AccountMetrics, type Rates } from '@fxvps/trading-core';

export function useT() {
  const lang = useTerminal((s) => s.lang);
  return useCallback((key: MessageKey, vars?: Record<string, string | number>) => translate(lang, key, vars), [lang]);
}

export function useRates(): Rates {
  const symbols = useTerminal((s) => s.symbols);
  const quotes = useTerminal((s) => s.quotes);
  return useMemo(() => buildRates(symbols, quotes), [symbols, quotes]);
}

export function useMetrics(): AccountMetrics | null {
  const account = useTerminal(selectActiveAccount);
  const positions = useTerminal(selectPositions);
  const quotes = useTerminal((s) => s.quotes);
  const symbols = useTerminal((s) => s.symbols);
  const rates = useRates();
  return useMemo(
    () => (account ? computeAccountMetrics(account, positions, quotes, symbols, rates) : null),
    [account, positions, quotes, symbols, rates],
  );
}
