// ============================================================
// rules.ts — фронтенд-зеркало канонической валидации (validate.rs)
// ============================================================
// Предсказывает ошибки для UX, но финальный барьер — бэкенд:
// start_project_execution отказывается выполнять невалидный стек.
// ПРАВИЛА ДОЛЖНЫ СОВПАДАТЬ С validate.rs — меняй обе стороны.
//
// Правила (все — severity=Error):
//   1. Фреймворк доступен на текущей ОС (platforms).
//   2. Взаимные конфликты из wizard_tree (conflicts).
//   3. Тип проекта разрешает фреймворк (project_types).
//   4. На каждую сторону — не более одного «главного» фреймворка
//      (kind="app", side != "either"). Побочные (aiogram, telegraf)
//      и универсальные (tauri, qt) этим правилом не ограничены.
//   5. Язык(и) стороны совместимы с фреймворком (side + languages).

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
        issues.push({
          severity: "Error",
          message: `«${a.label}» несовместим с «${b.label}».`,
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
  //    на сторону. Универсальные (tauri, qt) и побочные (aiogram, telegraf)
  //    в лимит сторон не входят — только явные conflicts.
  const bySide: { side: string; fw: FrameworkDef }[] = [];
  for (const fw of selected) {
    if (fw.kind === "app" && fw.side !== "either") {
      bySide.push({ side: fw.side, fw });
    }
  }
  for (let i = 0; i < bySide.length; i++) {
    for (let j = i + 1; j < bySide.length; j++) {
      const a = bySide[i];
      const b = bySide[j];
      if (a.side === b.side) {
        issues.push({
          severity: "Error",
          message: `«${a.fw.label}» и «${b.fw.label}» — оба главные фреймворки ${a.side}. На сторону можно выбрать только один главный фреймворк.`,
        });
      }
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
