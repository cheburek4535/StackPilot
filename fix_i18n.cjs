const fs = require('fs');

const path = 'src/lib/core/i18n.svelte.ts';
let content = fs.readFileSync(path, 'utf8');

// I need to change the type TranslationKey from keyof TranslationDict to string
content = content.replace(/export type TranslationKey = keyof TranslationDict;/, 'export type TranslationKey = string;');
// And the signature
content = content.replace(/t\(key: TranslationKey, vars\?: Record<string, string \| number>\) {/, 't(key: string, vars?: Record<string, string | number>) {');
fs.writeFileSync(path, content);
