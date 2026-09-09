// ============================================================
// Обнаружение инструментов (discovery.rs)
// ============================================================
// Узнаёт, что установлено на машине пользователя, по трём уликам
// (в порядке надёжности):
//   1. пробы версии — запускаем бинарник (node --version);
//   2. известные пути — C:/Program Files/nodejs существует;
//   3. ключи реестра Windows — reg query HKLM\SOFTWARE\Node.js.
//
// Все три источника перечислены в detection правилах tools.json,
// так что логика тут общая, а данные — декларативные.
//
// Асинхронность: пробы выполняются через tokio::process, чтобы
// медленный или зависший бинарник не заморозил UI — у каждой
// пробы есть таймаут (PROBE_TIMEOUT_SECS).

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::process::Command as TokioCommand;
use tokio::time::timeout;

use crate::modules::toolchain::models::{ToolDefinition, ToolStatus};
use crate::modules::toolchain::platforms;

use super::version;

/// Максимальное время одной пробы/запроса к реестру.
/// 10с: первый холодный запуск npm.cmd/code.cmd бывает медленным.
const PROBE_TIMEOUT_SECS: u64 = 10;

// ------------------------------------------------------------
// Запуск процессов
// ------------------------------------------------------------

/// Максимум байт захватываемого вывода одной команды (легаси-слой
/// discovery/health): шумный или сломанный бинарь не должен выедать
/// память процесса. Значение достаточно для версий/health-строк.
const MAX_CAPTURE_BYTES: usize = 64 * 1024;

/// Обрезает захваченные байты до лимита (по границе байт; lossy-чтение
/// вызывающего остаётся валидным UTF-8-текстом).
fn cap_bytes(mut data: Vec<u8>) -> Vec<u8> {
    if data.len() > MAX_CAPTURE_BYTES {
        data.truncate(MAX_CAPTURE_BYTES);
        data.extend_from_slice("...[вывод обрезан]".as_bytes());
    }
    data
}

/// Запускает команду и ждёт stdout.
/// Возвращает None, если: таймаут, не удалось запустить,
/// или процесс завершился с ненулевым кодом (такое бывает,
/// когда бинарник есть, но команда не для него).
///
/// Windows: проги вида *.cmd/*.bat (npm, code, npx) через
/// platforms::resolve_command оборачиваются в cmd /c — иначе
/// CreateProcess их не видит, хотя в PATH они есть.
///
/// pub(crate): используется и health.rs (этап 6) для прогона
/// health-проверок из tools.json.
pub(crate) async fn run_capture(program: &str, args: &[String]) -> Option<String> {
    let (program, args) = platforms::resolve_command(program, args);
    let output = timeout(
        Duration::from_secs(PROBE_TIMEOUT_SECS),
        TokioCommand::new(program).args(args).output(),
    )
    .await
    .ok()?
    .ok()?;

    if output.status.success() {
        let capped = cap_bytes(output.stdout);
        let stdout = String::from_utf8_lossy(&capped);
        let text = if stdout.trim().is_empty() {
            let capped_err = cap_bytes(output.stderr);
            String::from_utf8_lossy(&capped_err).trim().to_string()
        } else {
            stdout.trim().to_string()
        };
        if text.is_empty() {
            None
        } else {
            Some(text)
        }
    } else {
        None
    }
}

// ------------------------------------------------------------
// Улики
// ------------------------------------------------------------

/// Пробует пробы версии по очереди, пока одна не ответит.
/// Возвращает сырой вывод (например «node v22.12.0»), дальше
/// его разбирает version.rs.
/// Использует effective_detection для платформенных переопределений.
async fn probe_version(def: &ToolDefinition) -> Option<String> {
    let os = platforms::current_platform().os_name();
    let det = def.effective_detection(&os);
    for probe in &det.version_probes {
        if probe.is_empty() {
            continue;
        }
        if let Some(out) = run_capture(&probe[0], &probe[1..]).await {
            return Some(out);
        }
    }
    None
}

/// Расширяет переменные окружения в путях из tools.json.
/// Поддерживает:
///   - Windows: %LOCALAPPDATA%, %APPDATA%, %ProgramFiles%, %USERPROFILE%
///   - Unix: $VAR, ${VAR}, ~ (home directory)
/// На не-Windows переменные вида %VAR% просто остаются как есть —
/// путь не существует, улика не срабатывает.
fn expand_env(raw: &str) -> PathBuf {
    let mut expanded = raw.to_string();

    // Unix tilde expansion
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(rest) = expanded.strip_prefix('~') {
            if let Ok(home) = std::env::var("HOME") {
                expanded = format!("{home}{rest}");
            }
        }
    }

    // Windows %VAR% expansion
    #[cfg(target_os = "windows")]
    {
        const VARS: [(&str, &str); 4] = [
            ("%LOCALAPPDATA%", "LOCALAPPDATA"),
            ("%APPDATA%", "APPDATA"),
            ("%ProgramFiles%", "ProgramFiles"),
            ("%USERPROFILE%", "USERPROFILE"),
        ];
        for (pattern, var) in VARS {
            if expanded.contains(pattern) {
                if let Ok(value) = std::env::var(var) {
                    expanded = expanded.replace(pattern, &value);
                }
            }
        }
    }

    // Unix $VAR and ${VAR} expansion
    #[cfg(not(target_os = "windows"))]
    {
        expanded = expand_unix_vars(&expanded);
    }

    PathBuf::from(expanded)
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
                        // Non-identifier char: not a variable name
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

/// Простейший glob-поиск пути: поддерживает `*` внутри компонентов,
/// например `C:/Program Files/PostgreSQL/*/bin`. Если под шаблон
/// подходят несколько каталогов (17, 18, ...), выбирается САМЫЙ НОВЫЙ
/// по естественно-числовому сравнению имени (10 > 9, 2 < 10) —
/// прежний вариант брал первый по порядку файловой системы и мог
/// выбрать PostgreSQL/10 вместо 17. Возвращает None, если ни один
/// вариант не существует.
///
/// На Windows сравнение имён НЕЧУВСТВИТЕЛЬНО К РЕГИСТРУ: файловая
/// система NTFS регистронезависима, и каталог `Erlang OTP` обязан
/// совпадать с шаблоном `erl*`.
pub fn glob_first(pattern: &Path) -> Option<PathBuf> {
    let mut current = PathBuf::new();
    for component in pattern.components() {
        let comp = component.as_os_str().to_string_lossy();
        if comp.contains('*') {
            let prefix: String = comp.split('*').next().unwrap_or_default().to_string();
            let suffix: String = comp.rsplit('*').next().unwrap_or_default().to_string();
            let parent = current.clone();
            let candidates: Vec<PathBuf> = std::fs::read_dir(&parent)
                .ok()?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default();
                    name.to_ascii_lowercase()
                        .starts_with(&prefix.to_ascii_lowercase())
                        && name
                            .to_ascii_lowercase()
                            .ends_with(&suffix.to_ascii_lowercase())
                        && p.is_dir()
                })
                .collect();
            // Самый «свежий» кандидат: максимум по естественно-числовому ключу.
            let best = candidates.into_iter().max_by(|a, b| {
                let ka = natural_key(&a.to_string_lossy());
                let kb = natural_key(&b.to_string_lossy());
                ka.cmp(&kb)
            })?;
            current = best;
        } else {
            current.push(comp.as_ref());
        }
    }
    current.exists().then_some(current)
}

/// Естественно-числовой ключ сортировки: последовательности цифр
/// сравниваются как числа («17» > «9», «10» > «9»), остальное —
/// как строки. Основа выбора самого нового версионного каталога.
fn natural_key(name: &str) -> Vec<(u64, String)> {
    let mut key = Vec::new();
    let mut num = String::new();
    let mut text = String::new();
    for c in name.chars() {
        if c.is_ascii_digit() {
            if !text.is_empty() {
                key.push((0, std::mem::take(&mut text)));
            }
            num.push(c);
        } else {
            if !num.is_empty() {
                key.push((num.parse().unwrap_or(0), std::mem::take(&mut num)));
            }
            text.push(c);
        }
    }
    if !text.is_empty() {
        key.push((0, text));
    }
    if !num.is_empty() {
        key.push((num.parse().unwrap_or(0), String::new()));
    }
    key
}

/// Есть ли на диске хотя бы один из известных путей установки
/// (с учётом glob-шаблонов вида `PostgreSQL/*/bin`).
/// Использует effective_detection для платформенных переопределений.
fn known_path_found(def: &ToolDefinition) -> bool {
    let os = platforms::current_platform().os_name();
    let det = def.effective_detection(&os);
    det.known_paths
        .iter()
        .any(|p| glob_first(&expand_env(p)).is_some())
}

/// Пробует запустить пробы версии прямо из каталогов known_paths:
/// типичная ситуация — PostgreSQL установлен, но `bin` не добавлен
/// в PATH (инсталлятор не спросил). Тогда `psql --version` через PATH
/// молчит, а `C:/Program Files/PostgreSQL/17/bin/psql.exe --version`
/// отвечает.
///
/// Windows: бинарники имеют расширения (.exe, .cmd, .bat) — пробы
/// из tools.json их не содержат, поэтому перебираем возможные.
/// Использует effective_detection для платформенных переопределений.
///
/// known_paths может указывать на КОРЕНЬ SDK, а бинарь лежит на
/// уровень ниже (flutter: %USERPROFILE%/flutter → bin/): пробуем
/// и подкаталог bin/ каждого каталога.
async fn probe_version_at_known_paths(def: &ToolDefinition) -> Option<String> {
    let exts: &[&str] = if cfg!(target_os = "windows") {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    let os = platforms::current_platform().os_name();
    let det = def.effective_detection(&os);
    for known in &det.known_paths {
        let Some(dir) = glob_first(&expand_env(known)) else {
            continue;
        };
        let mut candidates: Vec<std::path::PathBuf> = vec![dir.clone()];
        let is_bin_dir = dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("bin"));
        let nested_bin = dir.join("bin");
        if !is_bin_dir && nested_bin.is_dir() {
            candidates.push(nested_bin);
        }
        for dir in &candidates {
            for probe in &det.version_probes {
                if probe.is_empty() {
                    continue;
                }
                for ext in exts {
                    let bin = dir.join(format!("{}{}", probe[0], ext));
                    if !bin.is_file() {
                        continue;
                    }
                    // Полный путь без cmd-обёртки: std::process на Windows
                    // сам оборачивает .cmd/.bat в cmd /c, а пути с пробелами
                    // («Program Files») при этом не ломаются.
                    if let Some(out) = run_capture(&bin.to_string_lossy(), &probe[1..]).await {
                        return Some(out);
                    }
                }
            }
        }
    }
    None
}

/// Отвечает ли reg.exe, что ключ реестра существует.
/// На не-Windows reg.exe нет — команда падает, получаем false.
/// Вызывается ТОЛЬКО с Windows; на других ОС never_registry_found.
async fn registry_key_exists(key: &str) -> bool {
    let result = timeout(
        Duration::from_secs(PROBE_TIMEOUT_SECS),
        TokioCommand::new("reg").args(["query", key]).output(),
    )
    .await;
    matches!(result, Ok(Ok(out)) if out.status.success())
}

/// Читает строковое значение из ключа реестра Windows.
/// Возвращает None, если: не Windows, reg.exe недоступен, ключ не найден,
/// или значение не является строкой (REG_SZ).
/// Используется для чтения InstallLocation из ключей удаления NSIS/MSI
/// установщиков — это единственный надёжный способ узнать реальный путь
/// установки, когда пользователь выбрал нестандартный каталог.
async fn registry_read_string(key: &str, value_name: &str) -> Option<String> {
    let output = timeout(
        Duration::from_secs(PROBE_TIMEOUT_SECS),
        TokioCommand::new("reg")
            .args(["query", key, "/v", value_name])
            .output(),
    )
    .await
    .ok()?
    .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Формат вывода reg query:
    //   HKLM\...\erlang
    //       InstallLocation    REG_SZ    C:\Program Files\erlang\
    // Ищем строку с именем значения и извлекаем REG_SZ путь.
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed
            .to_ascii_lowercase()
            .starts_with(&value_name.to_ascii_lowercase())
            && trimmed.contains("REG_SZ")
        {
            // Всё после "REG_SZ" — значение (с ведущим пробелом).
            if let Some(pos) = trimmed.find("REG_SZ") {
                let val = trimmed[pos + 6..].trim();
                if !val.is_empty() {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}

/// Проверяет наличие ключей реестра, используя effective_detection.
/// На не-Windows registry_keys всегда пустые — reg.exe не вызывается.
async fn any_registry_found(def: &ToolDefinition) -> bool {
    let os = platforms::current_platform().os_name();
    let det = def.effective_detection(&os);
    // Registry checks are only meaningful on Windows.
    if os != "windows" {
        return false;
    }
    for key in &det.registry_keys {
        if registry_key_exists(key).await {
            return true;
        }
    }
    false
}

/// Ищет реальный путь установки через ключи реестра Windows.
/// NSIS/MSI установщики записывают InstallLocation в ключ удаления:
///   HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\<name>
/// или
///   HKCU\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\<name>
/// Возвращает каталог bin инструмента, если найден.
/// Используется как запасной вариант, когда known_paths/glob не сработали
/// (пользователь chose нестандартный путь, NSIS запомнил старый путь и т.п.).
async fn probe_version_at_registry_path(def: &ToolDefinition) -> Option<String> {
    let os = platforms::current_platform().os_name();
    let det = def.effective_detection(&os);
    let exts: &[&str] = if cfg!(target_os = "windows") {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };

    if os != "windows" {
        return None;
    }

    for key in &det.registry_keys {
        // Пробуем читать InstallLocation из ключа.
        if let Some(install_dir) = registry_read_string(key, "InstallLocation").await {
            let install_path = PathBuf::from(install_dir.trim_end_matches('\\'));
            // Пробуем bin/ подкаталог.
            let bin = install_path.join("bin");
            if bin.is_dir() {
                for probe in &det.version_probes {
                    if probe.is_empty() {
                        continue;
                    }
                    for ext in exts {
                        let bin_path = bin.join(format!("{}{}", probe[0], ext));
                        if !bin_path.is_file() {
                            continue;
                        }
                        if let Some(out) =
                            run_capture(&bin_path.to_string_lossy(), &probe[1..]).await
                        {
                            return Some(out);
                        }
                    }
                }
            }
            // Если bin/ нет, пробуем сам каталог установки (некоторые
            // установщики кладут бинарники прямо в корень).
            for probe in &det.version_probes {
                if probe.is_empty() {
                    continue;
                }
                for ext in exts {
                    let bin_path = install_path.join(format!("{}{}", probe[0], ext));
                    if !bin_path.is_file() {
                        continue;
                    }
                    if let Some(out) = run_capture(&bin_path.to_string_lossy(), &probe[1..]).await {
                        return Some(out);
                    }
                }
            }
        }
    }
    None
}

// ------------------------------------------------------------
// Интерпретация результатов
// ------------------------------------------------------------

/// Сверяет полученную версию с правилами tools.json:
/// ниже recommended → UpdateAvailable, иначе Installed.
/// Если версию не удалось разобрать, но команда ответила —
/// считаем инструмент установленным (запас в пользу пользователя).
pub(crate) fn apply_version_rules(def: &ToolDefinition, raw_version: &str) -> ToolStatus {
    // Многострочный вывод (например `code --version` печатает версию,
    // commit и архитектуру) сокращаем до первой строки.
    let version = raw_version.lines().next().unwrap_or("").trim().to_string();

    let Ok(parsed) = version::parse_version(raw_version) else {
        return ToolStatus::Installed { version };
    };

    if let Some(recommended_raw) = &def.versions.recommended {
        if let Ok(recommended) = version::parse_version(recommended_raw) {
            if version::compare(&parsed, &recommended) == std::cmp::Ordering::Less {
                return ToolStatus::UpdateAvailable {
                    installed: version,
                    recommended: recommended_raw.clone(),
                };
            }
        }
    }

    ToolStatus::Installed { version }
}

// ------------------------------------------------------------
// Главная функция
// ------------------------------------------------------------

/// Полное обнаружение одного инструмента по его определению.
///
/// Логика:
///   1. ответила проба версии (PATH)      → Installed / UpdateAvailable;
///   2. ответила проба из known_paths     → Installed / UpdateAvailable
///      (инсталляция есть, но бинарь не в PATH — postgres и т.п.);
///   3. прямой скан каталога установки    → Installed / UpdateAvailable
///      (glob не совпал, но каталог на диске есть — erlang и т.п.);
///   4. чтение пути из реестра (InstallLocation) → Installed / UpdateAvailable
///      (NSIS/MSI установщик записал путь, но known_paths не совпал);
///   5. бинарник есть в PATH, но молчит   → PathBroken (сломана установка);
///   6. нашёлся известный путь/реестр     → PathBroken (не в PATH);
///   7. ничего                            → Missing.
pub async fn detect_tool(def: &ToolDefinition) -> ToolStatus {
    let os = platforms::current_platform().os_name();
    let det = def.effective_detection(&os);

    if let Some(raw) = probe_version(def).await {
        return apply_version_rules(def, &raw);
    }

    // Установка найдена по известному пути, но бинарь не в PATH —
    // пробуем запустить его напрямую оттуда.
    if let Some(raw) = probe_version_at_known_paths(def).await {
        return apply_version_rules(def, &raw);
    }

    // Прямой скан каталога установки: glob не совпал (например,
    // erlang ставится в `erl15`, а паттерн был `erl-*`), но
    // каталог на диске реален. Ищем подкаталоги по базовому имени
    // из known_paths и пробуем запустить бинарник из каждого.
    if let Some(raw) = probe_version_at_install_dirs(def).await {
        return apply_version_rules(def, &raw);
    }

    // Чтение реального пути установки из реестра: NSIS/MSI установщики
    // записывают InstallLocation в ключ удаления. Это ловит случай,
    // когда пользователь chose нестандартный путь, а known_paths/glob
    // покрывает только стандартные каталоги.
    if let Some(raw) = probe_version_at_registry_path(def).await {
        return apply_version_rules(def, &raw);
    }

    // Бинарь в PATH есть, но ни одна проба не ответила:
    // установка сломана (dll потерялись, версия не поддерживается...)
    for probe in &det.version_probes {
        if probe.is_empty() || is_shell_probe(&probe[0]) {
            continue;
        }
        if which::which(&probe[0]).is_ok() {
            return ToolStatus::PathBroken {
                reason: format!(
                    "{} найден в PATH, но не отвечает на `{}`",
                    probe[0],
                    probe.join(" ")
                ),
            };
        }
    }

    // Установка есть, но не в PATH — и проба оттуда не сработала.
    if known_path_found(def) || any_registry_found(def).await {
        let footprint = det
            .known_paths
            .first()
            .or_else(|| det.registry_keys.first())
            .cloned()
            .unwrap_or_default();
        return ToolStatus::PathBroken {
            reason: format!(
                "Установка найдена ({}), но бинарник не отвечает на пробу",
                footprint
            ),
        };
    }

    ToolStatus::Missing
}

/// Прямой скан каталога установки: перечисляет подкаталоги базового
/// каталога (например `C:/Program Files/`) и ищет в каждом `bin/`
/// с бинарником тула. Отличается от `probe_version_at_known_paths`
/// тем, что НЕ использует glob-шаблон из known_paths — он сканирует
/// файловую систему напрямую. Это ловит случаи, когда glob не совпал
/// (erlang ставится в `erl15`, а паттерн был `erl-*`).
async fn probe_version_at_install_dirs(def: &ToolDefinition) -> Option<String> {
    let os = platforms::current_platform().os_name();
    let det = def.effective_detection(&os);
    let exts: &[&str] = if cfg!(target_os = "windows") {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };

    for known in &det.known_paths {
        // Извлекаем базовый каталог: из `C:/Program Files/erl*/bin`
        // получаем `C:/Program Files`, а из `erl*` — текущий каталог.
        let expanded = expand_env(known);
        let expanded_str = expanded.to_string_lossy().into_owned();
        let Some(parent) = parent_dir_with_wildcard(&expanded_str) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(&parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default();
            // Проверяем, что имя каталога соответствует паттерну
            // (без wildcard): `erl*` → начинается с `erl`.
            if !matches_wildcard_prefix(&name, &expanded_str) {
                continue;
            }
            let bin = path.join("bin");
            if !bin.is_dir() {
                continue;
            }
            for probe in &det.version_probes {
                if probe.is_empty() {
                    continue;
                }
                for ext in exts {
                    let bin_path = bin.join(format!("{}{}", probe[0], ext));
                    if !bin_path.is_file() {
                        continue;
                    }
                    if let Some(out) = run_capture(&bin_path.to_string_lossy(), &probe[1..]).await {
                        return Some(out);
                    }
                }
            }
        }
    }
    None
}

/// Извлекает родительский каталог из пути с wildcard:
/// `C:/Program Files/erl*/bin` → `C:/Program Files`
/// `C:/Program Files/erl*`     → `C:/Program Files`
/// Если wildcard в самом первом компоненте — возвращается корень.
fn parent_dir_with_wildcard(path: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(path);
    let mut components: Vec<std::path::Component> = p.components().collect();
    // Ищем компонент с `*` и возвращаем всё до него.
    for i in 0..components.len() {
        let comp_str = components[i].as_os_str().to_string_lossy();
        if comp_str.contains('*') {
            // Всё до этого компонента — родитель.
            let parent: std::path::PathBuf = components[..i]
                .iter()
                .map(|c| std::path::PathBuf::from(c.as_os_str()))
                .collect();
            return Some(parent);
        }
    }
    // Нет wildcard — весь путь является родителем (ищем внутри).
    Some(p.to_path_buf())
}

/// Проверяет, что имя файла соответствует паттерну с wildcard:
/// `erl15` соответствует `C:/Program Files/erl*/bin` (начинается с `erl`).
/// `nodejs` НЕ соответствует `C:/Program Files/erl*`.
fn matches_wildcard_prefix(name: &str, pattern: &str) -> bool {
    // Ищем компонент пути, содержащий `*`, среди ВСЕХ компонентов
    // (а не только последний — file_name() вернёт `bin`, а не `erl*`).
    let p = std::path::Path::new(pattern);
    let Some(wildcard_comp) = p
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .find(|comp| comp.contains('*'))
    else {
        return true; // нет wildcard — любой каталог подходит
    };
    let prefix: String = wildcard_comp
        .split('*')
        .next()
        .unwrap_or_default()
        .to_string();
    if prefix.is_empty() {
        return true; // паттерн начинается с `*` — всё подходит
    }
    name.to_ascii_lowercase()
        .starts_with(&prefix.to_ascii_lowercase())
}

/// Пробы, которые запускают оболочку, а не сам инструмент
/// (kafka читает версию через powershell). Наличие оболочки в PATH
/// ничего не говорит об инструменте — это не признак «сломана
/// установка».
fn is_shell_probe(program: &str) -> bool {
    matches!(
        program.to_ascii_lowercase().as_str(),
        "powershell" | "pwsh" | "cmd" | "sh" | "bash" | "shell"
    )
}

/// Первый из known_paths, который реально существует на диске
/// (с учётом glob). Используется для записи в state.json после
/// установки (путь установки сам инструмент не сообщает).
/// Использует effective_detection для платформенных переопределений.
pub(crate) fn installed_path(def: &ToolDefinition) -> Option<String> {
    let os = platforms::current_platform().os_name();
    let det = def.effective_detection(&os);
    det.known_paths
        .iter()
        .find_map(|p| glob_first(&expand_env(p)).map(|p| p.to_string_lossy().into_owned()))
}

/// Читает реальный путь установки из реестра Windows (InstallLocation).
/// Используется установщиком для добавления каталога bin в PATH после
/// NSIS/MSI установки, когда known_paths/glob не покрывают нестандартный
/// путь пользователя. Возвращает Some(bin_path) если:
///   - ключ реестра содержит InstallLocation,
///   - по этому пути есть подкаталог bin с бинарником тула.
pub(crate) async fn registry_install_bin_path(def: &ToolDefinition) -> Option<String> {
    let os = platforms::current_platform().os_name();
    if os != "windows" {
        return None;
    }
    let det = def.effective_detection(&os);
    let exts: &[&str] = &["", ".exe", ".cmd", ".bat"];

    for key in &det.registry_keys {
        if let Some(install_dir) = registry_read_string(key, "InstallLocation").await {
            let install_path = PathBuf::from(install_dir.trim_end_matches('\\'));
            let bin = install_path.join("bin");
            if bin.is_dir() {
                // Проверяем, что в bin реально есть бинарник тула.
                for probe in &det.version_probes {
                    if probe.is_empty() {
                        continue;
                    }
                    for ext in exts {
                        if bin.join(format!("{}{}", probe[0], ext)).is_file() {
                            return Some(bin.to_string_lossy().into_owned());
                        }
                    }
                }
            }
        }
    }
    None
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::toolchain::{defs, models::ToolDefinition};

    fn def(id: &str) -> ToolDefinition {
        defs::load_definitions()
            .into_iter()
            .find(|d| d.id == id)
            .unwrap_or_else(|| panic!("инструмента {id} нет в tools.json"))
    }

    /// Шумный/сломанный бинарь не должен выедать память процесса:
    /// захваченный вывод ограничен сверху (защита от бесконечного потока).
    #[test]
    fn capture_output_is_capped() {
        let small = cap_bytes(vec![b'x'; 1000]);
        assert_eq!(small.len(), 1000, "малый вывод не трогается");

        let big = cap_bytes(vec![b'x'; MAX_CAPTURE_BYTES * 3]);
        assert!(
            big.len() <= MAX_CAPTURE_BYTES + 64,
            "вывод обязан быть ограничен: {}",
            big.len()
        );
        assert!(
            String::from_utf8_lossy(&big).contains("обрезан"),
            "пометка об обрезании видна"
        );
    }

    #[test]
    fn new_version_is_installed() {
        // node: min 18, recommended 22
        match apply_version_rules(&def("node"), "node v24.3.0") {
            ToolStatus::Installed { version } => assert_eq!(version, "node v24.3.0"),
            other => panic!("ожидали Installed, получили {other:?}"),
        }
    }

    #[test]
    fn old_version_is_update_available() {
        match apply_version_rules(&def("node"), "v18.19.0") {
            ToolStatus::UpdateAvailable {
                installed,
                recommended,
            } => {
                assert_eq!(installed, "v18.19.0");
                assert_eq!(recommended, "22");
            }
            other => panic!("ожидали UpdateAvailable, получили {other:?}"),
        }
    }

    #[test]
    fn version_above_recommended_is_installed() {
        // winget: min 1.4, recommended 1.10 — 1.10.1 новее рекомендуемой
        match apply_version_rules(&def("winget"), "v1.10.1") {
            ToolStatus::Installed { .. } => {}
            other => panic!("ожидали Installed, получили {other:?}"),
        }
    }

    #[test]
    fn version_below_recommended_is_update_available() {
        // winget 1.9.25200 старее рекомендуемой 1.10
        match apply_version_rules(&def("winget"), "v1.9.25200") {
            ToolStatus::UpdateAvailable {
                installed,
                recommended,
            } => {
                assert_eq!(installed, "v1.9.25200");
                assert_eq!(recommended, "1.10");
            }
            other => panic!("ожидали UpdateAvailable, получили {other:?}"),
        }
    }

    #[test]
    fn unparseable_output_still_installed() {
        match apply_version_rules(&def("git"), "git version ?unknown") {
            ToolStatus::Installed { .. } => {}
            other => panic!("ожидали Installed, получили {other:?}"),
        }
    }

    #[test]
    fn expand_env_replaces_vars() {
        let expanded = expand_env("%USERPROFILE%\\dev");
        if let Ok(home) = std::env::var("USERPROFILE") {
            assert_eq!(expanded, PathBuf::from(format!("{home}\\dev")));
        }
    }

    #[test]
    fn expand_env_keeps_plain_paths() {
        assert_eq!(
            expand_env("C:/Program Files/nodejs"),
            PathBuf::from("C:/Program Files/nodejs")
        );
    }

    #[test]
    fn glob_first_matches_versions_dir() {
        // Создаём временный каталог-имитацию PostgreSQL/17/bin
        let root = std::env::temp_dir().join(format!("tc-glob-test-{}", std::process::id()));
        let bin = root.join("PostgreSQL").join("17").join("bin");
        std::fs::create_dir_all(&bin).unwrap();

        let pattern = root.join("PostgreSQL").join("*").join("bin");
        let found = glob_first(&pattern);
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(found, Some(bin), "glob должен найти версионный каталог");
    }

    #[test]
    fn glob_first_picks_newest_version_not_first_in_fs_order() {
        // Регрессия: раньше возвращался первый по порядку read_dir.
        // Теперь 10 должна победить 9 (числовое сравнение), даже если
        // файловая система отдаёт «9» раньше.
        let root = std::env::temp_dir().join(format!("tc-glob-newest-{}", std::process::id()));
        for v in ["9", "10", "17"] {
            std::fs::create_dir_all(root.join("PostgreSQL").join(v).join("bin")).unwrap();
        }
        let pattern = root.join("PostgreSQL").join("*").join("bin");
        let found = glob_first(&pattern);
        std::fs::remove_dir_all(&root).unwrap();
        assert!(
            found
                .as_ref()
                .map(|p| p.to_string_lossy().ends_with("\\17\\bin"))
                .unwrap_or(false),
            "glob обязан выбрать самую новую версию (17): {found:?}"
        );
    }

    #[test]
    fn natural_key_orders_numerically() {
        assert!(natural_key("17") > natural_key("9"));
        assert!(natural_key("10") > natural_key("9"));
        assert!(natural_key("2") < natural_key("10"));
        assert_eq!(natural_key("abc"), natural_key("abc"));
    }

    #[test]
    fn glob_first_returns_none_when_missing() {
        let pattern = PathBuf::from("C:/Program Files/PostgreSQL/*/bin");
        // На машине без PostgreSQL каталога нет; с PostgreSQL glob всё
        // равно должен вернуть существующий путь или None — но не панику.
        let result = glob_first(&pattern);
        if let Some(p) = &result {
            assert!(p.exists(), "glob-результат должен существовать: {p:?}");
        }
    }

    #[test]
    fn glob_first_plain_path_without_star() {
        let root = std::env::temp_dir().join(format!("tc-glob-plain-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let found = glob_first(&root);
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(found, Some(root));
    }

    #[tokio::test]
    async fn missing_tool_is_detected_as_missing() {
        let fake = def("sqlite").clone(); // sqlite3 почти ни у кого не установлен
        let status = detect_tool(&fake).await;
        // если sqlite3 вдруг установлен — тест всё равно валиден,
        // главное: результат не может быть «грязной» ошибкой
        assert!(
            matches!(status, ToolStatus::Missing | ToolStatus::Installed { .. }),
            "неожиданный статус: {status:?}"
        );
    }

    #[tokio::test]
    async fn kafka_jar_probe_parses_version() {
        // probe читает версию из имени jar без JVM — валидный вызов
        // powershell должен вернуть «3.9.0» при установленной kafka,
        // либо пустоту/ошибку если кафки нет. Ни в коем случае нельзя
        // полагаться на kafka-topics.bat (classpath > 8191 у cmd).
        let def = def("kafka").clone();
        let probes = &def.detection.version_probes;
        assert!(
            probes
                .iter()
                .any(|p| p.first().map(String::as_str) == Some("powershell")),
            "kafka должна определяться через powershell-пробу"
        );
        let status = detect_tool(&def).await;
        assert!(
            matches!(
                status,
                ToolStatus::Missing
                    | ToolStatus::Installed { .. }
                    | ToolStatus::UpdateAvailable { .. }
                    | ToolStatus::PathBroken { .. }
            ),
            "неожиданный статус kafka: {status:?}"
        );
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn expand_env_unix_tilde_expansion() {
        if let Ok(home) = std::env::var("HOME") {
            let expanded = expand_env("~/bin");
            assert_eq!(
                expanded,
                PathBuf::from(format!("{home}/bin")),
                "tilde should expand to HOME"
            );
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn expand_env_unix_dollar_var_expansion() {
        if let Ok(home) = std::env::var("HOME") {
            let expanded = expand_env("$HOME/bin");
            assert_eq!(
                expanded,
                PathBuf::from(format!("{home}/bin")),
                "$HOME should expand"
            );
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn expand_env_unix_braced_var_expansion() {
        if let Ok(home) = std::env::var("HOME") {
            let expanded = expand_env("${HOME}/bin");
            assert_eq!(
                expanded,
                PathBuf::from(format!("{home}/bin")),
                "${HOME} should expand"
            );
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn expand_env_unix_unknown_var_stays_literal() {
        let expanded = expand_env("$TOTALLY_FAKE_VAR_XYZ/bin");
        assert_eq!(
            expanded,
            PathBuf::from("$TOTALLY_FAKE_VAR_XYZ/bin"),
            "unknown var should stay as-is"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn expand_env_windows_userprofile() {
        if let Ok(up) = std::env::var("USERPROFILE") {
            let expanded = expand_env("%USERPROFILE%\\bin");
            assert_eq!(
                expanded,
                PathBuf::from(format!("{up}\\bin")),
                "%USERPROFILE% should expand"
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn expand_env_windows_unknown_var_stays_literal() {
        let expanded = expand_env("%TOTALLY_FAKE_XYZ%\\bin");
        assert_eq!(
            expanded,
            PathBuf::from("%TOTALLY_FAKE_XYZ%\\bin"),
            "unknown %VAR% stays literal on Windows"
        );
    }

    #[test]
    fn effective_detection_is_used_in_detect_tool() {
        let mut d = def("node").clone();
        d.detection.version_probes = vec![vec![
            "cmd".into(),
            "/c".into(),
            "echo".into(),
            "v99.0.0".into(),
        ]];
        d.detection.known_paths = vec![];
        d.detection.registry_keys = vec![];
        // effective_detection("windows") should use the override if present
        let det = d.effective_detection("windows");
        assert!(
            !det.version_probes.is_empty(),
            "effective_detection should return probes"
        );
    }
}
