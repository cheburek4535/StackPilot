const fs = require('fs');

let content = fs.readFileSync('src/lib/core/i18n.svelte.ts', 'utf8');

// Replace imports
content = content.replace(/import en from '\.\/locales\/en';/, `import en from './locales/en';
import es from './locales/es';
import zh from './locales/zh';
import de from './locales/de';
import hi from './locales/hi';
import pt from './locales/pt';
import ja from './locales/ja';`);

// Replace Locale type
content = content.replace(/export type Locale = 'en' | 'ru';/, "export type Locale = 'en' | 'ru' | 'es' | 'zh' | 'de' | 'hi' | 'pt' | 'ja';");

// Replace availableLocales
content = content.replace(/export const availableLocales = \[[\s\S]*?\] as const;/, `export const availableLocales = [
  { id: 'en', name: 'English', icon: '🇬🇧', index: 'EN' },
  { id: 'ru', name: 'Русский', icon: '🇷🇺', index: 'RU' },
  { id: 'es', name: 'Español', icon: '🇪🇸', index: 'ES' },
  { id: 'zh', name: '中文 (简体)', icon: '🇨🇳', index: 'ZH' },
  { id: 'de', name: 'Deutsch', icon: '🇩🇪', index: 'DE' },
  { id: 'hi', name: 'हिन्दी', icon: '🇮🇳', index: 'HI' },
  { id: 'pt', name: 'Português', icon: '🇵🇹', index: 'PT' },
  { id: 'ja', name: '日本語', icon: '🇯🇵', index: 'JA' }
] as const;`);

// Change default locale to 'en'
content = content.replace(/locale = \$state<Locale>\('ru'\);/, "locale = $state<Locale>('en');");

// Update localStorage check
content = content.replace(/if \(saved === 'en' \|\| saved === 'ru'\)/, "if (['en', 'ru', 'es', 'zh', 'de', 'hi', 'pt', 'ja'].includes(saved))");

// Update dict selection in t()
content = content.replace(/const dict = this\.locale === 'en' \? en : ru;/, `const dicts: Record<Locale, TranslationDict> = { en, ru, es, zh, de, hi, pt, ja };
    const dict = dicts[this.locale] || en;`);

fs.writeFileSync('src/lib/core/i18n.svelte.ts', content);
