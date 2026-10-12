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
// берёт за основу СВЕЖИЕ записи системы (раскрывая %VAR% сам) и
// дополняет их недостающими записями текущего процесса — иначе
// только что установленный инструмент остался бы за Store-заглушкой
// (WindowsApps\python.exe), которая в PATH процесса стоит раньше.

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

/// Раскрывает переменные окружения в строке.
/// - Windows: %VAR% через текущее окружение процесса
/// - Unix: $VAR, ${VAR} через текущее окружение, ~ через $HOME
/// Неизвестная переменная остаётся как есть (и не крутит цикл).
pub fn expand_env_vars(raw: &str) -> String {
    // Unix tilde expansion
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(rest) = raw.strip_prefix('~') {
            if let Ok(home) = std::env::var("HOME") {
                let expanded = format!("{home}{rest}");
                return expand_env_vars(&expanded);
            }
        }
    }

    // Windows %VAR% expansion
    #[cfg(target_os = "windows")]
    {
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
        return out;
    }

    // Unix $VAR and ${VAR} expansion
    #[cfg(not(target_os = "windows"))]
    {
        expand_unix_vars(raw)
    }
}

/// Expand $VAR and ${VAR} patterns using current environment (Unix only).
#[cfg(not(target_os = "windows"))]
fn expand_unix_vars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' {
            if let Some(&'{') = chars.peek() {
                chars.next(); // consume '{'
                let mut name = String::new();
                for ch in chars.by_ref() {
                    if ch == '}' {
                        break;
                    }
                    name.push(ch);
                }
                if let Ok(val) = std::env::var(&name) {
                    out.push_str(&val);
                } else {
                    out.push_str("${");
                    out.push_str(&name);
                    out.push('}');
                }
            } else {
                let mut name = String::new();
                for ch in chars.by_ref() {
                    if ch.is_ascii_alphanumeric() || ch == '_' {
                        name.push(ch);
                    } else {
                        if !name.is_empty() {
                            if let Ok(val) = std::env::var(&name) {
                                out.push_str(&val);
                            } else {
                                out.push('$');
                                out.push_str(&name);
                            }
                            name.clear();
                        }
                        out.push(ch);
                        break;
                    }
                }
                if !name.is_empty() {
                    if let Ok(val) = std::env::var(&name) {
                        out.push_str(&val);
                    } else {
                        out.push('$');
                        out.push_str(&name);
                    }
                }
            }
        } else {
            out.push(c);
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

/// Относится ли запись каталога к текущей ОС. Каталог tools.json общий
/// для всех ОС, поэтому в path_entries лежат и Windows-, и Unix-записи:
/// при добавлении PATH на Windows записи вида `/usr/lib/dotnet` или
/// `$HOME/.dotnet` пропускаются (а не роняют добавление ВСЕГО набора
/// ошибкой «не абсолютный каталог»), и наоборот.
fn entry_applies_to_platform(entry: &str) -> bool {
    let trimmed = entry.trim();
    #[cfg(target_os = "windows")]
    {
        // Unix-стиль: /path, ~/path, $VAR/path — к Windows не относится.
        !(trimmed.starts_with('/') || trimmed.starts_with('~') || trimmed.starts_with('$'))
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Windows-стиль: %VAR%\path, C:\path, C:/path, \\server\share.
        let bytes = trimmed.as_bytes();
        let drive = bytes.len() >= 2 && bytes[1] == b':' && (bytes[0] as char).is_ascii_alphabetic();
        !(trimmed.starts_with('%') || trimmed.starts_with("\\\\") || drive)
    }
}

/// Добавляет каталоги в пользовательский PATH (постоянно).
/// Идемпотентно: уже присутствующие записи не дублируются
/// (нормализованное сравнение). glob-записи (`PostgreSQL/*/bin`)
/// резолвятся в конкретный каталог.
///
/// Валидация: в PATH уходят ТОЛЬКО абсолютные каталоги (после раскрытия
/// %VAR% и glob). Относительная запись — мусор в реестре/rc-файле:
/// она отклоняется с ошибкой, а не молча пишется.
///
/// Записи, содержащие wildcards (`*`) после раскрытия glob, также
/// отклоняются — Windows не понимает wildcard в PATH. Если glob не
/// совпал (каталог не найден), запись не добавляется, чтобы не
/// засорять PATH мусором вида `C:\Program Files\erl*\bin`.
pub async fn add_to_user_path(dirs: &[String]) -> Result<(), String> {
    let platform = platforms::current_platform();
    let existing = platform.read_user_path().await.unwrap_or_default();
    let mut resolved: Vec<String> = Vec::new();
    for raw in dirs {
        if !entry_applies_to_platform(raw) {
            // Запись другой ОС — не наш PATH, пропускаем без ошибки.
            log::debug!("[toolchain] пропуск записи PATH другой ОС: {raw}");
            continue;
        }
        let entry = resolve_path_entry(raw);
        if entry.trim().is_empty() {
            continue;
        }
        if entry.contains('*') {
            // Wildcard не раскрылся (glob не совпал) — пропускаем,
            // чтобы не засорять PATH строкой вида `C:\erl*\bin`.
            log::warn!("[toolchain] пропуск wildcard-записи PATH: {raw} → {entry} (glob не совпал)");
            continue;
        }
        if !is_absolute_entry(&entry) {
            return Err(format!(
                "Запись PATH «{raw}» не является абсолютным каталогом — PATH не изменён"
            ));
        }
        // На Unix несуществующие каталоги в rc-файл не пишем: PATH
        // засорялся бы «мёртвыми» записями (каталог другой ОС,
        // необязательный путь dotnet и т.п.), а диагностика PATH
        // справедливо ругалась бы на них. На Windows поведение прежнее.
        #[cfg(not(target_os = "windows"))]
        {
            if !std::path::Path::new(&entry).is_dir() {
                log::debug!(
                    "[toolchain] пропуск несуществующего каталога PATH: {raw} → {entry}"
                );
                continue;
            }
        }
        resolved.push(entry);
    }
    let merged = merge_dirs(&existing, &resolved);
    if merged == existing {
        return Ok(()); // менять нечего — не трогаем реестр/rc-файл
    }
    platform.write_user_path(&merged).await
}

/// Абсолютный ли это путь? На Windows — с дисководом или UNC; на Unix —
/// с ведущим '/'. Относительные записи в постоянный PATH не пишутся.
fn is_absolute_entry(entry: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        let bytes = entry.as_bytes();
        (bytes.len() >= 2 && bytes[1] == b':' && (bytes[0] as char).is_ascii_alphabetic())
            || entry.starts_with("\\\\")
    }
    #[cfg(not(target_os = "windows"))]
    {
        entry.starts_with('/')
    }
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
///
/// Порядок записей: СВЕЖИЕ записи (система+пользователь) идут ПЕРВЫМИ,
/// затем — записи процесса, которых в свежем списке нет. Это критично
/// для Windows: установщики (Python с PrependPath=1) добавляют свои
/// каталоги В НАЧАЛО пользовательского PATH, а запущенное приложение
/// держит PATH со времён старта. Дописывание свежих каталогов в КОНЕЦ
/// оставляло Microsoft Store-заглушку (WindowsApps\python.exe) ПЕРВОЙ —
/// `python` продолжал открывать Store/падать с кодом 9009 вместо
/// только что установленного интерпретатора.
pub async fn sync_process_path() -> Result<(), String> {
    let platform = platforms::current_platform();
    let base = process_path_entries();
    let mut fresh = platform.read_system_path().await.unwrap_or_default();
    fresh.extend(platform.read_user_path().await.unwrap_or_default());
    let expanded: Vec<String> = fresh.iter().map(|d| expand_env_vars(d)).collect();
    // Свежие записи впереди: они отражают текущую правду реестра
    // (порядок, установленный инсталлятором), процесс-записи, которых
    // в свежем списке нет, сохраняются в конце.
    let merged = merge_dirs(&expanded, &base);

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

    /// Регрессия sync_process_path: свежие записи реестра (система +
    /// пользователь, после установки Python с PrependPath=1 Python313 стоит
    /// ПЕРВЫМ) обязаны опередить «застаревший» PATH процесса, где раньше
    /// сидит Store-заглушка WindowsApps\python.exe. Дописывание свежих
    /// каталогов в конец оставляло заглушку первой — `python` находил
    /// Store-заглушку (код 9009), а не реальный интерпретатор.
    #[test]
    fn fresh_entries_precede_stale_process_entries() {
        // PATH процесса: заглушка WindowsApps в середине, реального python нет.
        let base = vec![
            "C:\\Windows\\System32".to_string(),
            "C:\\Users\\And\\AppData\\Local\\Microsoft\\WindowsApps".to_string(),
            "C:\\tools\\git\\cmd".to_string(),
        ];
        // Свежий пользовательский PATH после установки Python: Python313
        // ДО WindowsApps (PrependPath=1), затем обычные каталоги.
        let fresh = vec![
            "%LOCALAPPDATA%\\Programs\\Python\\Python313".to_string(),
            "%LOCALAPPDATA%\\Programs\\Python\\Python313\\Scripts".to_string(),
            "%SystemRoot%\\System32".to_string(),
            "%LOCALAPPDATA%\\Microsoft\\WindowsApps".to_string(),
            "C:\\tools\\git\\cmd".to_string(),
        ];
        let expanded: Vec<String> = fresh.iter().map(|d| expand_env_vars(d)).collect();
        let merged = merge_dirs(&expanded, &base);
        let windows_apps_pos = merged
            .iter()
            .position(|d| same_dir(d, "%LOCALAPPDATA%\\Microsoft\\WindowsApps"));
        let python_pos = merged.iter().position(|d| {
            let expanded = expand_env_vars("%LOCALAPPDATA%\\Programs\\Python\\Python313");
            same_dir(d, &expanded)
        });
        assert!(python_pos.is_some(), "Python313 обязан быть в PATH: {merged:?}");
        assert!(
            windows_apps_pos.is_none() || python_pos < windows_apps_pos,
            "Python обязан опережать Store-заглушку WindowsApps: {merged:?}"
        );
        // Процесс-записи, которых нет в свежем списке, сохраняются в конце.
        let system32_expanded = expand_env_vars("%SystemRoot%\\System32");
        assert!(
            merged.iter().any(|d| same_dir(d, &system32_expanded)),
            "свежие System32 раскрыты и на месте: {merged:?}"
        );
        // Застаревшая запись процесса, которой нет в свежем списке
        // (чужая WindowsApps «C:\Users\And\...»), сохраняется в конце.
        let stale_pos = merged
            .iter()
            .position(|d| same_dir(d, "C:\\Users\\And\\AppData\\Local\\Microsoft\\WindowsApps"));
        assert!(
            stale_pos.is_some() && python_pos < stale_pos,
            "застаревшая запись остаётся ПОСЛЕ свежего Python: {merged:?}"
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

    #[cfg(target_os = "windows")]
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

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn remove_dirs_never_touches_unrelated_entries_unix() {
        // Unix: сравнение регистрозависимо, совпадающие записи удаляются.
        let dirs = vec!["/usr/bin".to_string(), "/opt/nodejs".to_string()];
        let removals = vec!["/opt/nodejs/".to_string()];
        let remaining = remove_dirs(&dirs, &removals);
        assert_eq!(remaining, vec!["/usr/bin".to_string()]);
        // Пустые удаления — ничего не меняется.
        assert_eq!(remove_dirs(&dirs, &[]), dirs);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn expand_known_and_unknown_vars() {
        // Windows: %USERPROFILE% — настоящая переменная.
        let expected = std::env::var("USERPROFILE").expect("нет USERPROFILE");
        assert_eq!(
            expand_env_vars("%USERPROFILE%\\dev"),
            format!("{expected}\\dev")
        );

        // неизвестная переменная остаётся как есть
        assert_eq!(
            expand_env_vars("%NO_SUCH_VAR_XYZ%\\bin"),
            "%NO_SUCH_VAR_XYZ%\\bin"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn expand_known_and_unknown_vars_unix() {
        // Unix: %VAR% не раскрывается, а $VAR и ~ — раскрываются.
        assert_eq!(expand_env_vars("%HOME%/dev"), "%HOME%/dev");
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() {
            assert_eq!(expand_env_vars("~/dev"), format!("{home}/dev"));
            assert_eq!(
                expand_env_vars("$HOME/dev"),
                format!("{home}/dev")
            );
        }

        // неизвестная переменная остаётся как есть
        assert_eq!(expand_env_vars("$NO_SUCH_VAR_XYZ/bin"), "$NO_SUCH_VAR_XYZ/bin");
    }

    #[test]
    fn expand_leaves_plain_path() {
        assert_eq!(
            expand_env_vars("C:\\Program Files\\nodejs"),
            "C:\\Program Files\\nodejs"
        );
    }

    #[test]
    fn absolute_entry_detection_matches_platform() {
        #[cfg(target_os = "windows")]
        {
            assert!(is_absolute_entry("C:\\Program Files\\nodejs"));
            assert!(is_absolute_entry("D:/tools"));
            assert!(is_absolute_entry("\\\\server\\share\\bin"));
            assert!(!is_absolute_entry("bin"));
            assert!(!is_absolute_entry(".\\bin"));
            assert!(!is_absolute_entry("Program Files/nodejs"));
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert!(is_absolute_entry("/opt/node/bin"));
            assert!(!is_absolute_entry("opt/node/bin"));
            assert!(!is_absolute_entry("./bin"));
        }
    }

    /// Регрессия безопасности PATH: относительная запись — мусор
    /// в реестре/rc-файле. add_to_user_path обязана отклонять её
    /// ДО записи (а не молча писать).
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn add_to_user_path_rejects_relative_entries() {
        let err = add_to_user_path(&["bin".to_string()]).await.unwrap_err();
        assert!(
            err.contains("абсолютным каталогом"),
            "относительная запись отклоняется: {err}"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn expand_env_vars_unix_dollar_var() {
        if let Ok(home) = std::env::var("HOME") {
            assert_eq!(expand_env_vars("$HOME/bin"), format!("{home}/bin"));
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn expand_env_vars_unix_braced_var() {
        if let Ok(home) = std::env::var("HOME") {
            assert_eq!(expand_env_vars("${HOME}/bin"), format!("{home}/bin"));
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn expand_env_vars_unix_unknown_var_stays() {
        assert_eq!(expand_env_vars("$NOPE_XYZ/bin"), "$NOPE_XYZ/bin");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn expand_env_vars_unix_tilde() {
        if let Ok(home) = std::env::var("HOME") {
            assert_eq!(expand_env_vars("~/bin"), format!("{home}/bin"));
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn expand_env_vars_windows_percent_var() {
        if let Ok(home) = std::env::var("USERPROFILE") {
            assert_eq!(
                expand_env_vars("%USERPROFILE%\\bin"),
                format!("{home}\\bin")
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn expand_env_vars_windows_unknown_var_stays() {
        assert_eq!(expand_env_vars("%NOPE_XYZ%\\bin"), "%NOPE_XYZ%\\bin");
    }

    #[test]
    fn merge_dirs_dedupes_across_platforms() {
        let existing = vec!["/a".to_string(), "/b".to_string()];
        let additions = vec!["/b".to_string(), "/c".to_string()];
        let merged = merge_dirs(&existing, &additions);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0], "/a");
        assert_eq!(merged[1], "/b");
        assert_eq!(merged[2], "/c");
    }

    /// Записи PATH другой ОС не должны ронять добавление всего набора:
    /// каталог tools.json общий, и в path_entries лежат и Windows-, и
    /// Unix-записи одновременно.
    #[test]
    fn platform_entry_filter_matches_current_os() {
        #[cfg(target_os = "windows")]
        {
            assert!(entry_applies_to_platform("%APPDATA%\\npm"));
            assert!(entry_applies_to_platform("C:\\Program Files\\dotnet"));
            assert!(entry_applies_to_platform("bin")); // относительная — отклонит is_absolute_entry
            assert!(!entry_applies_to_platform("/usr/lib/dotnet"));
            assert!(!entry_applies_to_platform("$HOME/.dotnet"));
            assert!(!entry_applies_to_platform("~/.cargo/bin"));
        }
        #[cfg(not(target_os = "windows"))]
        {
            assert!(entry_applies_to_platform("/usr/lib/dotnet"));
            assert!(entry_applies_to_platform("$HOME/.dotnet"));
            assert!(entry_applies_to_platform("~/.cargo/bin"));
            assert!(entry_applies_to_platform("bin")); // относительная — отклонит is_absolute_entry
            assert!(!entry_applies_to_platform("%APPDATA%\\npm"));
            assert!(!entry_applies_to_platform("C:\\Program Files\\dotnet"));
            assert!(!entry_applies_to_platform("\\\\server\\share\\bin"));
        }
    }

    #[test]
    fn remove_dirs_handles_nonexistent_entries() {
        let dirs = vec!["/a".to_string(), "/b".to_string()];
        let remaining = remove_dirs(&dirs, &["/z".to_string()]);
        assert_eq!(remaining, dirs);
    }
}
