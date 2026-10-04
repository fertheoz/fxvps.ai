import { Tabs } from 'expo-router';
import { useSettings } from '../../src/state/settings';

export default function TabsLayout() {
  const { palette, t } = useSettings();
  return (
    <Tabs
      screenOptions={{
        headerStyle: { backgroundColor: palette.card },
        headerTintColor: palette.text,
        tabBarStyle: { backgroundColor: palette.card, borderTopColor: palette.border },
        tabBarActiveTintColor: palette.accent,
        tabBarInactiveTintColor: palette.muted,
        tabBarIcon: () => null,
      }}
    >
      <Tabs.Screen name="index" options={{ title: t('tab.watchlist') }} />
      <Tabs.Screen name="positions" options={{ title: t('tab.positions') }} />
      <Tabs.Screen name="account" options={{ title: t('tab.account') }} />
    </Tabs>
  );
}
