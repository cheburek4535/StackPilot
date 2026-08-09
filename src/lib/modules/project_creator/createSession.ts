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

export function loadCreateSession(): CreateSessionData | null {
  try {
    const raw = sessionStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as CreateSessionData) : null;
  } catch {
    return null;
  }
}

export function saveCreateSession(data: CreateSessionData): void {
  try {
    sessionStorage.setItem(KEY, JSON.stringify(data));
  } catch {
    // sessionStorage может быть переполнен огромными логами — молча игнорируем
  }
}

export function clearCreateSession(): void {
  try {
    sessionStorage.removeItem(KEY);
  } catch {
    // ignore
  }
}
