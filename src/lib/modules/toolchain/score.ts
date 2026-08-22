// ============================================================
// Toolchain — прозрачная разбивка оценки окружения (score)
// ============================================================
// Оценка — ЗДОРОВЬЕ окружения, а не «обязательный набор»: знаменатель
// считает только ПРИМЕНИМЫЕ инструменты каталога (зеркало backend
// domain/score.rs). Вклад: здоровый → 100, устаревший (update) → 50,
// отсутствующий/сломанный/нездоровый → 0. Непроверенные, неприменимые,
// ручные, docker и bundled-дети корректно исключаются БЕЗ штрафа.
//
// Никаких «N обязательных инструментов» здесь нет и быть не может:
// это поле появилось бы только из реального пользовательского профиля
// (Build Environment), а не из формулы.

import type { EnvironmentSnapshot, ToolScanResult } from "./types";
import { isRecord } from "./types";
import { toolStateInfo, type InfoTone } from "./format";

export type ScoreContribution = {
  tool_id: string;
  display: string;
  /** Подпись состояния (тотальный форматтер, никогда не падает). */
  label: string;
  tone: InfoTone;
  /** 100 / 50 / 0 — вклад в числитель. */
  contribution: number;
  /** true — инструмент вошёл в знаменатель формулы. */
  counted: boolean;
};

export type ScoreBreakdown = {
  /** Итоговая оценка (пересчитана тем же правилом, что бэкенд). */
  score: number;
  /** Применимые инструменты в знаменателе. */
  applicable: number;
  healthy: number;
  /** Деградация/устарели (update available). */
  degraded: number;
  /** Сломаны PATH + нездоровы. */
  unhealthy: number;
  missing: number;
  /** Не проверено: не успели, ошибка опроса, проверок нет. */
  unchecked: number;
  /** Неприменимо: чужая платформа / manual / docker / встроенные. */
  not_applicable: number;
  /** Исключены как bundled-дети (корректно обеспечены родителем). */
  excluded_bundled: number;
  /** Полный список с вкладами (для поповера «как посчитано»). */
  contributions: ScoreContribution[];
};

/** Классификация одного инструмента (зеркало backend classify). */
export function classifyScoreTool(tool: ToolScanResult): {
  counted: boolean;
  contribution: number;
  bucket:
    | "healthy"
    | "degraded"
    | "unhealthy"
    | "missing"
    | "unchecked"
    | "not_applicable"
    | "excluded_bundled";
} {
  // Граница IPC: инструмент без читаемого состояния не участвует в
  // формуле (неизвестное ≠ плохо).
  if (!isRecord(tool?.state)) {
    return { counted: false, contribution: 0, bucket: "unchecked" };
  }
  const s = tool.state;
  // Версия в реальном снапшоте у installed_*/update_available НЕ пустая
  // (бэкенд берёт её из рабочей улики). Пустой вариант — «живая»
  // заготовка идущего скана (регрессия «оценка растёт до финала»):
  // такие инструменты не считаются, пока не пришли факты.
  const versionKnown =
    s.kind === "installed_healthy" ||
    s.kind === "installed_unhealthy" ||
    s.kind === "installed_health_unknown"
      ? !!s.version
      : s.kind === "update_available"
        ? !!s.installed
        : true;
  if (!versionKnown) {
    return { counted: false, contribution: 0, bucket: "unchecked" };
  }
  switch (s.kind) {
    case "scan_pending":
    case "scan_failed":
    case "installed_health_unknown":
      return { counted: false, contribution: 0, bucket: "unchecked" };
    case "docker_managed":
    case "manual_install":
    case "built_in_system":
    case "unsupported_platform":
      return { counted: false, contribution: 0, bucket: "not_applicable" };
    case "missing":
      if (tool.bundled_with) {
        return { counted: false, contribution: 0, bucket: "excluded_bundled" };
      }
      return { counted: true, contribution: 0, bucket: "missing" };
    case "path_broken":
    case "installed_unhealthy":
      return { counted: true, contribution: 0, bucket: "unhealthy" };
    case "installed_healthy":
      return { counted: true, contribution: 100, bucket: "healthy" };
    case "update_available":
      return { counted: true, contribution: 50, bucket: "degraded" };
    default:
      return { counted: false, contribution: 0, bucket: "not_applicable" };
  }
}

/**
 * Считает полную разбивку по снапшоту. Итоговый score пересчитывается
 * тем же правилом, что и бэкенд (score = round(Σ / counted × 100)).
 * score бэкенда в снапшоте остаётся авторитетным для Hero; разбивка
 * используется для честного объяснения.
 */
export function scoreBreakdown(snapshot: EnvironmentSnapshot | null): ScoreBreakdown {
  const empty: ScoreBreakdown = {
    score: 0,
    applicable: 0,
    healthy: 0,
    degraded: 0,
    unhealthy: 0,
    missing: 0,
    unchecked: 0,
    not_applicable: 0,
    excluded_bundled: 0,
    contributions: [],
  };
  if (!snapshot) return empty;

  const tools = snapshot.tools ?? [];
  const contributions: ScoreContribution[] = [];
  let earned = 0;
  let counted = 0;
  const buckets = {
    healthy: 0,
    degraded: 0,
    unhealthy: 0,
    missing: 0,
    unchecked: 0,
    not_applicable: 0,
    excluded_bundled: 0,
  };

  for (const tool of tools) {
    // Граница IPC: нечитаемый инструмент не роняет разбивку (skip).
    if (!isRecord(tool)) continue;
    const c = classifyScoreTool(tool as ToolScanResult);
    earned += c.contribution;
    if (c.counted) counted += 1;
    buckets[c.bucket] += 1;
    const state = (tool as ToolScanResult).state;
    const info = isRecord(state)
      ? toolStateInfo(state as ToolScanResult["state"])
      : { label: "Неизвестное состояние", tone: "neutral" as InfoTone };
    contributions.push({
      tool_id: (tool as ToolScanResult).tool_id,
      display: (tool as ToolScanResult).display || (tool as ToolScanResult).tool_id,
      label: info.label,
      tone: info.tone,
      contribution: c.contribution,
      counted: c.counted,
    });
  }

  const score = counted === 0 ? 0 : Math.min(100, Math.round((earned * 100) / (counted * 100)));

  return {
    score,
    applicable: counted,
    healthy: buckets.healthy,
    degraded: buckets.degraded,
    unhealthy: buckets.unhealthy,
    missing: buckets.missing,
    unchecked: buckets.unchecked,
    not_applicable: buckets.not_applicable,
    excluded_bundled: buckets.excluded_bundled,
    contributions,
  };
}

/** Человекочитаемая формула (для поповера объяснения). */
export const SCORE_FORMULA_TEXT =
  "Оценка — это здоровье окружения." +
  " В формуле участвуют только применимые инструменты:" +
  " здоровый даёт 100, устаревший (доступно обновление) — 50," +
  " отсутствующий, сломанный или нездоровый — 0." +
  " Непроверенные, неприменимые, ручные и bundled-инструменты" +
  " оценку не снижают.";