// ============================================================
// Персистентность вкладки Create
// ============================================================
// SvelteKit размонтирует страницу при переходе между маршрутами,
// но бэкенд (создание проекта, установка окружения) продолжает работать.
// Чтобы вкладка Create переживала переключение вкладок, её состояние
// сохраняется в sessionStorage (переживает ремаунт компонента, но не
// перезапуск приложения) и восстанавливается при возврате.

const KEY = "stackpilot:create:session:v1";

export type CreateSessionData = Record<string, unknown>;

/** Поля, допустимые в sessionStorage. Всё тяжёлое — терминальные логи,
 *  execution_events, снапшоты выполнения (execLogs, execStatuses, envLogs,
 *  envCheck, envPlan, envTaskStates, envCheckProgress, ...) — сюда НЕ входит:
 *  такие данные живут на бэкенде (EXECUTION_SNAPSHOT / установочная сессия)
 *  и восстанавливаются вкладкой через reSyncLiveSessions().
 *
 *  Хранится только лёгкое состояние мастера: тип проекта, языки, фреймворки,
 *  инструменты, текущий шаг и мелкие строки/булевы поля. JSON.stringify
 *  такого объекта — доли миллисекунды, UI-поток не блокируется. */
const LIGHT_FIELDS = new Set<string>([
  "v",
  "mode",
  "phase",
  "typeId",
  "backendLangs",
  "frontendLangs",
  "manualBackendLangs",
  "manualFrontendLangs",
  "selectedFrameworks",
  "fwLangs",
  "linkedCompanions",
  "qtUiMode",
  "qtWebLinked",
  "selectedTools",
  "testing",
  "git",
  "vscode",
  "projectName",
  "selectedFolder",
  "conflictResolvedFolder",
  "folderExists",
  "envLocalInfra",
  "envSelectedIds",
  "envInstalling",
  "envInstallDone",
  "execOverallStatus",
  "execProjectPath",
  "execPlan",
  "execResult",
]);

export function loadCreateSession(): CreateSessionData | null {
  try {
    const raw = sessionStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as CreateSessionData) : null;
  } catch {
    return null;
  }
}

/** Сохраняет ТОЛЬКО лёгкие поля мастера (whitelist выше). Тяжёлые данные
 *  (логи, execution_events) молча отбрасываются — в sessionStorage они
 *  не попадают никогда, даже если их кто-то передаст сюда. */
export function saveCreateSession(data: CreateSessionData): void {
  try {
    const light: Record<string, unknown> = {};
    for (const key of Object.keys(data)) {
      if (LIGHT_FIELDS.has(key)) light[key] = data[key];
    }
    sessionStorage.setItem(KEY, JSON.stringify(light));
  } catch {
    // sessionStorage переполнен — молча игнорируем
  }
}

export function clearCreateSession(): void {
  try {
    sessionStorage.removeItem(KEY);
  } catch {
    // ignore
  }
}
