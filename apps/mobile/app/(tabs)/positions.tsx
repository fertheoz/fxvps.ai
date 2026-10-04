import { formatMoney, formatPrice, positionProfit, buildRates, volumeToLots } from '@fxvps/trading-core';
import { FlatList, View } from 'react-native';
import { Button, Card, Label, Screen } from '../../src/components/ui';
import { useSettings } from '../../src/state/settings';
import { useTrading } from '../../src/state/trading';

export default function PositionsScreen() {
  const { palette, t } = useSettings();
  const { api, account, positions, quotes, specs } = useTrading();
  const rates = buildRates(specs, quotes);
  return (
    <Screen>
      <FlatList
        data={positions}
        keyExtractor={(p) => p.id}
        ListEmptyComponent={
          <Card>
            <Label muted>{t('positions.empty')}</Label>
          </Card>
        }
        renderItem={({ item: p }) => {
          const spec = specs[p.symbol];
          const q = quotes[p.symbol];
          const pl = spec && q && account ? positionProfit(p, spec, q, account.currency, rates) : 0;
          return (
            <Card>
              <View style={{ flexDirection: 'row', justifyContent: 'space-between' }}>
                <Label>
                  {p.symbol} {p.side === 'buy' ? t('trade.buy') : t('trade.sell')} {volumeToLots(p.volume)}
                </Label>
                <Label color={pl >= 0 ? palette.up : palette.down}>{formatMoney(pl)}</Label>
              </View>
              <Label muted>@ {spec ? formatPrice(p.openPrice, spec.digits) : p.openPrice}</Label>
              <Button
                testID={`close-${p.id}`}
                title={t('positions.close')}
                color={palette.down}
                onPress={() => {
                  void api.closePosition(p.accountId, p.id);
                }}
              />
            </Card>
          );
        }}
      />
    </Screen>
  );
}
