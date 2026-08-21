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

use super::discovery;

/// Чистое объединение: существующие записи в прежнем порядке,
/// затем недостающие добавления. Пустые отбрасываются, дубли
/// не заносятся. Сравнение НОРМАЛИЗОВАННОЕ (см. normalize_entry):
/// «C:\foo» и «c:\foo\» считаются одной записью на Windows, но не
/// на Unix (там регистр значим).
pub fn merge_dirs(existing: &[String], additions: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for dir in existing.iter().chain(additions) {
        let dir = dir.trim();
        if dir.is_empty() {
            continue;
        }
        if out.iter().any(|e| same_dir(e, dir)) {
            continue;
        }
        out.push(dir.to_string());
    }
    out
}

/// Нормализация записи PATH для сравнения: единый разделитель,
/// без хвостового слеша; на Windows — без учёта регистра.
/// Содержимое записей НЕ меняется (пишем как есть) — нормализуются
/// только копии для сравнения.
fn normalize_entry(entry: &str) -> String {
    let mut s = entry.trim().replace('/', "\\");
    while s.ends_with('\\') && s.len() > 1 {
        s.pop();
    }
    if cfg!(target_os = "windows") {
        s.to_ascii_lowercase()
    } else {
        s
    }
}

/// Та же ли это запись каталога (нормализованное сравнение)?
pub fn same_dir(a: &str, b: &str) -> bool {
    normalize_entry(a) == normalize_entry(b)
}

/// Удаляет из `dirs` ТОЛЬКО записи, совпадающие с `removals`
/// (нормализованное сравнение). Чужие записи гарантированно
/// остаются нетронутыми — основа обратимого отката наших изменений.
pub fn remove_dirs(dirs: &[String], removals: &[String]) -> Vec<String> {
    dirs.iter()
        .filter(|d| !d.trim().is_empty() && !removals.iter().any(|r| same_dir(d, r)))
        .cloned()
        .collect()
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

/// Раскрывает %VAR% и glob (`*`) в записи пути: `PostgreSQL/*/bin`
/// → первый реально существующий каталог (17, 18...). Запись без
/// совпадения возвращается как есть — она просто не попадёт в PATH
/// как существующий каталог.
fn resolve_path_entry(raw: &str) -> String {
    let expanded = expand_env_vars(raw);
    if expanded.contains('*') {
        if let Some(found) = discovery::glob_first(&std::path::PathBuf::from(&expanded)) {
            return found.to_string_lossy().into_owned();
        }
    }
    expanded
}

/// Добавляет каталоги в пользовательский PATH (постоянно).
/// Идемпотентно: уже присутствующие записи не дублируются
/// (нормализованное сравнение). glob-записи (`PostgreSQL/*/bin`)
/// резолвятся в конкретный каталог.
pub async fn add_to_user_path(dirs: &[String]) -> Result<(), String> {
    let platform = platforms::current_platform();
    let existing = platform.read_user_path().await.unwrap_or_default();
    let resolved: Vec<String> = dirs.iter().map(|d| resolve_path_entry(d)).collect();
    let merged = merge_dirs(&existing, &resolved);
    if merged == existing {
        return Ok(()); // менять нечего — не трогаем реестр/rc-файл
    }
    platform.write_user_path(&merged).await
}

/// Удаляет из пользовательского PATH ровно перечисленные записи
/// (нормализованное сравнение) — обратная операция к add_to_user_path.
/// Чужие записи не трогаются. Вызывается только из явных заданий.
///
/// Примитив обратимости PATH (контракт §5.7): пока операции удаления
/// (uninstall/rollback) не вошли в набор заданий, функция держится
/// готовой и покрытой контрактом «убирать только своё».
#[allow(dead_code)]
pub async fn remove_from_user_path(dirs: &[String]) -> Result<(), String> {
    let platform = platforms::current_platform();
    let existing = platform.read_user_path().await.unwrap_or_default();
    let resolved: Vec<String> = dirs.iter().map(|d| resolve_path_entry(d)).collect();
    let remaining = remove_dirs(&existing, &resolved);
    if remaining.len() == existing.len() {
        return Ok(()); // ничего нашего не нашлось — менять нечего
    }
    platform.write_user_path(&remaining).await
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
            vec![
                "C:\\a".to_string(),
                "C:\\b".to_string(),
                "C:\\c".to_string()
            ]
        );
    }

    #[test]
    fn merge_skips_empty_and_whitespace() {
        let existing = vec!["".to_string(), "  ".to_string(), "C:\\x".to_string()];
        assert_eq!(merge_dirs(&existing, &[]), vec!["C:\\x".to_string()]);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn merge_is_case_sensitive() {
        // на Linux /opt/foo и /opt/Foo — разные каталоги, сливать нельзя
        let existing = vec!["/opt/Foo".to_string()];
        assert_eq!(
            merge_dirs(&existing, &["/opt/foo".to_string()]),
            vec!["/opt/Foo".to_string(), "/opt/foo".to_string()]
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn merge_normalizes_windows_paths() {
        // Windows: регистр и хвостовой слеш не делают записи разными.
        let existing = vec!["C:\\Program Files\\nodejs\\".to_string()];
        let merged = merge_dirs(&existing, &["c:/program files/nodejs".to_string()]);
        assert_eq!(merged.len(), 1, "нормализованный дубликат: {merged:?}");
    }

    #[test]
    fn remove_dirs_never_touches_unrelated_entries() {
        let dirs = vec![
            "C:\\Windows\\System32".to_string(),
            "C:\\Program Files\\nodejs".to_string(),
            "D:\\tools".to_string(),
        ];
        let removals = vec!["c:/program files/nodejs/".to_string()];
        let remaining = remove_dirs(&dirs, &removals);
        assert_eq!(
            remaining,
            vec!["C:\\Windows\\System32".to_string(), "D:\\tools".to_string()]
        );
        // Пустые удаления — ничего не меняется.
        assert_eq!(remove_dirs(&dirs, &[]), dirs);
    }

    #[test]
    fn expand_known_and_unknown_vars() {
        // Windows: %USERPROFILE% — настоящая переменная; Unix: %HOME%.
        let (var, expected) = if let Ok(v) = std::env::var("USERPROFILE") {
            ("%USERPROFILE%", v)
        } else {
            ("%HOME%", std::env::var("HOME").expect("нет HOME"))
        };
        assert_eq!(
            expand_env_vars(&format!("{var}\\dev")),
            format!("{expected}\\dev")
        );

        // неизвестная переменная остаётся как есть
        assert_eq!(
            expand_env_vars("%NO_SUCH_VAR_XYZ%\\bin"),
            "%NO_SUCH_VAR_XYZ%\\bin"
        );
    }

    #[test]
    fn expand_leaves_plain_path() {
        assert_eq!(
            expand_env_vars("C:\\Program Files\\nodejs"),
            "C:\\Program Files\\nodejs"
        );
    }
}
