// ============================================================
// rules.ts — фронтенд-зеркало канонической валидации (validate.rs)
// ============================================================
// Предсказывает ошибки для UX, но финальный барьер — бэкенд:
// start_project_execution отказывается выполнять невалидный стек.
// ПРАВИЛА ДОЛЖНЫ СОВПАДАТЬ С validate.rs — меняй обе стороны.
//
// Правила (все — severity=Error, кроме 6 — Warning):
//   1. Фреймворк доступен на текущей ОС (platforms).
//   2. Взаимные конфликты из wizard_tree (conflicts + conflict_notes).
//   3. Тип проекта разрешает фреймворк (project_types).
//   4. На каждую сторону — не более одного «главного» фреймворка
//      (kind="app", side != "either"). Исключения — data-driven:
//      allowed_main_pairs (gin+cobra, axum+clap, android+jetpack-compose,
//      electron+react/vue/svelte) и main_limit_exempt (zig-cli).
//      Побочные (aiogram, telegraf) и универсальные (tauri, qt) этим
//      правилом не ограничены.
//   5. Язык(и) стороны совместимы с фреймворком (side + languages).
//   6. Предупреждения из warning_pairs (Phoenix LiveView + SPA, два
//      full-stack фреймворка, backend + Electron).
//   Инструменты (зеркало Session 2 validate.rs):
//     8. зависимость (tool.requires) не выбрана — Error;
//     9. несовпадение языков (tool.for_languages) — Error
//        (пустой список = универсальный инструмент);
//    10. пересечение ответственностей (responsibility + alternative_policy):
//        exclusive — Error, warn — Warning, allow — допустимо; при
//        расхождении политик действует более строгая из двух;
//    11. связки фреймворк↔инструмент: tool_conflicts — Error,
//        tool_warnings — Warning (причина + рекомендация).

import { i18n } from "$lib/core/i18n.svelte";
import type { TranslationKey } from "$lib/core/i18n.svelte";
import type {
  WizardTreeData,
  FrameworkDef,
  ToolDef,
  StackIssue,
  StackSeverity,
} from "./types";

export type Severity = StackSeverity;

/** Один балл блокировки/предупреждения выбора фреймворка */
export type BlockedReason = {
  severity: Severity;
  message: string;
};

function isAllowedPair(tree: WizardTreeData, a: string, b: string): boolean {
  return tree.allowed_main_pairs.some(
    (p) => (p[0] === a && p[1] === b) || (p[0] === b && p[1] === a),
  );
}

function isMainLimitExempt(tree: WizardTreeData, id: string): boolean {
  return tree.main_limit_exempt.includes(id);
}

/** Объяснение конфликта (conflict_notes) в обе стороны.
 *  Значения conflict_notes теперь i18n-ключи — переводим через i18n.t(). */
function conflictNote(
  tree: WizardTreeData,
  fw: FrameworkDef,
  other: FrameworkDef,
): string | undefined {
  const key = fw.conflict_notes?.[other.id] ?? other.conflict_notes?.[fw.id];
  if (!key) return undefined;
  return i18n.t(key as TranslationKey);
}

function platformOk(fw: FrameworkDef, os: string): boolean {
  return !fw.platforms?.length || fw.platforms.some((p) => p === os);
}

/** Подстановка плейсхолдеров в текстах warning_pairs:
 *  {a} — label фреймворка a, {b} — label фреймворка b,
 *  {a_lang} — label рекомендованного языка фреймворка a.
 *  Текст в warning_pairs теперь i18n-ключ — сначала переводим, потом
 *  подставляем плейсхолдеры (они остаются в переведённой строке). */
export function renderWarningPairText(
  tree: WizardTreeData,
  text: string,
  a: FrameworkDef,
  b: FrameworkDef,
): string {
  const aLangLabel =
    tree.languages.find((l) => l.id === a.recommended_language)?.label ??
    a.recommended_language;
  return i18n.t(text as TranslationKey)
    .replaceAll("{a_lang}", aLangLabel)
    .replaceAll("{a}", a.label)
    .replaceAll("{b}", b.label);
}

export function validateStack(
  tree: WizardTreeData,
  projectType: string | null,
  backendLangs: string[],
  frontendLangs: string[],
  frameworks: string[],
  tools: string[],
  os: string,
): StackIssue[] {
  const issues: StackIssue[] = [];

  const selected: FrameworkDef[] = frameworks
    .map((id) => tree.frameworks.find((f) => f.id === id))
    .filter((f): f is FrameworkDef => !!f);

  // 1. Платформа
  for (const fw of selected) {
    if (!platformOk(fw, os)) {
      issues.push({
        severity: "Error",
        message: i18n.t("stack.platform", {
          a: fw.label,
          list: fw.platforms!.join(", "),
        }),
      });
    }
  }

  // 2. Взаимные конфликты
  for (const a of selected) {
    for (const b of selected) {
      if (a.id === b.id) continue;
      if (a.conflicts?.includes(b.id)) {
        let message = i18n.t("stack.conflict", { a: a.label, b: b.label });
        const note = conflictNote(tree, a, b);
        if (note) message += ` ${note}`;
        issues.push({
          severity: "Error",
          message,
        });
      }
    }
  }

  // 3. Тип проекта
  if (projectType) {
    for (const fw of selected) {
      if (fw.project_types?.length && !fw.project_types.includes(projectType)) {
        issues.push({
          severity: "Error",
          message: i18n.t("stack.project_type", { a: fw.label, pt: projectType }),
        });
      }
    }
  }

  // 4. Не более одного «главного» (kind="app", side != "either") фреймворка
  //    на сторону. Исключения: allowed_main_pairs (легальные связки —
  //    gin+cobra, axum+clap, android+jetpack-compose, electron+react/vue/svelte)
  //    и main_limit_exempt (zig-cli). Универсальные (tauri, qt) и побочные
  //    (aiogram, telegraf) в лимит сторон не входят — только явные conflicts.
  const bySide: { side: string; fw: FrameworkDef }[] = [];
  for (const fw of selected) {
    if (fw.kind === "app" && fw.side !== "either" && !isMainLimitExempt(tree, fw.id)) {
      bySide.push({ side: fw.side, fw });
    }
  }
  for (let i = 0; i < bySide.length; i++) {
    for (let j = i + 1; j < bySide.length; j++) {
      const a = bySide[i];
      const b = bySide[j];
      if (a.side === b.side) {
        if (isAllowedPair(tree, a.fw.id, b.fw.id)) continue;
        issues.push({
          severity: "Error",
          message: i18n.t("stack.two_main", {
            a: a.fw.label,
            b: b.fw.label,
            side: a.side,
          }),
        });
      }
    }
  }

  // 6. Предупреждения из warning_pairs (Phoenix LiveView + тяжёлый SPA,
  //    Laravel/Spring Boot + Next/Nuxt, backend + Electron...): не блокируют,
  //    но объясняют концептуальный конфликт и советуют альтернативу.
  for (const wp of tree.warning_pairs) {
    const a = selected.find((f) => f.id === wp.a);
    const b = selected.find((f) => f.id === wp.b);
    if (a && b) {
      const reason = renderWarningPairText(tree, wp.reason, a, b);
      const alternative = renderWarningPairText(tree, wp.alternative, a, b);
      issues.push({
        severity: "Warning",
        message: i18n.t("stack.warn_pair", {
          a: a.label,
          b: b.label,
          reason,
          alt: alternative,
        }),
      });
    }
  }

  // 5. Язык(и) стороны должны подходить фреймворку
  for (const fw of selected) {
    if (fw.side === "backend") {
      if (!fw.languages.some((l) => backendLangs.includes(l))) {
        issues.push({
          severity: "Error",
          message: i18n.t("stack.backend_lang", {
            a: fw.label,
            list: fw.languages.join(", "),
            rec: fw.recommended_language,
          }),
        });
      }
    } else if (fw.side === "frontend") {
      if (!fw.languages.some((l) => frontendLangs.includes(l))) {
        issues.push({
          severity: "Error",
          message: i18n.t("stack.frontend_lang", {
            a: fw.label,
            list: fw.languages.join(", "),
            rec: fw.recommended_language,
          }),
        });
      }
    } else {
      const anyOk = fw.languages.some((l) => backendLangs.includes(l) || frontendLangs.includes(l));
      if (!anyOk) {
        issues.push({
          severity: "Error",
          message: i18n.t("stack.either_lang", {
            a: fw.label,
            list: fw.languages.join(", "),
          }),
        });
      }
    }
  }

  // 7. UI-варианты фреймворка (qt-qml/qt-widgets/qt-webengine/qt-kirigami)
  //    не могут существовать без своего владельца (qt).
  for (const fw of selected) {
    const owner = tree.frameworks.find((f) =>
      (f.qt_ui_options ?? []).some((m) => m.id === fw.id),
    );
    if (owner && !selected.some((x) => x.id === owner.id)) {
      issues.push({
        severity: "Error",
        message: i18n.t("stack.ui_owner", {
          a: fw.label,
          b: owner.label,
        }),
      });
    }
  }

  // 8. Зависимости инструментов (tool.requires[]): alembic требует
  //    sqlalchemy. Если требуемый инструмент не выбран — Error.
  const selectedToolIds = new Set(tools);
  for (const toolId of tools) {
    const tool = tree.tools.find((t) => t.id === toolId);
    if (!tool) continue;
    for (const requiredId of tool.requires) {
      if (selectedToolIds.has(requiredId)) continue;
      const requiredLabel =
        tree.tools.find((t) => t.id === requiredId)?.label ?? requiredId;
      const args = { tool: tool.label, required: requiredLabel };
      issues.push({
        severity: "Error",
        message_key: "stack.tool.missing_dependency",
        args,
        message: i18n.t("stack.tool.missing_dependency", args),
      });
    }
  }

  // 9. Языковая совместимость инструмента (tool.for_languages[]). Пустой
  //    список — универсальный инструмент. Несовпадение — Error.
  const languages = [...backendLangs, ...frontendLangs];
  for (const toolId of tools) {
    const tool = tree.tools.find((t) => t.id === toolId);
    if (!tool || tool.for_languages.length === 0) continue;
    const compatible = tool.for_languages.some((l) => languages.includes(l));
    if (compatible) continue;
    const args = { tool: tool.label, languages: tool.for_languages.join(", ") };
    issues.push({
      severity: "Error",
      message_key: "stack.tool.language_mismatch",
      args,
      message: i18n.t("stack.tool.language_mismatch", args),
    });
  }

  // 10. Пересечение ответственностей (tool.responsibility + alternative_policy).
  //     exclusive — Error, warn — Warning, allow — допустимо; при расхождении
  //     политик действует более строгая из двух.
  const selectedTools: ToolDef[] = tools
    .map((id) => tree.tools.find((t) => t.id === id))
    .filter((t): t is ToolDef => !!t);
  const byResponsibility = new Map<string, ToolDef[]>();
  for (const tool of selectedTools) {
    if (!tool.responsibility) continue;
    const group = byResponsibility.get(tool.responsibility) ?? [];
    group.push(tool);
    byResponsibility.set(tool.responsibility, group);
  }
  for (const [responsibility, group] of byResponsibility) {
    if (group.length <= 1) continue;
    for (let i = 0; i < group.length; i++) {
      for (let j = i + 1; j < group.length; j++) {
        const a = group[i];
        const b = group[j];
        const policyA = a.alternative_policy ?? "allow";
        const policyB = b.alternative_policy ?? "allow";
        const effective =
          policyA === "exclusive" || policyB === "exclusive"
            ? "exclusive"
            : policyA === "warn" || policyB === "warn"
              ? "warn"
              : "allow";
        if (effective === "exclusive") {
          const args = { a: a.label, b: b.label, responsibility };
          issues.push({
            severity: "Error",
            message_key: "stack.tool.exclusive_alternatives",
            args,
            message: i18n.t("stack.tool.exclusive_alternatives", args),
          });
        } else if (effective === "warn") {
          const args = { a: a.label, b: b.label, responsibility };
          issues.push({
            severity: "Warning",
            message_key: "stack.tool.overlapping_responsibility",
            args,
            message: i18n.t("stack.tool.overlapping_responsibility", args),
          });
        }
      }
    }
  }

  // 11. Связки фреймворк↔инструмент: tool_conflicts — жёсткая
  //     несовместимость (Error); tool_warnings — Warning с причиной и
  //     рекомендацией (показываются отдельной строкой из args).
  for (const fw of selected) {
    for (const toolId of tools) {
      if ((fw.tool_conflicts ?? []).includes(toolId)) {
        const toolLabel =
          tree.tools.find((t) => t.id === toolId)?.label ?? toolId;
        const args = { framework: fw.label, tool: toolLabel };
        issues.push({
          severity: "Error",
          message_key: "stack.framework_tool_conflict",
          args,
          message: i18n.t("stack.framework_tool_conflict", args),
        });
      }
      const warning = fw.tool_warnings?.[toolId];
      if (warning) {
        const toolLabel =
          tree.tools.find((t) => t.id === toolId)?.label ?? toolId;
        const args = {
          framework: fw.label,
          tool: toolLabel,
          reason: warning.reason,
          recommendation: warning.recommendation,
        };
        issues.push({
          severity: "Warning",
          message_key: "stack.framework_tool_warning",
          args,
          message: i18n.t("stack.framework_tool_warning", args),
        });
      }
    }
  }

  return issues;
}

export function firstError(issues: StackIssue[]): string | null {
  const e = issues.find((i) => i.severity === "Error");
  return e ? e.message : null;
}
