import { Stack } from 'expo-router';
import { StatusBar } from 'expo-status-bar';
import { SettingsProvider, useSettings } from '../src/state/settings';
import { TradingProvider } from '../src/state/trading';

function ThemedStack() {
  const { palette, isDark, t } = useSettings();
  return (
    <>
      <StatusBar style={isDark ? 'light' : 'dark'} />
      <Stack
        screenOptions={{
          headerStyle: { backgroundColor: palette.card },
          headerTintColor: palette.text,
          contentStyle: { backgroundColor: palette.bg },
        }}
      >
        <Stack.Screen name="(tabs)" options={{ headerShown: false }} />
        <Stack.Screen name="chart/[symbol]" options={{ title: t('chart.title') }} />
        <Stack.Screen name="trade/[symbol]" options={{ title: t('trade.title'), presentation: 'modal' }} />
      </Stack>
    </>
  );
}

export default function RootLayout() {
  return (
    <SettingsProvider>
      <TradingProvider>
        <ThemedStack />
      </TradingProvider>
    </SettingsProvider>
  );
}
