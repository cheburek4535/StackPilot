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
//   6. Предупреждения из warning_pairs (Phoenix LiveView + SPA).

import type {
  WizardTreeData,
  FrameworkDef,
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

/** Объяснение конфликта (conflict_notes) в обе стороны */
function conflictNote(
  tree: WizardTreeData,
  fw: FrameworkDef,
  other: FrameworkDef,
): string | undefined {
  return fw.conflict_notes?.[other.id] ?? other.conflict_notes?.[fw.id];
}

function platformOk(fw: FrameworkDef, os: string): boolean {
  return !fw.platforms?.length || fw.platforms.some((p) => p === os);
}

export function validateStack(
  tree: WizardTreeData,
  projectType: string | null,
  backendLangs: string[],
  frontendLangs: string[],
  frameworks: string[],
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
        message: `«${fw.label}» недоступен на этой ОС (требуется: ${fw.platforms!.join(", ")}).`,
      });
    }
  }

  // 2. Взаимные конфликты
  for (const a of selected) {
    for (const b of selected) {
      if (a.id === b.id) continue;
      if (a.conflicts?.includes(b.id)) {
        let message = `«${a.label}» несовместим с «${b.label}».`;
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
          message: `«${fw.label}» не подходит для проекта «${projectType}». Выберите другой тип или снимите фреймворк.`,
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
          message: `«${a.fw.label}» и «${b.fw.label}» — оба главные фреймворки ${a.side}. На сторону можно выбрать только один главный фреймворк.`,
        });
      }
    }
  }

  // 6. Предупреждения из warning_pairs (Phoenix LiveView + тяжёлый SPA):
  //    не блокируют, но объясняют концептуальный конфликт и советуют альтернативу.
  for (const wp of tree.warning_pairs) {
    const a = selected.find((f) => f.id === wp.a);
    const b = selected.find((f) => f.id === wp.b);
    if (a && b) {
      let message = `«${a.label}» и «${b.label}» — спорная связка. ${wp.reason}`;
      if (wp.alternative) message += ` Альтернатива: ${wp.alternative}.`;
      issues.push({ severity: "Warning", message });
    }
  }

  // 5. Язык(и) стороны должны подходить фреймворку
  for (const fw of selected) {
    if (fw.side === "backend") {
      if (!fw.languages.some((l) => backendLangs.includes(l))) {
        issues.push({
          severity: "Error",
          message: `«${fw.label}» работает на бэкенде и требует один из языков: ${fw.languages.join(", ")}. Замените бэкенд-язык на «${fw.recommended_language}».`,
        });
      }
    } else if (fw.side === "frontend") {
      if (!fw.languages.some((l) => frontendLangs.includes(l))) {
        issues.push({
          severity: "Error",
          message: `«${fw.label}» работает на фронтенде и требует один из языков: ${fw.languages.join(", ")}. Замените фронтенд-язык на «${fw.recommended_language}».`,
        });
      }
    } else {
      const anyOk = fw.languages.some((l) => backendLangs.includes(l) || frontendLangs.includes(l));
      if (!anyOk) {
        issues.push({
          severity: "Error",
          message: `«${fw.label}» требует один из языков: ${fw.languages.join(", ")} (на любой стороне).`,
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
