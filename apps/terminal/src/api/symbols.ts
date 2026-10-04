import type { SymbolSpec } from './types';

type Seed = SymbolSpec & { start: number; spreadPoints: number; volPoints: number };

const fx = (name: string, description: string, start: number, digits: number, spreadPoints: number, volPoints: number): Seed => ({
  name,
  description,
  base: name.slice(0, 3),
  quote: name.slice(3, 6),
  digits,
  pipSize: digits === 3 ? '0.01' : '0.0001',
  contractSize: 100_000,
  minVolume: 1,
  maxVolume: 10_000,
  volumeStep: 1,
  marginRate: 1,
  category: 'fx',
  start,
  spreadPoints,
  volPoints,
});

/** Instruments offered by the mock backend (start prices are illustrative). */
export const MOCK_SYMBOLS: Seed[] = [
  fx('EURUSD', 'Euro vs US Dollar', 1.085, 5, 6, 4),
  fx('GBPUSD', 'Pound vs US Dollar', 1.27, 5, 9, 5),
  fx('USDJPY', 'US Dollar vs Yen', 149.5, 3, 8, 4),
  fx('AUDUSD', 'Australian Dollar vs US Dollar', 0.66, 5, 8, 3),
  fx('USDCHF', 'US Dollar vs Swiss Franc', 0.88, 5, 10, 3),
  fx('USDCAD', 'US Dollar vs Canadian Dollar', 1.36, 5, 10, 3),
  fx('NZDUSD', 'NZ Dollar vs US Dollar', 0.6, 5, 12, 3),
  fx('EURGBP', 'Euro vs Pound', 0.854, 5, 10, 2),
  fx('EURJPY', 'Euro vs Yen', 162.2, 3, 14, 5),
  {
    name: 'XAUUSD', description: 'Gold vs US Dollar', base: 'XAU', quote: 'USD', digits: 2, pipSize: '0.1',
    contractSize: 100, minVolume: 1, maxVolume: 5_000, volumeStep: 1, marginRate: 1, category: 'metal',
    start: 2350, spreadPoints: 25, volPoints: 30,
  },
  {
    name: 'US500', description: 'S&P 500 Index CFD', base: 'US500', quote: 'USD', digits: 1, pipSize: '0.1',
    contractSize: 1, minVolume: 10, maxVolume: 50_000, volumeStep: 10, marginRate: 1, category: 'index',
    start: 5200, spreadPoints: 5, volPoints: 4,
  },
  {
    name: 'BTCUSD', description: 'Bitcoin vs US Dollar', base: 'BTC', quote: 'USD', digits: 2, pipSize: '1',
    contractSize: 1, minVolume: 1, maxVolume: 1_000, volumeStep: 1, marginRate: 5, category: 'crypto',
    start: 65000, spreadPoints: 2000, volPoints: 1500,
  },
  {
    name: 'ETHUSD', description: 'Ether vs US Dollar', base: 'ETH', quote: 'USD', digits: 2, pipSize: '0.1',
    contractSize: 1, minVolume: 1, maxVolume: 5_000, volumeStep: 1, marginRate: 5, category: 'crypto',
    start: 3200, spreadPoints: 150, volPoints: 120,
  },
];

export function toSpec(s: Seed): SymbolSpec {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  const { start, spreadPoints, volPoints, ...spec } = s;
  return spec;
}
