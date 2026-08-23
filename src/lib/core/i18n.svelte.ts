import { browser } from '$app/environment';
import ru from './locales/ru';
import en from './locales/en';

export type Locale = 'en' | 'ru';
export const availableLocales = [
  { id: 'en', name: 'English', icon: '🇬🇧', index: 'EN' },
  { id: 'ru', name: 'Русский', icon: '🇷🇺', index: 'RU' }
] as const;

export type TranslationDict = typeof ru;
export type TranslationKey = string;

class I18nService {
  locale = $state<Locale>('en');

  constructor() {
    if (browser) {
      const saved = localStorage.getItem('sp-locale') as Locale;
      if (['en', 'ru'].includes(saved)) {
        this.locale = saved;
      }
    }
  }

  setLocale(l: Locale) {
    this.locale = l;
    if (browser) {
      localStorage.setItem('sp-locale', l);
    }
  }

  t(key: string, vars?: Record<string, string | number>) {
    const dicts: Record<Locale, TranslationDict> = { en, ru };
    const dict = dicts[this.locale] || en;
    let text = (dict as Record<string, string>)[key] || (en as Record<string, string>)[key] || key;
    if (vars) {
      for (const [k, v] of Object.entries(vars)) {
        // Поддерживаем оба стиля плейсхолдеров: {{k}} и {k}.
        text = text.replace(new RegExp(`\\{\\{${k}\\}\\}`, 'g'), String(v));
        text = text.replace(new RegExp(`\\{${k}\\}`, 'g'), String(v));
      }
    }
    return text;
  }
}

export const i18n = new I18nService();
