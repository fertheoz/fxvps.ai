import AsyncStorage from '@react-native-async-storage/async-storage';
import { getLocales } from 'expo-localization';
import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';
import { useColorScheme } from 'react-native';
import { pickLang, translate, type Lang, type MessageKey } from '../i18n';
import { resolvePalette, type Palette, type ThemePref } from '../theme';

interface SettingsValue {
  lang: Lang;
  setLang: (l: Lang) => void;
  themePref: ThemePref;
  setThemePref: (t: ThemePref) => void;
  palette: Palette;
  isDark: boolean;
  t: (key: MessageKey) => string;
}

const KEY = 'fxvps.settings.v1';
const SettingsContext = createContext<SettingsValue | null>(null);

export function SettingsProvider({ children }: { children: ReactNode }) {
  const system = useColorScheme();
  const [lang, setLangState] = useState<Lang>(() => pickLang(getLocales().map((l) => l.languageTag)));
  const [themePref, setThemeState] = useState<ThemePref>('system');

  useEffect(() => {
    AsyncStorage.getItem(KEY)
      .then((raw) => {
        if (!raw) return;
        const saved = JSON.parse(raw) as Partial<{ lang: Lang; themePref: ThemePref }>;
        if (saved.lang === 'en' || saved.lang === 'tr') setLangState(saved.lang);
        if (saved.themePref === 'light' || saved.themePref === 'dark' || saved.themePref === 'system') {
          setThemeState(saved.themePref);
        }
      })
      .catch(() => undefined);
  }, []);

  const persist = useCallback((next: { lang: Lang; themePref: ThemePref }) => {
    AsyncStorage.setItem(KEY, JSON.stringify(next)).catch(() => undefined);
  }, []);

  const value = useMemo<SettingsValue>(() => {
    const palette = resolvePalette(themePref, system === 'unspecified' ? null : system);
    return {
      lang,
      themePref,
      palette,
      isDark: palette.bg === resolvePalette('dark', null).bg,
      setLang: (l) => {
        setLangState(l);
        persist({ lang: l, themePref });
      },
      setThemePref: (tp) => {
        setThemeState(tp);
        persist({ lang, themePref: tp });
      },
      t: (key) => translate(lang, key),
    };
  }, [lang, themePref, system, persist]);

  return <SettingsContext.Provider value={value}>{children}</SettingsContext.Provider>;
}

export function useSettings(): SettingsValue {
  const v = useContext(SettingsContext);
  if (!v) throw new Error('useSettings outside SettingsProvider');
  return v;
}
