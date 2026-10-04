import { applyTick, formatPrice, TIMEFRAME_SECONDS, type Bar, type Timeframe } from '@fxvps/trading-core';
import { Stack, useLocalSearchParams, useRouter } from 'expo-router';
import { useEffect, useState } from 'react';
import { View } from 'react-native';
import { CandleChart } from '../../src/components/CandleChart';
import { Button, Card, Label, Screen, Segmented } from '../../src/components/ui';
import { useSettings } from '../../src/state/settings';
import { useTrading } from '../../src/state/trading';

const TFS: readonly Timeframe[] = ['M1', 'M5', 'M15', 'H1', 'D1'];

export default function ChartScreen() {
  const { symbol = '' } = useLocalSearchParams<{ symbol: string }>();
  const { palette, t } = useSettings();
  const { api, quotes, specs } = useTrading();
  const router = useRouter();
  const [tf, setTf] = useState<Timeframe>('M5');
  const [bars, setBars] = useState<Bar[]>([]);
  const quote = quotes[symbol];
  const spec = specs[symbol];

  useEffect(() => {
    let alive = true;
    api
      .getBars(symbol, tf, 80)
      .then((b) => alive && setBars(b))
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [api, symbol, tf]);

  // Live candle: subscribe to this symbol's ticks and fold them into the last bar.
  useEffect(
    () =>
      api.subscribeQuotes([symbol], (qs) => {
        const q = qs[qs.length - 1];
        if (!q) return;
        setBars((prev) => {
          const last = prev[prev.length - 1];
          if (!last) return prev;
          const next = applyTick(last, q.bid, Math.floor(q.time / 1000), TIMEFRAME_SECONDS[tf]);
          return next.time === last.time ? [...prev.slice(0, -1), next] : [...prev.slice(-79), next];
        });
      }),
    [api, symbol, tf],
  );

  return (
    <Screen>
      <Stack.Screen options={{ title: symbol }} />
      <Segmented options={TFS.map((v) => ({ value: v, label: v }))} value={tf} onChange={setTf} />
      <CandleChart bars={bars} />
      <Card>
        {quote && spec && (
          <View style={{ flexDirection: 'row', justifyContent: 'space-between' }}>
            <Label color={palette.down}>{formatPrice(quote.bid, spec.digits)}</Label>
            <Label color={palette.up}>{formatPrice(quote.ask, spec.digits)}</Label>
          </View>
        )}
        <Button
          testID="chart-trade"
          title={t('chart.trade')}
          onPress={() => router.push({ pathname: '/trade/[symbol]', params: { symbol } })}
        />
      </Card>
    </Screen>
  );
}
