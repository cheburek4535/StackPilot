// ============================================================
// PATH и переменные окружения (path_service.rs) — этап 5
// ============================================================
// Единая точка работы с PATH:
//
//   - add_to_user_path  — добавить каталоги в «постоянный» PATH
//                         пользователя (Windows: HKCU\Environment
//                         через [Environment] — Explorer узнает
//                         сразу; Unix: rc-файл с маркером StackPilot);
//   - sync_process_path — обновить PATH ТЕКУЩЕГО процесса свежими
//                         значениями из системы: без этого только
//                         что поставленный инструмент не найдётся
//                         в verify, т.к. окружение приложения
//                         осталось с момента запуска;
//   - merge_dirs        — чистое объединение записей без дублей;
//   - expand_env_vars   — раскрытие %VAR% в записях из реестра
//                         (%ProgramFiles% и т.п.).
//
// Windows-деталь: реестр хранит PATH с %VAR%-ссылками, а процесс
// при старте получает уже раскрытую копию. Поэтому sync_process_path
// берёт за базу раскрытый PATH процесса и ДОБАВЛЯЕТ к нему свежие
// записи (раскрывая их сам), а не заменяет целиком — иначе
// %VAR%-записи сломали бы запуск дочерних процессов.

use crate::modules::toolchain::platforms;

/// Чистое объединение: существующие записи в прежнем порядке,
/// затем недостающие добавления. Пустые отбрасываются, дубли
/// (точное совпадение строк) не заносятся.
pub fn merge_dirs(existing: &[String], additions: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for dir in existing.iter().chain(additions) {
        let dir = dir.trim();
        if dir.is_empty() || out.iter().any(|e| e == dir) {
            continue;
        }
        out.push(dir.to_string());
    }
    out
}

/// Раскрывает %VAR% в строке через переменные текущего процесса.
/// Неизвестная переменная остаётся как есть (и не крутит цикл).
pub fn expand_env_vars(raw: &str) -> String {
    let mut out = raw.to_string();
    let mut guard = 0;
    while let Some(start) = out.find('%') {
        if guard > 10 {
            break; // защита от причудливых данных
        }
        guard += 1;
        let Some(end_rel) = out[start + 1..].find('%') else {
            break;
        };
        let end = start + 1 + end_rel;
        let name = &out[start + 1..end];
        if let Ok(value) = std::env::var(name) {
            out.replace_range(start..=end, &value);
        } else {
            break;
        }
    }
    out
}

/// Записи PATH текущего процесса (уже раскрытые ОС).
pub fn process_path_entries() -> Vec<String> {
    let sep = platforms::current_platform().path_separator();
    std::env::var("PATH")
        .map(|p| {
            p.split(&sep)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Добавляет каталоги в пользовательский PATH (постоянно).
/// Идемпотентно: уже присутствующие записи не дублируются.
pub async fn add_to_user_path(dirs: &[String]) -> Result<(), String> {
    let platform = platforms::current_platform();
    let existing = platform.read_user_path().await.unwrap_or_default();
    let merged = merge_dirs(&existing, dirs);
    if merged == existing {
        return Ok(()); // менять нечего — не трогаем реестр/rc-файл
    }
    platform.write_user_path(&merged).await
}

/// Обновляет PATH текущего процесса свежими значениями системы.
///
/// Вызывается после каждой успешной установки: winget/инсталляторы
/// пишут PATH в реестр, а процесс об этом не узнаёт — без вызова
/// следующая проверка версии («инструмент не найден») была бы
/// ложной. Ошибки чтения системы не фатальны: используем то,
/// что удалось получить.
pub async fn sync_process_path() -> Result<(), String> {
    let platform = platforms::current_platform();
    let base = process_path_entries();
    let mut fresh = platform.read_system_path().await.unwrap_or_default();
    fresh.extend(platform.read_user_path().await.unwrap_or_default());
    let expanded: Vec<String> = fresh.iter().map(|d| expand_env_vars(d)).collect();
    let merged = merge_dirs(&base, &expanded);

    std::env::set_var("PATH", merged.join(&platform.path_separator()));
    Ok(())
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_order_and_dedupes() {
        let existing = vec!["C:\\a".to_string(), "C:\\b".to_string()];
        let additions = vec!["C:\\b".to_string(), "C:\\c".to_string()];
        assert_eq!(
            merge_dirs(&existing, &additions),
            vec!["C:\\a".to_string(), "C:\\b".to_string(), "C:\\c".to_string()]
        );
    }

    #[test]
    fn merge_skips_empty_and_whitespace() {
        let existing = vec!["".to_string(), "  ".to_string(), "C:\\x".to_string()];
        assert_eq!(
            merge_dirs(&existing, &[]),
            vec!["C:\\x".to_string()]
        );
    }

    #[test]
    fn merge_is_case_sensitive() {
        // на Linux /opt/foo и /opt/Foo — разные каталоги, сливать нельзя
        let existing = vec!["/opt/Foo".to_string()];
        assert_eq!(
            merge_dirs(&existing, &["/opt/foo".to_string()]),
            vec!["/opt/Foo".to_string(), "/opt/foo".to_string()]
        );
    }

    #[test]
    fn expand_known_and_unknown_vars() {
        // Windows: %USERPROFILE% — настоящая переменная; Unix: %HOME%.
        let (var, expected) = if let Ok(v) = std::env::var("USERPROFILE") {
            ("%USERPROFILE%", v)
        } else {
            ("%HOME%", std::env::var("HOME").expect("нет HOME"))
        };
        assert_eq!(expand_env_vars(&format!("{var}\\dev")), format!("{expected}\\dev"));

        // неизвестная переменная остаётся как есть
        assert_eq!(
            expand_env_vars("%NO_SUCH_VAR_XYZ%\\bin"),
            "%NO_SUCH_VAR_XYZ%\\bin"
        );
    }

    #[test]
    fn expand_leaves_plain_path() {
        assert_eq!(expand_env_vars("C:\\Program Files\\nodejs"), "C:\\Program Files\\nodejs");
    }
}
