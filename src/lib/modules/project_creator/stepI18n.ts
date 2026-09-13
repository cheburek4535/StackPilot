/**
 * Перевод названий шагов генерации Project Creator.
 *
 * Бэкенд (Rust) шлёт названия шагов на английском (`step.label`, `step_name`),
 * где они зашиты строковыми литералами/format! — трогать его не нужно.
 * Локализация выполняется на фронтенде при отображении:
 *   1) точное совпадение English-строки ищется в словарях как ключ
 *      `step.label.<English text>` (en.ts — идентичность, ru.ts — перевод);
 *   2) для динамических названий (`Create src/index.ts`, `Install npm
 *      dependencies (frontend)`) применимы шаблонные ключи `step.tmpl.*`;
 *   3) если ни одно правило не сработало — возвращаем строку как есть.
 */

import { i18n } from "$lib/core/i18n.svelte";

type TemplateRule = {
  test: RegExp;
  key: string;
  vars: (m: RegExpMatchArray) => Record<string, string>;
};

/** Динамические названия шагов. Правила от частного к общему: сначала
 *  точные шаблоны с переменной в конце, затем общий «Create {path}». */
const TEMPLATE_RULES: TemplateRule[] = [
  {
    test: /^Create config dir for (.+)$/,
    key: "step.tmpl.create_config_dir_for",
    vars: (m) => ({ tool: m[1] }),
  },
  {
    test: /^Install npm dependencies \((.+)\)$/,
    key: "step.tmpl.install_npm_deps",
    vars: (m) => ({ wd: m[1] }),
  },
  {
    test: /^Check (.+) toolchain$/,
    key: "step.tmpl.check_toolchain",
    vars: (m) => ({ grp: m[1] }),
  },
  {
    test: /^Validate (.+) package\.json$/,
    key: "step.tmpl.validate_package_json",
    vars: (m) => ({ fw: m[1] }),
  },
  {
    test: /^Fix package\.json name for (.+)$/,
    key: "step.tmpl.fix_package_name",
    vars: (m) => ({ fw: m[1] }),
  },
  {
    test: /^Create (.+) app$/,
    key: "step.tmpl.create_app",
    vars: (m) => ({ fw: m[1] }),
  },
  {
    test: /^Create (.+)$/,
    key: "step.tmpl.create_path",
    vars: (m) => ({ path: m[1] }),
  },
];

/** Перевести название шага генерации (label / step_name) в язык UI. */
export function tStepLabel(label: string): string {
  if (!label) return label;
  const exactKey = `step.label.${label}`;
  const direct = i18n.t(exactKey);
  if (direct !== exactKey) return direct;
  for (const rule of TEMPLATE_RULES) {
    const match = label.match(rule.test);
    if (match) return i18n.t(rule.key, rule.vars(match));
  }
  return label;
}