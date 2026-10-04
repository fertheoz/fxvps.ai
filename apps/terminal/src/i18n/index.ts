import { en, type MessageKey } from './en';
import { tr } from './tr';

export type Lang = 'en' | 'tr';
export type { MessageKey };

const dictionaries: Record<Lang, Record<MessageKey, string>> = { en, tr };

export function translate(lang: Lang, key: MessageKey, vars?: Record<string, string | number>): string {
  const template = dictionaries[lang][key] ?? en[key];
  if (!vars) return template;
  return template.replace(/\{(\w+)\}/g, (_, k: string) => String(vars[k] ?? `{${k}}`));
}
