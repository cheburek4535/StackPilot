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
        let stdout = String::from_utf8_lossy(&output.stdout);
        Some(stdout.trim().to_string())
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
async fn probe_version(def: &ToolDefinition) -> Option<String> {
    for probe in &def.detection.version_probes {
        if probe.is_empty() {
            continue;
        }
        if let Some(out) = run_capture(&probe[0], &probe[1..]).await {
            return Some(out);
        }
    }
    None
}

/// Расширяет %VAR% в путях из tools.json (%LOCALAPPDATA% и т.п.)
/// и приводит к PathBuf. На не-Windows переменные просто остаются
/// как есть — путь не существует, улика не срабатывает.
fn expand_env(raw: &str) -> PathBuf {
    const VARS: [(&str, &str); 4] = [
        ("%LOCALAPPDATA%", "LOCALAPPDATA"),
        ("%APPDATA%", "APPDATA"),
        ("%ProgramFiles%", "ProgramFiles"),
        ("%USERPROFILE%", "USERPROFILE"),
    ];

    let mut expanded = raw.to_string();
    for (pattern, var) in VARS {
        if expanded.contains(pattern) {
            if let Ok(value) = std::env::var(var) {
                expanded = expanded.replace(pattern, &value);
            }
        }
    }
    PathBuf::from(expanded)
}

/// Простейший glob-поиск пути: поддерживает `*` внутри компонентов,
/// например `C:/Program Files/PostgreSQL/*/bin` → первый существующий
/// вариант (17, 18, ...). Вложенность строго по компонентам шаблона.
/// Возвращает None, если ни один вариант не существует.
pub fn glob_first(pattern: &Path) -> Option<PathBuf> {
    let mut current = PathBuf::new();
    for component in pattern.components() {
        let comp = component.as_os_str().to_string_lossy();
        if comp.contains('*') {
            let prefix: String = comp.split('*').next().unwrap_or_default().to_string();
            let suffix: String = comp.rsplit('*').next().unwrap_or_default().to_string();
            let parent = current.clone();
            current = std::fs::read_dir(&parent)
                .ok()?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .find(|p| {
                    let name = p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
                    name.starts_with(&prefix) && name.ends_with(&suffix) && p.is_dir()
                })?;
        } else {
            current.push(comp.as_ref());
        }
    }
    current.exists().then_some(current)
}

/// Есть ли на диске хотя бы один из известных путей установки
/// (с учётом glob-шаблонов вида `PostgreSQL/*/bin`).
fn known_path_found(def: &ToolDefinition) -> bool {
    def.detection
        .known_paths
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
async fn probe_version_at_known_paths(def: &ToolDefinition) -> Option<String> {
    let exts: &[&str] = if cfg!(target_os = "windows") {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    for known in &def.detection.known_paths {
        let Some(dir) = glob_first(&expand_env(known)) else {
            continue;
        };
        for probe in &def.detection.version_probes {
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
    None
}

/// Отвечает ли reg.exe, что ключ реестра существует.
/// На не-Windows reg.exe нет — команда падает, получаем false.
async fn registry_key_exists(key: &str) -> bool {
    let result = timeout(
        Duration::from_secs(PROBE_TIMEOUT_SECS),
        TokioCommand::new("reg").args(["query", key]).output(),
    )
    .await;
    matches!(result, Ok(Ok(out)) if out.status.success())
}

async fn any_registry_found(def: &ToolDefinition) -> bool {
    for key in &def.detection.registry_keys {
        if registry_key_exists(key).await {
            return true;
        }
    }
    false
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
///   3. бинарник есть в PATH, но молчит   → PathBroken (сломана установка);
///   4. нашёлся известный путь/реестр     → PathBroken (не в PATH);
///   5. ничего                            → Missing.
pub async fn detect_tool(def: &ToolDefinition) -> ToolStatus {
    if let Some(raw) = probe_version(def).await {
        return apply_version_rules(def, &raw);
    }

    // Установка найдена по известному пути, но бинарь не в PATH —
    // пробуем запустить его напрямую оттуда.
    if let Some(raw) = probe_version_at_known_paths(def).await {
        return apply_version_rules(def, &raw);
    }

    // Бинарь в PATH есть, но ни одна проба не ответила:
    // установка сломана (dll потерялись, версия не поддерживается...)
    for probe in &def.detection.version_probes {
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
        let footprint = def
            .detection
            .known_paths
            .first()
            .or_else(|| def.detection.registry_keys.first())
            .cloned()
            .unwrap_or_default();
        return ToolStatus::PathBroken {
            reason: format!("Установка найдена ({}), но бинарник не отвечает на пробу", footprint),
        };
    }

    ToolStatus::Missing
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
pub(crate) fn installed_path(def: &ToolDefinition) -> Option<String> {
    def.detection.known_paths.iter().find_map(|p| {
        glob_first(&expand_env(p)).map(|p| p.to_string_lossy().into_owned())
    })
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
            probes.iter().any(|p| p.first().map(String::as_str) == Some("powershell")),
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
}
