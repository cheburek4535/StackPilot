// ============================================================
// /environment — легаси-алиас Control Center (контракт §4.7).
// Канонический маршрут — /toolchain; алиас сохраняет работающие
// ссылки и закладки. Навигация уже покрывает оба пути.
import { redirect } from "@sveltejs/kit";

export function load(): never {
  redirect(308, "/toolchain");
}
