import { formatMoney, formatPrice } from '@fxvps/trading-core';
import { useLocalSearchParams, useRouter } from 'expo-router';
import { useState } from 'react';
import { ScrollView, StyleSheet, TextInput } from 'react-native';
import { Button, Card, Label, Row, Screen, Segmented } from '../../src/components/ui';
import { evaluateTicket, type MobileOrderType, type TicketForm } from '../../src/lib/ticket';
import { useSettings } from '../../src/state/settings';
import { useTrading } from '../../src/state/trading';

export default function TradeScreen() {
  const { symbol = '' } = useLocalSearchParams<{ symbol: string }>();
  const { palette, t } = useSettings();
  const { api, account, specs, quotes, metrics } = useTrading();
  const router = useRouter();
  const [form, setForm] = useState<TicketForm>({ side: 'buy', type: 'market', lots: '0.10', price: '', sl: '', tp: '' });
  const [result, setResult] = useState<string>();
  const spec = specs[symbol];
  const quote = quotes[symbol];
  if (!spec || !account || !metrics) return <Screen />;

  const ev = evaluateTicket(form, {
    account,
    spec,
    quote,
    freeMargin: metrics.freeMargin,
    specs,
    quotes,
    // No expiry on mobile tickets; quote time is a stable "now" for validation.
    now: quote?.time ?? 0,
  });
  const set = (patch: Partial<TicketForm>) => setForm((f) => ({ ...f, ...patch }));
  const input = [styles.input, { color: palette.text, borderColor: palette.border }];

  const submit = async () => {
    if (!ev.request) return;
    const r = await api.placeOrder(ev.request);
    setResult(r.ok ? t('trade.placed') : `${t('trade.failed')}: ${r.error}`);
    if (r.ok) router.back();
  };

  return (
    <Screen>
      <ScrollView>
        <Card>
          <Label>{symbol}</Label>
          {quote && (
            <Row
              label={`${t('watchlist.bid')} / ${t('watchlist.ask')}`}
              value={`${formatPrice(quote.bid, spec.digits)} / ${formatPrice(quote.ask, spec.digits)}`}
            />
          )}
          <Segmented
            options={[
              { value: 'buy', label: t('trade.buy'), color: palette.up },
              { value: 'sell', label: t('trade.sell'), color: palette.down },
            ]}
            value={form.side}
            onChange={(side) => set({ side })}
          />
          <Segmented<MobileOrderType>
            options={(['market', 'limit', 'stop'] as const).map((v) => ({ value: v, label: t(`trade.type.${v}`) }))}
            value={form.type}
            onChange={(type) => set({ type })}
          />
          <Label muted>{t('trade.volume')}</Label>
          <TextInput testID="ticket-lots" style={input} keyboardType="decimal-pad" value={form.lots} onChangeText={(lots) => set({ lots })} />
          {form.type !== 'market' && (
            <>
              <Label muted>{t('trade.price')}</Label>
              <TextInput style={input} keyboardType="decimal-pad" value={form.price} onChangeText={(price) => set({ price })} />
            </>
          )}
          <Label muted>{t('trade.sl')}</Label>
          <TextInput style={input} keyboardType="decimal-pad" value={form.sl} onChangeText={(sl) => set({ sl })} />
          <Label muted>{t('trade.tp')}</Label>
          <TextInput style={input} keyboardType="decimal-pad" value={form.tp} onChangeText={(tp) => set({ tp })} />
          {ev.requiredMargin !== undefined && <Row label={t('trade.margin')} value={formatMoney(ev.requiredMargin)} />}
          {ev.errors.map((e) => (
            <Label key={`${e.field}-${e.key}`} color={palette.down}>
              {t(`err.${e.key}`)}
            </Label>
          ))}
          <Button
            testID="ticket-submit"
            title={`${t('trade.submit')} (${form.side === 'buy' ? t('trade.buy') : t('trade.sell')})`}
            color={form.side === 'buy' ? palette.up : palette.down}
            disabled={!ev.request}
            onPress={() => void submit()}
          />
          {result && <Label muted>{result}</Label>}
        </Card>
      </ScrollView>
    </Screen>
  );
}

const styles = StyleSheet.create({
  input: { borderWidth: 1, borderRadius: 8, paddingHorizontal: 10, paddingVertical: 8, marginVertical: 4, fontSize: 16 },
});
