import { en, type MessageKey } from './en';
import { tr } from './tr';

export type Lang = 'en' | 'tr';
export const LANGS: readonly Lang[] = ['en', 'tr'];
export const messages: Record<Lang, Record<MessageKey, string>> = { en, tr };
export type { MessageKey };

export function translate(lang: Lang, key: MessageKey): string {
  return messages[lang][key] ?? en[key];
}

/** Picks a supported language from a BCP-47 tag list (e.g. expo-localization). */
export function pickLang(tags: readonly (string | null | undefined)[]): Lang {
  for (const tag of tags) {
    const code = tag?.slice(0, 2).toLowerCase();
    if (code === 'tr' || code === 'en') return code;
  }
  return 'en';
}
