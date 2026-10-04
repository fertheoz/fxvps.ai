import { formatMoney } from '@fxvps/trading-core';
import { ScrollView } from 'react-native';
import { LANGS, type Lang } from '../../src/i18n';
import { Card, Label, Row, Screen, Segmented } from '../../src/components/ui';
import { useSettings } from '../../src/state/settings';
import { useTrading } from '../../src/state/trading';
import type { ThemePref } from '../../src/theme';

const THEMES: readonly ThemePref[] = ['system', 'light', 'dark'];

export default function AccountScreen() {
  const { palette, t, lang, setLang, themePref, setThemePref } = useSettings();
  const { account, metrics } = useTrading();
  return (
    <Screen>
      <ScrollView>
        {account && metrics && (
          <Card>
            <Label>
              {account.name}
              {account.isDemo ? ` · ${t('account.demo')}` : ''}
            </Label>
            <Row label={t('account.balance')} value={formatMoney(metrics.balance)} />
            <Row label={t('account.equity')} value={formatMoney(metrics.equity)} />
            <Row
              label={t('account.floating')}
              value={formatMoney(metrics.floating)}
              color={metrics.floating >= 0 ? palette.up : palette.down}
            />
            <Row label={t('account.margin')} value={formatMoney(metrics.margin)} />
            <Row label={t('account.freeMargin')} value={formatMoney(metrics.freeMargin)} />
            <Row label={t('account.marginLevel')} value={metrics.marginLevel ? `${metrics.marginLevel}%` : '-'} />
            <Row label={t('account.leverage')} value={`1:${account.leverage}`} />
          </Card>
        )}
        <Card>
          <Label muted>{t('settings.theme')}</Label>
          <Segmented
            options={THEMES.map((v) => ({ value: v, label: t(`settings.theme.${v}`) }))}
            value={themePref}
            onChange={setThemePref}
          />
          <Label muted>{t('settings.language')}</Label>
          <Segmented<Lang>
            options={LANGS.map((v) => ({ value: v, label: v.toUpperCase() }))}
            value={lang}
            onChange={setLang}
          />
        </Card>
      </ScrollView>
    </Screen>
  );
}
