import { formatPrice, spreadPoints } from '@fxvps/trading-core';
import { useRouter } from 'expo-router';
import { FlatList, Pressable, StyleSheet, Text, View } from 'react-native';
import { Screen } from '../../src/components/ui';
import { useSettings } from '../../src/state/settings';
import { useTrading } from '../../src/state/trading';

export default function WatchlistScreen() {
  const { palette, t } = useSettings();
  const { symbols, specs, quotes, connection } = useTrading();
  const router = useRouter();
  return (
    <Screen>
      <Text style={[styles.status, { color: connection === 'connected' ? palette.up : palette.muted }]}>
        {t(`conn.${connection}`)}
      </Text>
      <View style={[styles.header, { borderColor: palette.border }]}>
        <Text style={[styles.sym, { color: palette.muted }]} />
        <Text style={[styles.cell, { color: palette.muted }]}>{t('watchlist.bid')}</Text>
        <Text style={[styles.cell, { color: palette.muted }]}>{t('watchlist.ask')}</Text>
        <Text style={[styles.small, { color: palette.muted }]}>{t('watchlist.spread')}</Text>
      </View>
      <FlatList
        data={symbols}
        keyExtractor={(s) => s}
        renderItem={({ item }) => {
          const spec = specs[item];
          const q = quotes[item];
          if (!spec) return null;
          const up = q ? q.bid >= q.dayOpen : true;
          return (
            <Pressable
              testID={`watch-${item}`}
              onPress={() => router.push({ pathname: '/chart/[symbol]', params: { symbol: item } })}
              style={[styles.row, { borderColor: palette.border }]}
            >
              <Text style={[styles.sym, { color: palette.text }]}>{item}</Text>
              <Text style={[styles.cell, { color: up ? palette.up : palette.down }]}>
                {q ? formatPrice(q.bid, spec.digits) : '-'}
              </Text>
              <Text style={[styles.cell, { color: up ? palette.up : palette.down }]}>
                {q ? formatPrice(q.ask, spec.digits) : '-'}
              </Text>
              <Text style={[styles.small, { color: palette.muted }]}>{q ? spreadPoints(q, spec.digits) : '-'}</Text>
            </Pressable>
          );
        }}
      />
    </Screen>
  );
}

const styles = StyleSheet.create({
  status: { paddingHorizontal: 12, paddingTop: 8, fontSize: 12 },
  header: { flexDirection: 'row', paddingHorizontal: 12, paddingVertical: 6, borderBottomWidth: StyleSheet.hairlineWidth },
  row: { flexDirection: 'row', paddingHorizontal: 12, paddingVertical: 12, borderBottomWidth: StyleSheet.hairlineWidth },
  sym: { flex: 1.2, fontWeight: '700' },
  cell: { flex: 1, textAlign: 'right', fontVariant: ['tabular-nums'] },
  small: { flex: 0.6, textAlign: 'right' },
});
