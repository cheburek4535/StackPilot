// ============================================================
// Живое обнаружение одного инструмента (domain/detect.rs)
// ============================================================
// Расширенный детектор поверх примитивов discovery: различает
// НЕСКОЛЬКО установок, определяет видимость в PATH, честно
// отделяет «не найден» от «опрос не удался» (scan-failed ≠ missing).
//
// Запускаются ТОЛЬКО каталог-одобренные пробы из tools.json
// (version_probes / known_paths / registry_keys / health_checks) —
// произвольные команды извне сюда попасть не могут по типам.
// Все пробы идут через probe::run_probe: таймаут, лимит вывода,
// санитизация. Ничего на диск не пишется, окружение не меняется.

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::modules::toolchain::core::version;
use crate::modules::toolchain::models::{HealthCheck, ToolDefinition, ToolStatus};

use super::models::{
    DetectedInstall, DetectionOutcome, EvidenceKind, HealthCheckResult, HealthOutcome, HealthState,
    PathScope, PlatformApplicability, ProbeLog, Provenance, ToolPlatformCapabilities,
    ToolScanResult, ToolState, VersionAssessment,
};
use super::path_report;
use super::probe;

/// Таймаут одной проверки здоровья (может быть тяжелее пробы версии:
/// daemon-пинги и т.п.).
pub const HEALTH_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

// ------------------------------------------------------------
// Glob: ВСЕ совпадения (для различения нескольких установок)
// ------------------------------------------------------------

/// Как discovery::glob_first, но собирает ВСЕ существующие каталоги под
/// шаблон с `*` (по одному `*` на компонент), отсортированные по
/// естественно-числовому ключу (новые версии первыми). Только чтение ФС.
pub fn glob_all(pattern: &Path) -> Vec<PathBuf> {
    let comps: Vec<String> = pattern
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    glob_step(PathBuf::new(), &comps)
}

/// Шаг рекурсивного сопоставления: head-компонента + хвост.
/// Сравнение имён на Windows НЕЧУВСТВИТЕЛЬНО К РЕГИСТРУ (NTFS).
fn glob_step(base: PathBuf, comps: &[String]) -> Vec<PathBuf> {
    let Some((head, rest)) = comps.split_first() else {
        return if base.exists() {
            vec![base]
        } else {
            Vec::new()
        };
    };
    if head.contains('*') {
        let prefix: String = head.split('*').next().unwrap_or_default().to_string();
        let suffix: String = head.rsplit('*').next().unwrap_or_default().to_string();
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&base) {
            let mut candidates: Vec<PathBuf> = rd
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    let name = p
                        .file_name()
                        .map(|n| n.to_string_lossy())
                        .unwrap_or_default();
                    name.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase())
                        && name.to_ascii_lowercase().ends_with(&suffix.to_ascii_lowercase())
                        && p.is_dir()
                })
                .collect();
            // Новейшие версии первыми (естественно-числовой порядок).
            candidates.sort_by_key(natural_key_desc);
            for candidate in candidates {
                out.extend(glob_step(candidate, rest));
            }
        }
        out
    } else {
        glob_step(base.join(head), rest)
    }
}

/// Ключ сортировки «новые первыми»: обратный естественно-числовой порядок.
fn natural_key_desc(path: &PathBuf) -> std::cmp::Reverse<Vec<(u64, String)>> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
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
    std::cmp::Reverse(key)
}

/// Расширяет %VAR% в путях из tools.json (как discovery::expand_env).
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

/// Пробы, запускающие оболочку: их наличие в PATH ничего не говорит
/// об инструменте (паритет с discovery::is_shell_probe).
fn is_shell_probe(program: &str) -> bool {
    matches!(
        program.to_ascii_lowercase().as_str(),
        "powershell" | "pwsh" | "cmd" | "sh" | "bash" | "shell"
    )
}

/// Виден ли каталог в PATH процесса (нормализованное сравнение).
fn dir_on_path(dir: &Path, process_entries: &[String]) -> bool {
    let normalized = path_report::normalize_entry(&dir.to_string_lossy());
    process_entries
        .iter()
        .any(|e| path_report::normalize_entry(e) == normalized)
}

// ------------------------------------------------------------
// Детальное обнаружение
// ------------------------------------------------------------

/// Читает строковое значение InstallLocation из ключа реестра Windows.
/// NSIS/MSI установщики записывают реальный путь установки в ключ удаления.
/// Возвращает None, если: не Windows, reg.exe недоступен, ключ не найден,
/// или значение не является строкой (REG_SZ).
async fn read_registry_install_location(key: &str) -> Option<String> {
    let output = probe::run_probe(
        "reg",
        &[
            "query".to_string(),
            key.to_string(),
            "/v".to_string(),
            "InstallLocation".to_string(),
        ],
        probe::PROBE_TIMEOUT,
    )
    .await;

    if !output.success {
        return None;
    }

    // Формат вывода reg query:
    //   HKLM\...\erlang
    //       InstallLocation    REG_SZ    C:\Program Files\erlang\
    for line in output.stdout.lines() {
        let trimmed = line.trim();
        if trimmed.to_ascii_lowercase().starts_with("installlocation")
            && trimmed.contains("REG_SZ")
        {
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

/// Бинарь есть в PATH, но молчит: улика PathBroken с местом находки.
#[derive(Debug, Clone)]
pub struct SilentOnPath {
    /// Человекочитаемая причина (попадает в ToolState::PathBroken).
    pub reason: String,
    /// Разрешённый which() путь бинаря (улика остаётся объяснимой).
    pub location: String,
    /// Журнал неудавшейся пробы (полный лог для UI).
    pub probe_log: Option<ProbeLog>,
}

/// Результат детального опроса одного инструмента.
#[derive(Debug, Default)]
pub struct DetailedDetection {
    /// Все найденные установки/следы.
    pub installs: Vec<DetectedInstall>,
    /// Some(причина): опрос был неконclusive (таймауты/ошибки запуска),
    /// положительных улик нет → ScanFailed, а НЕ Missing.
    pub inconclusive: Option<String>,
    /// Бинарь есть в PATH, но молчит (PathBroken-улика).
    pub silent_on_path: Option<SilentOnPath>,
    /// В известном каталоге найден ожидаемый бинарь, но проба не
    /// ответила (ненулевой код/пустой вывод/таймаут/ошибка запуска) —
    /// вторая законная улика PathBroken. Обычный след без бинаря сюда
    /// НЕ попадает (контракт §4.2: footprint ≠ PathBroken).
    pub broken_known_path: Option<String>,
}

/// Полный опрос одного инструмента по каталогу.
pub async fn detect_detailed(
    def: &ToolDefinition,
    process_entries: &[String],
) -> DetailedDetection {
    let mut result = DetailedDetection::default();
    // Неконclusive-статистика собирается в первом (единственном) проходе:
    // повторный прогон проб удваивал бы время на «плохих» инструментах.
    let mut timeouts = 0usize;
    let mut launch_errors = 0usize;
    // Журналы НЕудавшихся проб: попадают в SilentOnPath-улику, чтобы UI
    // показывал ПОЛНЫЙ лог того, что именно не ответило.
    let mut failed_logs: Vec<(String, ProbeLog)> = Vec::new();

    // --- 1. Пробы версии через PATH ---
    for probe_cmd in &def.detection.version_probes {
        if probe_cmd.is_empty() {
            continue;
        }
        let started = Instant::now();
        let out = probe::run_probe(&probe_cmd[0], &probe_cmd[1..], probe::PROBE_TIMEOUT).await;
        let log = ProbeLog {
            command: probe_cmd.clone(),
            stdout: out.stdout.clone(),
            stderr: out.stderr.clone(),
            exit_code: out.exit_code,
            timed_out: out.timed_out,
            not_found: out.not_found,
            launch_error: out.launch_error.clone(),
            duration_ms: started.elapsed().as_millis() as u64,
        };
        if out.timed_out {
            timeouts += 1;
            failed_logs.push((probe_cmd[0].clone(), log));
            continue;
        }
        if let Some(err) = out.launch_error {
            launch_errors += 1;
            result.inconclusive = Some(format!(
                "проба `{}` не выполнилась: {err}",
                probe_cmd.join(" ")
            ));
            failed_logs.push((probe_cmd[0].clone(), log));
            continue;
        }
        // Ответ инструмента может идти в stderr (`java -version`,
        // `kotlinc -version`): успех определяется КОДОМ выхода, а не тем,
        // какой из потоков непуст. Пустой stdout больше не ломает вердикт.
        let output_text = if out.stdout.trim().is_empty() {
            out.stderr.clone()
        } else {
            out.stdout.clone()
        };
        if out.success && !output_text.trim().is_empty() {
            let location = which_location(&probe_cmd[0]);
            let mut install =
                versioned_install(&output_text, location, EvidenceKind::VersionProbe, true);
            install.probe_log = Some(log);
            result.installs.push(install);
            break; // первая ответившая проба достаточна (паритет с discovery)
        }
        // Ответил ненулевым кодом/пусто — пробуем следующую пробу.
        failed_logs.push((probe_cmd[0].clone(), log));
    }

    // Бинарь есть в PATH, но ни одна проба не ответила → PathBroken-улика.
    // Место находки и журнал неудавшейся пробы сохраняются: вердикт обязан
    // быть объяснимым из payload.
    //
    // ИСКЛЮЧЕНИЕ (bundled-инструменты): тул, который поставляется с другим
    // (pip с python, npm с node), не «сломан», когда его проба не ответила —
    // проба идёт ЧЕРЕЗ носитель (`python -m pip`), и её провал означает
    // «модуль/команда отсутствует», а НЕ «бинарь сломан». Ставить тут
    // PathBroken было бы ложью (пример: Python 3.14 без встроенного pip —
    // python.exe отвечает, pip просто не установлен). Такие тулы честно
    // классифицируются как Missing.
    if result.installs.is_empty() && def.bundled_with.is_none() {
        for probe_cmd in &def.detection.version_probes {
            if probe_cmd.is_empty() || is_shell_probe(&probe_cmd[0]) {
                continue;
            }
            if let Ok(path) = which::which(&probe_cmd[0]) {
                let probe_log = failed_logs
                    .iter()
                    .find(|(program, _)| program == &probe_cmd[0])
                    .map(|(_, log)| log.clone());
                result.silent_on_path = Some(SilentOnPath {
                    reason: format!(
                        "{} найден в PATH ({}), но не отвечает на `{}`",
                        probe_cmd[0],
                        path.to_string_lossy(),
                        probe_cmd.join(" ")
                    ),
                    location: path.to_string_lossy().into_owned(),
                    probe_log,
                });
                break;
            }
        }
    }

    // --- 2. Известные пути (все совпадения → несколько установок) ---
    for known in &def.detection.known_paths {
        for dir in glob_all(&expand_env(known)) {
            let probe = probe_known_dir(def, &dir, process_entries).await;
            // Первая зафиксированная улика «бинарь есть, но не отвечает».
            if result.broken_known_path.is_none() {
                result.broken_known_path = probe.broken_executable;
            }
            result.installs.push(probe.install);
        }
    }

    // --- 3. Реестр (Windows; на других ОС reg отсутствует — чистый промах) ---
    // NSIS/MSI установщики записывают InstallLocation в ключ удаления —
    // это единственный способ узнать реальный путь, когда пользователь
    // выбрал нестандартный каталог. Пробуем прочитать InstallLocation и
    // запустить бинарник оттуда; если не удалось — честный след «ключ
    // существует» (Footprint).
    if result.installs.is_empty() {
        for key in &def.detection.registry_keys {
            // Шаг A: читаем InstallLocation из ключа реестра.
            let install_location = read_registry_install_location(key).await;
            if let Some(loc) = install_location {
                let install_path = PathBuf::from(loc.trim_end_matches('\\'));
                // Пробуем bin/ подкаталог, затем сам каталог установки.
                for candidate in [
                    Some(install_path.join("bin")),
                    Some(install_path.clone()),
                ]
                .into_iter()
                .flatten()
                {
                    if candidate.is_dir() {
                        let probe = probe_known_dir(def, &candidate, process_entries).await;
                        if probe.install.raw_version.is_empty()
                            && probe.broken_executable.is_none()
                        {
                            continue; // бинарь не найден или не отвечает — пробуем дальше
                        }
                        if probe.broken_executable.is_some() {
                            result.broken_known_path = probe.broken_executable;
                        }
                        result.installs.push(probe.install);
                        break;
                    }
                }
                if !result.installs.is_empty() {
                    break;
                }
            }

            // Шаг B: fallback — просто проверяем наличие ключа (Footprint).
            let started = Instant::now();
            let out = probe::run_probe(
                "reg",
                &["query".to_string(), key.clone()],
                probe::PROBE_TIMEOUT,
            )
            .await;
            if out.success {
                result.installs.push(DetectedInstall {
                    raw_version: String::new(),
                    parsed_version: None,
                    location: key.clone(),
                    evidence: EvidenceKind::Footprint,
                    reachable_via_path: false,
                    path_scope: None,
                    probe_log: Some(ProbeLog {
                        command: vec!["reg".to_string(), "query".to_string(), key.clone()],
                        stdout: out.stdout.clone(),
                        stderr: out.stderr.clone(),
                        exit_code: out.exit_code,
                        timed_out: out.timed_out,
                        not_found: out.not_found,
                        launch_error: out.launch_error.clone(),
                        duration_ms: started.elapsed().as_millis() as u64,
                    }),
                });
                break;
            }
        }
    }

    // --- 4. Вердикт о неконclusive: улик нет + были таймауты/ошибки запуска ---
    if result.installs.is_empty() && result.silent_on_path.is_none() {
        if launch_errors == 0 && timeouts > 0 {
            result.inconclusive = Some(format!(
                "{timeouts} проб(ы) не уложились в таймаут {}с",
                probe::PROBE_TIMEOUT.as_secs()
            ));
        }
    } else {
        // Есть улика — неконclusive-причина не нужна.
        result.inconclusive = None;
    }

    result
}

/// Итог опроса одного известного каталога: след установки всегда
/// сохраняется, а «бинарь есть, но не отвечает» фиксируется отдельной
/// уликой (контракт §4.2: только она даёт право на PathBroken).
#[derive(Debug)]
struct KnownDirProbe {
    install: DetectedInstall,
    /// Some(причина) — в каталоге найден ожидаемый бинарь, но проба
    /// не ответила (таймаут/ошибка запуска/ненулевой код/пустой вывод).
    broken_executable: Option<String>,
}

/// Проба бинаря внутри известного каталога установки.
async fn probe_known_dir(
    def: &ToolDefinition,
    dir: &Path,
    process_entries: &[String],
) -> KnownDirProbe {
    let exts: &[&str] = if cfg!(target_os = "windows") {
        // Windows-обёртки (.bat/.cmd) — полноценные исполняемые: elixir.bat
        // и т.п. обязаны проверяться наравне с .exe.
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    let on_path = dir_on_path(dir, process_entries);
    // Найден ли хоть один кандидат-бинарь: отличает «каталог с бинарем,
    // который молчит» от «след без бинаря» (данные/остатки/ключ реестра).
    let mut saw_binary = false;
    let mut failure_note: Option<String> = None;
    let mut failure_log: Option<ProbeLog> = None;

    for probe_cmd in &def.detection.version_probes {
        if probe_cmd.is_empty() {
            continue;
        }
        for ext in exts {
            let bin = dir.join(format!("{}{}", probe_cmd[0], ext));
            if !bin.is_file() {
                continue;
            }
            saw_binary = true;
            let started = Instant::now();
            let out = probe::run_probe(
                &bin.to_string_lossy(),
                &probe_cmd[1..],
                probe::PROBE_TIMEOUT,
            )
            .await;
            let log = ProbeLog {
                command: {
                    let mut cmd = vec![bin.to_string_lossy().into_owned()];
                    cmd.extend(probe_cmd[1..].iter().cloned());
                    cmd
                },
                stdout: out.stdout.clone(),
                stderr: out.stderr.clone(),
                exit_code: out.exit_code,
                timed_out: out.timed_out,
                not_found: out.not_found,
                launch_error: out.launch_error.clone(),
                duration_ms: started.elapsed().as_millis() as u64,
            };
            if out.timed_out {
                failure_note = Some(format!(
                    "`{}` не уложился в таймаут {}с",
                    bin.to_string_lossy(),
                    probe::PROBE_TIMEOUT.as_secs()
                ));
                failure_log = Some(log);
                continue;
            }
            if let Some(err) = out.launch_error {
                failure_note = Some(format!(
                    "`{}` не удалось запустить: {err}",
                    bin.to_string_lossy()
                ));
                failure_log = Some(log);
                continue;
            }
            // `java -version` и `kotlinc -version` отвечают в stderr:
            // успех — по коду выхода, текст версии берём из любого потока.
            let output_text = if out.stdout.trim().is_empty() {
                out.stderr.clone()
            } else {
                out.stdout.clone()
            };
            if out.success && !output_text.trim().is_empty() {
                let mut install = versioned_install(
                    &output_text,
                    bin.to_string_lossy().into_owned(),
                    EvidenceKind::KnownPath,
                    on_path,
                );
                install.probe_log = Some(log);
                return KnownDirProbe {
                    install,
                    broken_executable: None,
                };
            }
            // Процесс выполнился, но версии не выдал (ненулевой код или пусто).
            failure_note = Some(match out.exit_code {
                Some(code) => format!(
                    "`{}` завершился с кодом {code} и не выдал версию",
                    bin.to_string_lossy()
                ),
                None => format!("`{}` не выдал версию", bin.to_string_lossy()),
            });
            failure_log = Some(log);
        }
    }

    // Каталог существует; след установки честный. PathBroken-улика —
    // ТОЛЬКО если в нём был ожидаемый бинарь и он не ответил.
    let mut install = DetectedInstall {
        raw_version: String::new(),
        parsed_version: None,
        location: dir.to_string_lossy().into_owned(),
        evidence: EvidenceKind::Footprint,
        reachable_via_path: on_path,
        path_scope: None,
        probe_log: None,
    };
    if saw_binary {
        install.probe_log = failure_log;
    }
    KnownDirProbe {
        install,
        broken_executable: if saw_binary { failure_note } else { None },
    }
}

/// Собирает DetectedInstall из сырого вывода пробы (версия парсится
/// устойчиво к мусору; непарсируемый вывод — всё равно улика установки,
/// сырая строка сохраняется и помечается непарсенной).
fn versioned_install(
    raw_output: &str,
    location: String,
    evidence: EvidenceKind,
    reachable_via_path: bool,
) -> DetectedInstall {
    let first_line = raw_output.lines().next().unwrap_or("").trim().to_string();
    match probe::extract_version_token(raw_output) {
        Some((raw, parsed)) => DetectedInstall {
            raw_version: raw,
            parsed_version: Some(probe::version_string(&parsed)),
            location,
            evidence,
            reachable_via_path,
            path_scope: None,
            probe_log: None,
        },
        None => DetectedInstall {
            raw_version: first_line,
            parsed_version: None,
            location,
            evidence,
            reachable_via_path,
            path_scope: None,
            probe_log: None,
        },
    }
}

fn which_location(program: &str) -> String {
    which::which(program)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ------------------------------------------------------------
// Проверки здоровья (явные объявления каталога)
// ------------------------------------------------------------

/// Прогоняет health_checks из каталога. Пустой список → NoChecksDefined
/// («не проверяли», а не «нездоров»). Различает процесс-отказ
/// (не запустился/таймаут) и провал условия (ненулевой код выхода).
///
/// Вердикт:
///   - все прошли → Healthy;
///   - процесс-отказ хотя бы одной проверки (запуск/таймаут) → FailedToRun
///     («не смогли проверить»);
///   - часть прошла, часть нет (без процесс-отказа) → Degraded: инструмент
///     работает (часть утверждений выполнена), но не в полном порядке.
///     Пример: Docker Desktop установлен (CLI отвечает), а daemon сейчас
///     не запущен — это деградация, а НЕ «нездоров»;
///   - ни одна не прошла (условия нарушены) → Unhealthy.
///
/// Деградация (Healthy при версии ниже рекомендуемой) уточняется
/// вызывающей стороной, которая знает оценку версии.
pub async fn run_health_checks(def: &ToolDefinition) -> HealthOutcome {
    if def.health_checks.is_empty() {
        return HealthOutcome {
            state: HealthState::NoChecksDefined,
            results: Vec::new(),
        };
    }

    let mut results = Vec::with_capacity(def.health_checks.len());
    for check in &def.health_checks {
        results.push(run_one_health_check(def, check).await);
    }

    let any_process_failure = results.iter().any(|r| r.process_failed);
    let all_passed = results.iter().all(|r| r.passed);
    let any_passed = results.iter().any(|r| r.passed);
    let state = if all_passed {
        HealthState::Healthy
    } else if any_process_failure {
        HealthState::FailedToRun
    } else if any_passed {
        HealthState::Degraded
    } else {
        HealthState::Unhealthy
    };

    HealthOutcome { state, results }
}

/// Запускает одну health-проверку. Если команда не найдена в PATH —
/// ищет бинарь в известных каталогах установки (glob) и пробует оттуда:
/// PostgreSQL/MySQL/Grafana и т.п. часто стоят БЕЗ добавления bin в PATH,
/// и проверка обязана найти установку так же, как её находит обнаружение.
async fn run_one_health_check(def: &ToolDefinition, check: &HealthCheck) -> HealthCheckResult {
    let started = Instant::now();
    let Some((program, args)) = check.command.split_first() else {
        return HealthCheckResult {
            label: check.label.clone(),
            command: check.command.clone(),
            passed: false,
            process_failed: true,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            timed_out: false,
            detail: "команда пустая (ошибка в tools.json)".to_string(),
            duration_ms: 0,
        };
    };

    let out = probe::run_probe(program, args, HEALTH_CHECK_TIMEOUT).await;

    // Бинарь не в PATH — пробуем известные каталоги установки (тот же
    // приём, что у обнаружения: install bin без PATH не означает «нет»).
    let out = if out.not_found {
        match probe_health_at_known_paths(def, program, args).await {
            Some(patched) => patched,
            None => out,
        }
    } else {
        out
    };
    let duration_ms = started.elapsed().as_millis() as u64;
    let timed_out = out.timed_out;

    if out.timed_out {
        return HealthCheckResult {
            label: check.label.clone(),
            command: check.command.clone(),
            passed: false,
            process_failed: true,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            timed_out,
            detail: format!(
                "таймаут {}с при выполнении `{}`",
                HEALTH_CHECK_TIMEOUT.as_secs(),
                check.command.join(" ")
            ),
            duration_ms,
        };
    }
    if let Some(err) = out.launch_error {
        return HealthCheckResult {
            label: check.label.clone(),
            command: check.command.clone(),
            passed: false,
            process_failed: true,
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            timed_out,
            detail: format!("не удалось запустить `{}`: {err}", check.command.join(" ")),
            duration_ms,
        };
    }
    if out.not_found {
        return HealthCheckResult {
            label: check.label.clone(),
            command: check.command.clone(),
            passed: false,
            process_failed: true,
            stdout: out.stdout.clone(),
            stderr: out.stderr.clone(),
            exit_code: None,
            timed_out,
            detail: format!("команда не найдена: {}", check.command.join(" ")),
            duration_ms,
        };
    }

    // Процесс выполнился: код выхода — утверждение из каталога.
    // Компактная сводка: stderr важнее stdout при провале (там живёт
    // диагностика daemon-подключений docker и т.п.), иначе stdout.
    let detail_source = if !out.stderr.trim().is_empty() && !out.success {
        out.stderr.clone()
    } else if !out.stdout.trim().is_empty() {
        out.stdout.clone()
    } else if !out.stderr.trim().is_empty() {
        out.stderr.clone()
    } else {
        format!("код выхода {}", out.exit_code.unwrap_or(0))
    };

    HealthCheckResult {
        label: check.label.clone(),
        command: check.command.clone(),
        passed: out.success,
        process_failed: false,
        stdout: out.stdout.clone(),
        stderr: out.stderr.clone(),
        exit_code: out.exit_code,
        timed_out,
        detail: detail_source,
        duration_ms,
    }
}

/// Запасная попытка health-проверки: команда не нашлась в PATH, но
/// бинарь с таким именем есть в известном каталоге установки (install
/// bin не обязан быть в PATH — PostgreSQL, MySQL, Grafana, ...).
/// Возвращает Some(вывод пробы), если бинарь найден и запустился.
async fn probe_health_at_known_paths(
    def: &ToolDefinition,
    program: &str,
    args: &[String],
) -> Option<probe::ProbeOutput> {
    let exts: &[&str] = if cfg!(target_os = "windows") {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    for known in &def.detection.known_paths {
        for dir in glob_all(&expand_env(known)) {
            for ext in exts {
                let bin = dir.join(format!("{program}{ext}"));
                if !bin.is_file() {
                    continue;
                }
                let out =
                    probe::run_probe(&bin.to_string_lossy(), args, HEALTH_CHECK_TIMEOUT).await;
                if !out.not_found && !out.timed_out {
                    return Some(out);
                }
            }
        }
    }
    None
}

// ------------------------------------------------------------
// Классификация и композиция состояния
// ------------------------------------------------------------

/// Применимость инструмента к платформе (чистая функция от данных каталога).
///
/// Порядок правил (важен):
///   1) ЯВНО заявленная доступность по ОС (`platform_availability` в
///      каталоге) исключает платформу раньше всего остального: Xcode
///      на Windows — «неприменим», а НЕ «ставьте вручную» (автоматической
///      установки там нет и быть не может);
///   2) «двойной» docker-инструмент Project Creator (в standalone-скане
///      dual_tools пуст — правило не срабатывает);
///   3) источник для этой ОС → Installable;
///   4) manual_install → ManualOnly;
///   5) источники есть, но не для этой ОС → UnsupportedOnPlatform;
///   6) иначе BuiltIn.
pub fn classify_applicability(
    def: &ToolDefinition,
    os_name: &str,
    is_dual_tool: bool,
) -> PlatformApplicability {
    let declared = &def.extended.platform_availability;
    if !declared.is_empty() && !declared.iter().any(|os| os == os_name) {
        return PlatformApplicability::UnsupportedOnPlatform;
    }
    if is_dual_tool {
        return PlatformApplicability::DockerDefault;
    }
    let os_sources = sources_for_os(def, os_name);
    if !os_sources.is_empty() {
        return PlatformApplicability::Installable;
    }
    if def.manual_install.is_some() {
        return PlatformApplicability::ManualOnly;
    }
    let any_sources = !def.sources.windows.is_empty()
        || !def.sources.linux.is_empty()
        || !def.sources.macos.is_empty();
    if any_sources {
        return PlatformApplicability::UnsupportedOnPlatform;
    }
    PlatformApplicability::BuiltIn
}

fn sources_for_os<'a>(
    def: &'a ToolDefinition,
    os: &str,
) -> &'a [crate::modules::toolchain::models::InstallSource] {
    match os {
        "windows" => &def.sources.windows,
        "linux" => &def.sources.linux,
        "macos" => &def.sources.macos,
        _ => &[],
    }
}

/// Оценка лучшей установки против правил каталога.
pub fn assess_versions(def: &ToolDefinition, installs: &[DetectedInstall]) -> VersionAssessment {
    let best = installs
        .iter()
        .filter_map(|i| i.parsed_version.as_deref())
        .filter_map(|v| version::parse_version(v).ok())
        .max_by(|a, b| version::compare(a, b));
    let Some(parsed) = best else {
        return if installs.iter().any(|i| !i.raw_version.is_empty()) {
            VersionAssessment::Unparseable
        } else {
            VersionAssessment::Unknown
        };
    };

    // Самопротиворечивая политика каталога (min > recommended): честно
    // сообщить о нарушении политики, а не выдавать произвольный вердикт.
    if let (Some(min_raw), Some(rec_raw)) = (&def.versions.min, &def.versions.recommended) {
        if let (Ok(min), Ok(rec)) = (
            version::parse_version(min_raw),
            version::parse_version(rec_raw),
        ) {
            if version::compare(&min, &rec) == std::cmp::Ordering::Greater {
                return VersionAssessment::PolicyViolation;
            }
        }
    }

    if let Some(min_raw) = &def.versions.min {
        if let Ok(min) = version::parse_version(min_raw) {
            if version::compare(&parsed, &min) == std::cmp::Ordering::Less {
                return VersionAssessment::BelowMin;
            }
        }
    }
    if let Some(rec_raw) = &def.versions.recommended {
        if let Ok(rec) = version::parse_version(rec_raw) {
            if version::compare(&parsed, &rec) == std::cmp::Ordering::Less {
                return VersionAssessment::BelowRecommended;
            }
        }
    }
    VersionAssessment::MeetsRecommended
}

/// Композиция презентационного состояния из размерностей
/// (правила контракта §3, §4.2; нормализация manual-тулов — паритет check.rs).
///
/// `path_broken_evidence` — ЯВНАЯ улика «установка есть, но исполняемый
/// файл не отвечает/недоступен через ожидаемый PATH»: молчащий в PATH
/// бинарь или найденный в известном каталоге бинарь с непройденной
/// пробой. Только она даёт право на ToolState::PathBroken; обычный след
/// (ключ реестра, каталог без бинаря) PathBroken не порождает.
///
/// `canonical` — индекс канонической установки в installs: главное
/// состояние собирается из НЕЁ, а не из «первой попавшейся».
#[allow(clippy::too_many_arguments)]
pub fn compose_state(
    def: &ToolDefinition,
    detection: &DetectionOutcome,
    installs: &[DetectedInstall],
    health: Option<&HealthOutcome>,
    applicability: PlatformApplicability,
    assessment: VersionAssessment,
    path_broken_evidence: Option<&str>,
    canonical: Option<usize>,
) -> ToolState {
    // Ошибка опроса важнее всего остального: неизвестно ≠ missing.
    if let DetectionOutcome::Failed { reason } = detection {
        return ToolState::ScanFailed {
            reason: reason.clone(),
        };
    }

    // Каноническая установка приоритетна; без неё — первая ответившая.
    let working_install = canonical
        .and_then(|i| installs.get(i))
        .filter(|i| !i.raw_version.is_empty() || i.parsed_version.is_some())
        .or_else(|| installs.iter().find(|i| !i.raw_version.is_empty()));

    if working_install.is_none() {
        if installs.is_empty() {
            // Ничего не нашли вовсе.
            if let Some(reason) = &def.manual_install {
                // Движок/SDK: отсутствие — ручная инструкция, не блокировка.
                if applicability != PlatformApplicability::UnsupportedOnPlatform {
                    return ToolState::ManualInstall {
                        reason: reason.clone(),
                    };
                }
            }
            return match applicability {
                PlatformApplicability::UnsupportedOnPlatform => ToolState::UnsupportedPlatform,
                PlatformApplicability::DockerDefault => ToolState::DockerManaged,
                // Bundled-инструмент (npm/pip/rustc/cargo) без источников:
                // классификация BuiltIn справедлива только для встроенных
                // в ОС (curl/tar). Отсутствие bundled-тула — честное
                // «не установлен», а НЕ «встроен в ОС».
                PlatformApplicability::BuiltIn if def.bundled_with.is_some() => ToolState::Missing,
                PlatformApplicability::BuiltIn => ToolState::BuiltInSystem,
                _ => ToolState::Missing,
            };
        }

        // Следы есть, рабочей пробы нет. Порядок честности (контракт §4.2):
        //   1) явная улика сломанного исполнения → PathBroken;
        //   2) иначе нейтральное «установка есть, проверить не удалось» —
        //      НЕ PathBroken и НЕ Missing (след ≠ отсутствие). Версия
        //      неизвестна честно пустой строкой; объяснение вердикта —
        //      в installs[] (location/evidence/reachable_via_path).
        // След сильнее текста manual_install: раз след найден, «ставьте
        // вручную» было бы ложью.
        if let Some(reason) = path_broken_evidence {
            return ToolState::PathBroken {
                reason: reason.to_string(),
            };
        }
        return ToolState::InstalledHealthUnknown {
            version: String::new(),
        };
    }

    let version = working_install
        .and_then(|i| i.parsed_version.clone())
        .or_else(|| working_install.map(|i| i.raw_version.clone()))
        .unwrap_or_default();

    // Здоровье: вердикт только для Healthy/Degraded/Unhealthy.
    // FailedToRun («не смогли выполнить проверку») — это «не знаем»,
    // а не «сломано»: показывается как InstalledHealthUnknown.
    let health_state = health.map(|h| h.state);
    let health_failed = matches!(health_state, Some(HealthState::Unhealthy));

    // «Доступно обновление» — честно ТОЛЬКО когда инструмент можно обновить
    // ЛОКАЛЬНО (у каталога есть источник на этой ОС). Bundled-тулы без
    // собственных источников (pip с python, npm с node) обновляются вместе
    // с хостом: заявлять «обновление доступно» для них фантомно — кнопки
    // нет, план не построить. Факт «ниже рекомендуемой/минимума» остаётся
    // в version_assessment и виден в деталях инструмента.
    let locally_updatable = applicability == PlatformApplicability::Installable;
    if locally_updatable && assessment == VersionAssessment::BelowMin {
        // Ниже минимума — это деградация с явной пометкой версии.
        return ToolState::UpdateAvailable {
            installed: version,
            // Рекомендуемая версия может быть не объявлена в каталоге:
            // пустая строка честно означает «не указана» (не выдумываем "?").
            recommended: def.versions.recommended.clone().unwrap_or_default(),
        };
    }
    if locally_updatable && assessment == VersionAssessment::BelowRecommended {
        return ToolState::UpdateAvailable {
            installed: version,
            recommended: def.versions.recommended.clone().unwrap_or_default(),
        };
    }
    if health_failed {
        return ToolState::InstalledUnhealthy { version };
    }
    match health_state {
        // Проверки прошли ИЛИ проверок не объявлено вовсе — вердикт
        // положительный. Ключевое отличие от проекта: сюда мы попадаем
        // ТОЛЬКО с РАБОЧЕЙ уликой (версия ответила), а ответившая проба
        // версии — это и есть утверждение «инструмент работает» (паритет
        // с интеграцией Project Creator, где ответившая проба = Installed).
        // NoChecksDefined НЕ означает «не проверяли» в смысле «не знаем»:
        // мы проверили запуск инструмента его же командой версии.
        Some(HealthState::Healthy)
        | Some(HealthState::Degraded)
        | Some(HealthState::NoChecksDefined) => ToolState::InstalledHealthy { version },
        Some(HealthState::Unhealthy) => ToolState::InstalledUnhealthy { version },
        _ => ToolState::InstalledHealthUnknown { version },
    }
}

// ------------------------------------------------------------
// Скан одного инструмента (переиспользуется движком и командами)
// ------------------------------------------------------------

/// Контекст скана: PATH процесса и происхождение из метаданных.
#[derive(Clone, Default)]
pub struct ScanContext {
    /// Записи PATH процесса (нормализованное сравнение внутри).
    pub process_entries: Vec<String>,
    /// Записи ПОСТОЯННОГО PATH (пользовательский + системный). Отдельно от
    /// процесса: «есть в постоянном PATH, но не в процессе» — отдельный
    /// честный класс доступности, а не «всё сломано» и не «всё работает».
    pub persisted_entries: Vec<String>,
    /// Инструменты, записанные в state.json (StackPilot-managed).
    pub managed_tools: std::collections::HashSet<String>,
    /// Имена «двойных» docker-инструментов каталога.
    pub dual_tools: std::collections::HashSet<String>,
    /// Имя ОС (платформенный слой).
    pub os_name: String,
}

/// Классификация доступности установки по слоям PATH (чистая функция):
/// процесс → постоянный → вне PATH. Для не-файловых следов (ключ
/// реестра) каталог видимости берётся по родителю строки — он не совпадёт
/// с записями PATH, что честно даёт OutsidePath («вне PATH»).
fn classify_path_scope(
    location: &str,
    process_entries: &[String],
    persisted_entries: &[String],
) -> PathScope {
    let norm = |s: &str| path_report::normalize_entry(s);
    let path = Path::new(location);
    // location может быть бинарем (берём родительский каталог) или уже
    // каталогом/следом (берём как есть).
    let dir = match path.parent() {
        Some(parent) if !path.is_dir() => parent.to_string_lossy().into_owned(),
        _ => location.to_string(),
    };
    let dir_norm = norm(&dir);
    if dir_norm.is_empty() {
        return PathScope::OutsidePath;
    }
    if process_entries.iter().any(|e| norm(e) == dir_norm) {
        PathScope::ProcessPath
    } else if persisted_entries.iter().any(|e| norm(e) == dir_norm) {
        PathScope::PersistedPathOnly
    } else {
        PathScope::OutsidePath
    }
}

fn normalize_location(location: &str) -> String {
    path_report::normalize_entry(location)
}

/// Одна и та же физическая установка? Совпадают нормализованное место И
/// разобранная версия. Разные версии в одном каталоге (или разные места)
/// — РАЗНЫЕ установки и сохраняются все (контракт: ретейн всех улик).
fn same_installation(a: &DetectedInstall, b: &DetectedInstall) -> bool {
    if a.evidence != EvidenceKind::VersionProbe || b.evidence != EvidenceKind::VersionProbe {
        return false;
    }
    if a.location.is_empty() || b.location.is_empty() {
        return false;
    }
    normalize_location(&a.location) == normalize_location(&b.location)
        && a.parsed_version == b.parsed_version
}

/// Слияние дублирующихся улик одной установки (python/python3, алиасы,
/// обёртки .bat, указывающие на тот же бинарь): дубликат НЕ создаёт
/// вторую запись «ещё одна установка», но более информативная улика
/// (разобранная версия, журнал пробы) сохраняется в первой записи.
pub fn dedupe_installs(installs: Vec<DetectedInstall>) -> Vec<DetectedInstall> {
    let mut out: Vec<DetectedInstall> = Vec::new();
    for mut install in installs {
        match out
            .iter()
            .position(|kept| same_installation(kept, &install))
        {
            Some(index) => {
                let kept = &mut out[index];
                if kept.parsed_version.is_none() && install.parsed_version.is_some() {
                    kept.raw_version = install.raw_version.clone();
                    kept.parsed_version = install.parsed_version.take();
                }
                if kept.probe_log.is_none() {
                    kept.probe_log = install.probe_log.take();
                }
            }
            None => out.push(install),
        }
    }
    out
}

/// Выбор канонической установки для главного состояния (детерминированно):
///   1) наибольшая разобранная версия (при равенстве — первая по порядку);
///   2) иначе первая ответившая проба с сырым выводом;
///   3) иначе первый след установки.
/// Возвращает индекс и ОБЪЯСНЕНИЕ выбора для UI (почему именно она).
pub fn select_canonical_install(installs: &[DetectedInstall]) -> Option<(usize, String)> {
    let mut best: Option<(usize, Vec<u32>)> = None;
    for (index, install) in installs.iter().enumerate() {
        let parsed = install
            .parsed_version
            .as_deref()
            .and_then(|v| version::parse_version(v).ok());
        let Some(parsed) = parsed else { continue };
        match &best {
            Some((_, best_v)) if version::compare(best_v, &parsed) != std::cmp::Ordering::Less => {}
            _ => best = Some((index, parsed)),
        }
    }

    if let Some((index, _)) = best {
        let total = installs.len();
        let reason = if total > 1 {
            format!(
                "выбрана наибольшая разобранная версия из {total} установок \
                 (установка #{}, остальные сохранены в списке)",
                index + 1
            )
        } else {
            "единственная обнаруженная установка".to_string()
        };
        return Some((index, reason));
    }

    if let Some(index) = installs.iter().position(|i| !i.raw_version.is_empty()) {
        return Some((
            index,
            "версия не разбирается: показан сырой вывод первой ответившей пробы".to_string(),
        ));
    }

    if !installs.is_empty() {
        return Some((
            0,
            "рабочей пробы нет: показан первый найденный след установки".to_string(),
        ));
    }
    None
}

/// Детект-вердикт строго по уликам (контракт §4.1):
///   Detected — есть положительные улики (installs / silent-on-path);
///   Failed — неконclusive БЕЗ улик: «неизвестно», не «отсутствует»;
///   NotDetected — чистый отрицательный результат.
/// Возврат NotDetected при найденных установках ЗАПРЕЩЁН контрактом.
/// Чистая функция: покрывается таблицей тестов без запуска процессов.
fn detection_verdict(detailed: &DetailedDetection) -> DetectionOutcome {
    if !detailed.installs.is_empty() || detailed.silent_on_path.is_some() {
        DetectionOutcome::Detected
    } else if let Some(reason) = &detailed.inconclusive {
        DetectionOutcome::Failed {
            reason: reason.clone(),
        }
    } else {
        DetectionOutcome::NotDetected
    }
}

/// Полное сканирование одного инструмента: обнаружение + здоровье +
/// PATH-находки + композиция состояния. Только чтение машины.
pub async fn scan_tool(def: &ToolDefinition, ctx: &ScanContext) -> ToolScanResult {
    let started = Instant::now();

    let detailed = detect_detailed(def, &ctx.process_entries).await;
    let detection = detection_verdict(&detailed);
    let has_evidence = !detailed.installs.is_empty() || detailed.silent_on_path.is_some();

    // Здоровье определяется НЕЗАВИСИМО от разбора версии (контракт §4.3):
    // прогоны запускаются при любых уликах установки, даже если версию
    // разобрать не удалось. Мусорная версия НЕ отменяет health-проверки;
    // наоборот, их вердикт часто и объясняет «мусорный» ответ инструмента.
    let mut health = if has_evidence {
        Some(run_health_checks(def).await)
    } else if detection_failed(&detection) {
        None
    } else {
        Some(HealthOutcome {
            state: HealthState::Unavailable,
            results: Vec::new(),
        })
    };

    let applicability = classify_applicability(def, &ctx.os_name, ctx.dual_tools.contains(&def.id));
    let assessment = assess_versions(def, &detailed.installs);

    // Деградация: проверки прошли, но версия ниже рекомендуемой/минимума —
    // работает, но не в полном порядке (контракт §4).
    if let Some(outcome) = health.as_mut() {
        if outcome.state == HealthState::Healthy
            && matches!(
                assessment,
                VersionAssessment::BelowRecommended | VersionAssessment::BelowMin
            )
        {
            outcome.state = HealthState::Degraded;
        }
    }

    // Silent-on-path становится честным следом: бинарь в PATH есть (место
    // сохранено), версии он не выдал. Улики не выбрасываются.
    let mut installs = detailed.installs;
    if let Some(silent) = &detailed.silent_on_path {
        if installs.is_empty() {
            installs.push(DetectedInstall {
                raw_version: String::new(),
                parsed_version: None,
                location: silent.location.clone(),
                evidence: EvidenceKind::Footprint,
                reachable_via_path: true,
                path_scope: None,
                probe_log: silent.probe_log.clone(),
            });
        }
    }

    // Дубликаты одной физической установки (алиасы/обёртки: python/python3)
    // сливаются; РАЗНЫЕ установки сохраняются все.
    let installs = dedupe_installs(installs);

    // Классификация PATH по слоям для каждой улики (правда, не догадка).
    let mut installs = installs;
    for install in &mut installs {
        install.path_scope = Some(classify_path_scope(
            &install.location,
            &ctx.process_entries,
            &ctx.persisted_entries,
        ));
    }

    // Каноническая установка для главного состояния + объяснение выбора.
    let (canonical_install, version_selected_because) = match select_canonical_install(&installs) {
        Some((index, reason)) => (Some(index), reason),
        None => (None, String::new()),
    };

    // PathBroken допустим ТОЛЬКО при явной улике сломанного исполнения:
    // молчащий в PATH бинарь ИЛИ найденный в известном каталоге ожидаемый
    // бинарь, который не ответил на пробу. Обычный след без бинаря
    // (каталог-остатки, ключ реестра) — НЕ PathBroken (контракт §4.2).
    let path_broken_reason = detailed
        .silent_on_path
        .as_ref()
        .map(|s| s.reason.clone())
        .or(detailed.broken_known_path.clone());

    let state = compose_state(
        def,
        &detection,
        &installs,
        health.as_ref(),
        applicability,
        assessment,
        path_broken_reason.as_deref(),
        canonical_install,
    );

    // PATH-находки имеют смысл для ЛЮБЫХ найденных установок, включая
    // следы вне PATH: это диагностика («бинарь есть, PATH молчит»),
    // а не отдельный вердикт.
    let path_findings = if has_evidence {
        path_report::tool_path_findings(def, &installs, &ctx.process_entries)
    } else {
        Vec::new()
    };

    // Происхождение: только факты, без догадок.
    //   - запись в state.json → ставили мы;
    //   - встроенный в ОС → System;
    //   - найден и идёт в комплекте с другим (npm→node) → BundledWith;
    //   - двойной docker-инструмент, ЛОКАЛЬНЫХ УЛИК НЕТ ВОВСЕ → Docker
    //     (живёт в docker-compose проекта); локальный след — не «Docker»;
    //   - найден сам по себе → External; нет данных → Unknown.
    // PackageManager здесь НЕ угадывается: надёжного «кто ставил» для
    // winget/apt на этапе скана нет — это остаётся Unknown/External.
    let responds = installs.iter().any(|i| !i.raw_version.is_empty());
    let provenance = if ctx.managed_tools.contains(&def.id) {
        Provenance::StackPilotManaged
    } else if matches!(state, ToolState::BuiltInSystem) {
        Provenance::System
    } else if responds && def.bundled_with.is_some() {
        Provenance::BundledWith {
            tool: def.bundled_with.clone().unwrap_or_default(),
        }
    } else if installs.is_empty() && applicability == PlatformApplicability::DockerDefault {
        Provenance::Docker
    } else if responds {
        Provenance::External
    } else {
        Provenance::Unknown
    };

    // Ошибка опроса сохраняется в результате: вердикт обязан быть
    // объяснимым из самого payload (контракт §4).
    let error = match &detection {
        DetectionOutcome::Failed { reason } => Some(reason.clone()),
        _ => None,
    };

    ToolScanResult {
        tool_id: def.id.clone(),
        display: def.display.clone(),
        category: def.category.clone(),
        icon: def.icon.clone(),
        detection,
        installs,
        path_findings,
        health,
        applicability,
        capabilities: ToolPlatformCapabilities::for_definition(
            def,
            &ctx.os_name,
            crate::modules::toolchain::core::installer::install_execution_supported(),
        ),
        provenance,
        bundled_with: def.bundled_with.clone(),
        version_assessment: assessment,
        state,
        error,
        canonical_install,
        version_selected_because,
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

fn detection_failed(detection: &DetectionOutcome) -> bool {
    matches!(detection, DetectionOutcome::Failed { .. })
}

/// Совместимость со старым статусом (для legacy-адаптеров).
/// Часть зафиксированного compat-поверхности (контракт §8.8): пока
/// старые `tc_*` потребители существуют, отображение ToolState →
/// ToolStatus обязано оставаться корректным и покрытым таблицей тестов.
///
/// ОГРАНИЧЕНИЕ ЛЕГАСИ-СЛОЯ (документировано): у ToolStatus нет варианта
/// «неизвестно», поэтому ScanPending/ScanFailed сжимаются в Missing.
/// Standalone-поверхность это НЕ использует (для неё «неизвестно ≠
/// отсутствует» гарантирует DetectionOutcome/ScanFailed в снапшоте);
/// отображение существует только для byte-compat легаси-потребителей.
#[cfg_attr(not(test), allow(dead_code))]
pub fn to_legacy_status(result: &ToolScanResult) -> ToolStatus {
    match &result.state {
        ToolState::Missing | ToolState::UnsupportedPlatform => ToolStatus::Missing,
        ToolState::InstalledHealthy { version }
        | ToolState::InstalledHealthUnknown { version }
        | ToolState::InstalledUnhealthy { version } => ToolStatus::Installed {
            version: version.clone(),
        },
        ToolState::UpdateAvailable {
            installed,
            recommended,
        } => ToolStatus::UpdateAvailable {
            installed: installed.clone(),
            recommended: recommended.clone(),
        },
        ToolState::PathBroken { reason } => ToolStatus::PathBroken {
            reason: reason.clone(),
        },
        ToolState::ManualInstall { reason } => ToolStatus::ManualInstall {
            reason: reason.clone(),
        },
        ToolState::DockerManaged => ToolStatus::RunInDocker,
        ToolState::BuiltInSystem => ToolStatus::Installed {
            version: String::new(),
        },
        ToolState::ScanPending => ToolStatus::Missing,
        ToolState::ScanFailed { .. } => ToolStatus::Missing,
    }
}

/// Правила обнаружения доступны для тестов (валидация каталога).
#[cfg(test)]
pub(crate) fn rules_have_any_evidence(
    rules: &crate::modules::toolchain::models::DetectionRules,
) -> bool {
    !rules.version_probes.is_empty()
        || !rules.known_paths.is_empty()
        || !rules.registry_keys.is_empty()
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::toolchain::defs;
    use crate::modules::toolchain::models::{
        DetectionRules, InstallSource, InstallSourceKind, InstallSources,
    };

    fn fake_def(id: &str) -> ToolDefinition {
        defs::load_definitions()
            .into_iter()
            .find(|d| d.id == id)
            .unwrap_or_else(|| panic!("{id} нет в tools.json"))
    }

    fn empty_rules_def(id: &str) -> ToolDefinition {
        ToolDefinition {
            id: id.to_string(),
            category: "utility".to_string(),
            display: "Fake".to_string(),
            description: String::new(),
            icon: None,
            detection: DetectionRules {
                version_probes: vec![vec!["definitely-missing-tool-xyz".to_string()]],
                known_paths: vec![],
                registry_keys: vec![],
                ..Default::default()
            },
            versions: Default::default(),
            sources: InstallSources {
                windows: vec![InstallSource {
                    kind: InstallSourceKind::PkgManager,
                    id: "Fake.Id".to_string(),
                    url: None,
                    args: vec![],
                    extra_args: vec![],
                    dynamic_args: false,
                    install_dir: None,
                    needs_admin: None,
                    file_name: None,
                    execution: None,
                    sha256: None,
                }],
                linux: vec![],
                macos: vec![],
            },
            size_mb: 10,
            needs_admin: false,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
            manual_install: None,
            extended: Default::default(),
        }
    }

    #[test]
    fn glob_all_finds_every_version_dir_newest_first() {
        let root = std::env::temp_dir().join(format!("tc-glob-all-{}", std::process::id()));
        for v in ["9", "10", "17"] {
            std::fs::create_dir_all(root.join("PG").join(v).join("bin")).unwrap();
        }
        let pattern = root.join("PG").join("*").join("bin");
        let found = glob_all(&pattern);
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(found.len(), 3, "все версии должны найтись: {found:?}");
        assert!(
            found[0].to_string_lossy().ends_with("\\17\\bin")
                || found[0].to_string_lossy().ends_with("/17/bin"),
            "новейшая версия первая: {found:?}"
        );
    }

    #[test]
    fn glob_all_plain_path() {
        let root = std::env::temp_dir().join(format!("tc-glob-plain2-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(glob_all(&root), vec![root.clone()]);
        std::fs::remove_dir(&root).unwrap();
    }

    #[tokio::test]
    async fn missing_tool_is_not_detected_not_failed() {
        let def = empty_rules_def("fake-missing");
        let ctx = ScanContext {
            process_entries: vec![],
            persisted_entries: vec![],
            managed_tools: Default::default(),
            dual_tools: Default::default(),
            os_name: "windows".to_string(),
        };
        let result = scan_tool(&def, &ctx).await;
        // Чистый отрицательный результат: не Failed и не Detected.
        assert!(matches!(result.detection, DetectionOutcome::NotDetected));
        assert_eq!(result.state, ToolState::Missing);
        assert_eq!(result.applicability, PlatformApplicability::Installable);
        assert!(result.error.is_none());
        assert!(result.installs.is_empty());
    }

    #[tokio::test]
    async fn echo_probe_def_is_detected_healthy() {
        // Проба cmd /c echo отвечает → установлен; проверок здоровья нет,
        // но ответившая проба версии — сама по себе утверждение «работает»
        // (паритет интеграции Project Creator: ответившая проба = Installed).
        let mut def = empty_rules_def("fake-echo");
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "5.5.5".to_string(),
        ]];
        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        // КРИТИЧЕСКАЯ РЕГРЕССИЯ: успешное обнаружение обязано быть
        // Detected — NotDetected при найденной установке запрещён.
        assert!(
            matches!(result.detection, DetectionOutcome::Detected),
            "успешное обнаружение потеряло вердикт Detected: {:?}",
            result.detection
        );
        assert_eq!(result.installs.len(), 1);
        assert_eq!(result.installs[0].parsed_version.as_deref(), Some("5.5.5"));
        assert_eq!(result.installs[0].evidence, EvidenceKind::VersionProbe);
        assert_eq!(
            result.state,
            ToolState::InstalledHealthy {
                version: "5.5.5".to_string()
            }
        );
        assert_eq!(result.provenance, Provenance::External);
    }

    #[tokio::test]
    async fn malformed_version_output_still_detected_as_unparseable() {
        // Мусорный вывод пробы: инструмент установлен, версия Unparseable.
        let mut def = empty_rules_def("fake-garbage");
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "build".to_string(),
            "unknown".to_string(),
        ]];
        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        assert!(
            matches!(result.detection, DetectionOutcome::Detected),
            "непарсируемый вывод — всё равно обнаружение: {:?}",
            result.detection
        );
        assert_eq!(result.installs.len(), 1);
        assert!(result.installs[0].parsed_version.is_none());
        assert_eq!(result.version_assessment, VersionAssessment::Unparseable);
        // Проба ответила (хоть и мусором) — инструмент работает.
        assert!(matches!(result.state, ToolState::InstalledHealthy { .. }));
    }

    #[tokio::test]
    async fn no_checks_defined_is_healthy_when_probe_answered() {
        // Проба версии ответила, а health_checks в каталоге нет:
        // ответившая проба — положительное утверждение, состояние —
        // InstalledHealthy (а не «здоровье не проверялось»).
        let mut def = empty_rules_def("fake-nochecks");
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "1".to_string(),
        ]];
        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        let health = result.health.expect("health должен присутствовать");
        assert_eq!(health.state, HealthState::NoChecksDefined);
        assert!(health.results.is_empty());
        assert!(health.all_passed().is_none(), "нет вердикта — нет и оценки");
        assert!(matches!(result.state, ToolState::InstalledHealthy { .. }));
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn failing_health_check_marks_unhealthy_with_assertion_kind() {
        let mut def = empty_rules_def("fake-badcheck");
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "1".to_string(),
        ]];
        def.health_checks = vec![HealthCheck {
            label: "always fails".to_string(),
            command: vec![
                "cmd".to_string(),
                "/c".to_string(),
                "exit".to_string(),
                "/b".to_string(),
                "3".to_string(),
            ],
        }];
        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        let health = result.health.unwrap();
        assert_eq!(health.state, HealthState::Unhealthy);
        assert_eq!(health.all_passed(), Some(false));
        assert!(!health.results[0].passed);
        assert!(
            !health.results[0].process_failed,
            "процесс выполнился — это провал УСЛОВИЯ, не процесса"
        );
        assert!(matches!(result.state, ToolState::InstalledUnhealthy { .. }));
    }

    /// Частичный провал проверок (часть прошла, часть нет, без
    /// процесс-отказа) — Degraded, а НЕ Unhealthy: инструмент работает,
    /// но не в полном порядке. Регрессия «Docker Desktop установлен,
    /// CLI отвечает, а daemon сейчас не запущен» — деградация, не «нездоров».
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn mixed_health_checks_are_degraded_not_unhealthy() {
        let mut def = empty_rules_def("fake-mixed");
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "1".to_string(),
        ]];
        def.health_checks = vec![
            HealthCheck {
                label: "passes".to_string(),
                command: vec![
                    "cmd".to_string(),
                    "/c".to_string(),
                    "echo".to_string(),
                    "ok".to_string(),
                ],
            },
            HealthCheck {
                label: "fails".to_string(),
                command: vec![
                    "cmd".to_string(),
                    "/c".to_string(),
                    "exit".to_string(),
                    "/b".to_string(),
                    "3".to_string(),
                ],
            },
        ];
        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        let health = result.health.unwrap();
        assert_eq!(health.state, HealthState::Degraded);
        assert!(health.results[0].passed);
        assert!(!health.results[1].passed);
        assert!(!health.results.iter().any(|r| r.process_failed));
        // Деградация презентационно — «установлен и работает».
        assert!(matches!(result.state, ToolState::InstalledHealthy { .. }));
    }

    /// Все проверки упали как условия (без процесс-отказа) — Unhealthy:
    /// частичный успех не размывает полный провал.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn all_assertions_failing_is_unhealthy_not_degraded() {
        let mut def = empty_rules_def("fake-allfail");
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "1".to_string(),
        ]];
        def.health_checks = vec![
            HealthCheck {
                label: "fails 1".to_string(),
                command: vec![
                    "cmd".to_string(),
                    "/c".to_string(),
                    "exit".to_string(),
                    "/b".to_string(),
                    "3".to_string(),
                ],
            },
            HealthCheck {
                label: "fails 2".to_string(),
                command: vec![
                    "cmd".to_string(),
                    "/c".to_string(),
                    "exit".to_string(),
                    "/b".to_string(),
                    "4".to_string(),
                ],
            },
        ];
        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        let health = result.health.unwrap();
        assert_eq!(health.state, HealthState::Unhealthy);
        assert!(matches!(result.state, ToolState::InstalledUnhealthy { .. }));
    }

    /// Bundled-инструмент (pip при python): проба через носитель не
    /// ответила (модуль не установлен), а НОСИТЕЛЬ в PATH есть — это НЕ
    /// «PATH сломан» (бинарь носителя исправен), а честное Missing:
    /// pip просто не установлен. Регрессия «Python 3.14 без pip».
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn bundled_tool_with_failed_probe_is_missing_not_path_broken() {
        let mut def = empty_rules_def("fake-pip");
        // Проба через «носитель» (аналог `python -m pip --version`),
        // завершается неуспешно — модуля нет.
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "exit".to_string(),
            "/b".to_string(),
            "3".to_string(),
        ]];
        def.bundled_with = Some("fake-host".to_string());

        let ctx = ScanContext {
            process_entries: vec![],
            persisted_entries: vec![],
            managed_tools: Default::default(),
            dual_tools: Default::default(),
            os_name: "windows".to_string(),
        };
        let result = scan_tool(&def, &ctx).await;

        assert!(
            !matches!(result.state, ToolState::PathBroken { .. }),
            "bundled-тул без пробы не имеет права быть PathBroken: {:?}",
            result.state
        );
        assert_eq!(result.state, ToolState::Missing);
        assert!(matches!(result.detection, DetectionOutcome::NotDetected));
        assert!(result.installs.is_empty());
    }

    /// Health-проверка падает на PATH, но бинарь есть в известном каталоге
    /// установки (PostgreSQL/MySQL/Grafana ставятся без bin в PATH):
    /// проверка обязана найти его и пройти.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn health_check_falls_back_to_known_paths() {
        use std::io::Write as _;

        let dir = unique_temp_dir("healthfp");
        let bin = dir.join("dbcheck.cmd");
        let mut f = std::fs::File::create(&bin).unwrap();
        f.write_all(b"@echo off\r\necho healthy-db\r\n").unwrap();
        drop(f);

        let mut def = empty_rules_def("fake-db");
        def.detection.version_probes = vec![vec!["dbcheck".to_string()]];
        def.detection.known_paths = vec![dir.to_string_lossy().into_owned()];
        def.health_checks = vec![HealthCheck {
            label: "db alive".to_string(),
            command: vec!["dbcheck".to_string(), "--version".to_string()],
        }];

        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        let health = result.health.expect("проверки объявлены");
        assert_eq!(
            health.state,
            HealthState::Healthy,
            "проверка нашла бинарь в known_paths"
        );
        assert!(health.results[0].passed);
        assert!(!health.results[0].process_failed);

        let _ = std::fs::remove_file(&bin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_platform_classified_for_foreign_os() {
        let def = empty_rules_def("win-only");
        // Источники только windows → на linux неприменим.
        assert_eq!(
            classify_applicability(&def, "linux", false),
            PlatformApplicability::UnsupportedOnPlatform
        );
        assert_eq!(
            classify_applicability(&def, "windows", false),
            PlatformApplicability::Installable
        );
    }

    /// ЯВНО заявленная доступность (platform_availability) сильнее
    /// manual_install: Xcode на Windows — «неприменим» (никогда не
    /// ставится автоматически и не «ручная установка»), а на macOS
    /// остаётся ручным. Правило стоит ПЕРЕЕ manual-классификации.
    #[test]
    fn declared_platform_availability_excludes_os_before_manual() {
        let mut def = empty_rules_def("xcode-like");
        def.sources = InstallSources::default();
        def.manual_install = Some("Только Mac App Store".to_string());
        def.extended.platform_availability = vec!["macos".to_string()];

        assert_eq!(
            classify_applicability(&def, "windows", false),
            PlatformApplicability::UnsupportedOnPlatform,
            "Xcode никогда не устанавливается на Windows"
        );
        assert_eq!(
            classify_applicability(&def, "linux", false),
            PlatformApplicability::UnsupportedOnPlatform
        );
        assert_eq!(
            classify_applicability(&def, "macos", false),
            PlatformApplicability::ManualOnly,
            "на macOS — честное «ставится вручную»"
        );
        // Пустое заявление ничего не исключает (старые записи каталога).
        def.extended.platform_availability.clear();
        assert_eq!(
            classify_applicability(&def, "windows", false),
            PlatformApplicability::ManualOnly
        );
    }

    #[test]
    fn builtin_classified_when_no_sources_anywhere() {
        let mut def = empty_rules_def("builtin");
        def.sources = InstallSources::default();
        assert_eq!(
            classify_applicability(&def, "linux", false),
            PlatformApplicability::BuiltIn
        );
    }

    #[test]
    fn docker_dual_tool_classified_before_sources() {
        let def = empty_rules_def("dual");
        assert_eq!(
            classify_applicability(&def, "windows", true),
            PlatformApplicability::DockerDefault
        );
    }

    #[test]
    fn manual_only_classified_when_no_local_sources_but_manual_text() {
        let mut def = empty_rules_def("manual");
        def.sources = InstallSources::default();
        def.manual_install = Some("Ставится вручную".to_string());
        assert_eq!(
            classify_applicability(&def, "windows", false),
            PlatformApplicability::ManualOnly
        );
    }

    #[test]
    fn assess_versions_respects_min_and_recommended() {
        let mut def = fake_def("node"); // min 18, recommended 22
        def.versions.min = Some("18".to_string());
        def.versions.recommended = Some("22".to_string());

        let mk = |v: &str| DetectedInstall {
            raw_version: v.to_string(),
            parsed_version: Some(v.to_string()),
            location: String::new(),
            evidence: EvidenceKind::VersionProbe,
            reachable_via_path: true,
            path_scope: None,
            probe_log: None,
        };

        assert_eq!(
            assess_versions(&def, &[mk("24.1.0")]),
            VersionAssessment::MeetsRecommended
        );
        assert_eq!(
            assess_versions(&def, &[mk("20.2.0")]),
            VersionAssessment::BelowRecommended
        );
        assert_eq!(
            assess_versions(&def, &[mk("16.0.0")]),
            VersionAssessment::BelowMin
        );
    }

    #[test]
    fn multiple_installs_best_version_wins_assessment() {
        let def = fake_def("node");
        let mk = |v: &str| DetectedInstall {
            raw_version: v.to_string(),
            parsed_version: Some(v.to_string()),
            location: v.to_string(),
            evidence: EvidenceKind::KnownPath,
            reachable_via_path: false,
            path_scope: None,
            probe_log: None,
        };
        // Старая и новая установки рядом: оценка по лучшей.
        assert_eq!(
            assess_versions(&def, &[mk("18.0.0"), mk("24.0.0")]),
            VersionAssessment::MeetsRecommended
        );
    }

    #[test]
    fn legacy_mapping_covers_states() {
        let mut r = scan_result_stub(ToolState::Missing);
        assert!(matches!(to_legacy_status(&r), ToolStatus::Missing));
        r.state = ToolState::DockerManaged;
        assert!(matches!(to_legacy_status(&r), ToolStatus::RunInDocker));
        r.state = ToolState::UpdateAvailable {
            installed: "1".into(),
            recommended: "2".into(),
        };
        assert!(matches!(
            to_legacy_status(&r),
            ToolStatus::UpdateAvailable { .. }
        ));
    }

    // ------------------------------------------------------------
    // Версионные состояния
    // ------------------------------------------------------------

    #[tokio::test]
    async fn self_contradictory_policy_is_policy_violation() {
        // min > recommended — сломанная политика каталога: честный
        // PolicyViolation вместо произвольного вердикта.
        let mut def = empty_rules_def("fake-policy");
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "1.0.0".to_string(),
        ]];
        def.versions.min = Some("3.0".to_string());
        def.versions.recommended = Some("2.0".to_string());

        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        assert_eq!(
            result.version_assessment,
            VersionAssessment::PolicyViolation
        );
    }

    #[tokio::test]
    async fn below_recommended_is_degraded_health_and_update_state() {
        // Проверки проходят, но версия ниже рекомендуемой: работает
        // с деградацией (HealthState::Degraded) и просит обновление.
        let mut def = empty_rules_def("fake-degraded");
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "1.0.0".to_string(),
        ]];
        def.versions.recommended = Some("2.0".to_string());
        def.health_checks = vec![HealthCheck {
            label: "always ok".to_string(),
            command: vec!["cmd".to_string(), "/c".to_string(), "echo".to_string()],
        }];

        let mut ctx = ScanContext::default();
        ctx.os_name = "windows".into();
        let result = scan_tool(&def, &ctx).await;

        assert_eq!(
            result.version_assessment,
            VersionAssessment::BelowRecommended
        );
        let health = result.health.expect("проверки объявлены");
        assert_eq!(health.state, HealthState::Degraded);
        assert_eq!(health.all_passed(), Some(true));
        assert!(matches!(result.state, ToolState::UpdateAvailable { .. }));
    }

    /// Регрессия «фантомное обновление у bundled-тула»: pip (нет собственных
    /// источников) ниже рекомендуемой версии НЕ должен показываться как
    /// «Доступно обновление» — обновить его отдельно невозможно; факт
    /// версии остаётся в version_assessment.
    #[tokio::test]
    async fn bundled_tool_without_sources_below_recommended_is_not_update_available() {
        let mut def = empty_rules_def("fake-pip");
        def.sources = InstallSources::default();
        def.bundled_with = Some("fake-python".to_string());
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "24.0".to_string(),
        ]];
        def.versions.recommended = Some("25".to_string());

        let mut ctx = ScanContext::default();
        ctx.os_name = "windows".into();
        let result = scan_tool(&def, &ctx).await;

        assert_eq!(result.applicability, PlatformApplicability::BuiltIn);
        assert_eq!(
            result.version_assessment,
            VersionAssessment::BelowRecommended,
            "факт версии остаётся честным"
        );
        assert!(
            !matches!(result.state, ToolState::UpdateAvailable { .. }),
            "без локального источника «обновление доступно» — фантом: {:?}",
            result.state
        );
    }

    #[tokio::test]
    async fn failed_to_run_health_is_unknown_not_unhealthy() {
        // Проверка не запускается (бинаря нет): «не смогли проверить»,
        // НЕ «сломано» — презентационно InstalledHealthUnknown.
        let mut def = empty_rules_def("fake-failed-run");
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "9.9.9".to_string(),
        ]];
        def.health_checks = vec![HealthCheck {
            label: "missing binary".to_string(),
            command: vec![
                "definitely-missing-tool-xyz".to_string(),
                "--version".to_string(),
            ],
        }];

        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        let health = result.health.expect("проверки объявлены");
        assert_eq!(health.state, HealthState::FailedToRun);
        assert_eq!(health.all_passed(), Some(false));
        assert!(
            health.results[0].process_failed,
            "процесс не запустился — это process_failed"
        );
        assert!(matches!(
            result.state,
            ToolState::InstalledHealthUnknown { .. }
        ));
    }

    // ------------------------------------------------------------
    // Семантика обнаружения (контракт §4): вердикты по уликам
    // ------------------------------------------------------------

    /// Таблица вердиктов без запуска процессов: улика → Detected всегда;
    /// неконclusive без улик → Failed («неизвестно» ≠ «отсутствует»);
    /// пусто → NotDetected. Регрессионный страж критического бага
    /// «NotDetected в обеих ветках».
    #[test]
    fn detection_verdict_table() {
        let mk_install = || DetectedInstall {
            raw_version: String::new(),
            parsed_version: None,
            location: "C:\\x".to_string(),
            evidence: EvidenceKind::Footprint,
            reachable_via_path: false,
            path_scope: None,
            probe_log: None,
        };

        // Только таймауты пробы, улик нет → Failed с причиной.
        let mut timed_out = DetailedDetection::default();
        timed_out.inconclusive = Some("2 проб(ы) не уложились в таймаут 10с".to_string());
        assert_eq!(
            detection_verdict(&timed_out),
            DetectionOutcome::Failed {
                reason: "2 проб(ы) не уложились в таймаут 10с".to_string()
            }
        );

        // Ошибка запуска, улик нет → Failed.
        let mut launch_err = DetailedDetection::default();
        launch_err.inconclusive = Some("проба `x` не выполнилась: boom".to_string());
        assert!(matches!(
            detection_verdict(&launch_err),
            DetectionOutcome::Failed { .. }
        ));

        // Таймауты БЫЛИ, но улика найдена → Detected (улика сильнее).
        let mut evidence_after_timeout = DetailedDetection {
            inconclusive: Some("таймаут".to_string()),
            ..Default::default()
        };
        evidence_after_timeout.installs.push(mk_install());
        assert_eq!(
            detection_verdict(&evidence_after_timeout),
            DetectionOutcome::Detected
        );

        // Установка найдена → Detected, никогда NotDetected.
        let mut found = DetailedDetection::default();
        found.installs.push(mk_install());
        assert_eq!(detection_verdict(&found), DetectionOutcome::Detected);

        // Бинарь молчит в PATH → тоже улика → Detected.
        let mut silent = DetailedDetection::default();
        silent.silent_on_path = Some(SilentOnPath {
            reason: "молчит".to_string(),
            location: "C:\\x\\t.exe".to_string(),
            probe_log: None,
        });
        assert_eq!(detection_verdict(&silent), DetectionOutcome::Detected);

        // Ничего нет → чистое NotDetected.
        assert_eq!(
            detection_verdict(&DetailedDetection::default()),
            DetectionOutcome::NotDetected
        );
    }

    #[cfg(target_os = "windows")]
    fn unique_temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tc-detect-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Известный путь с рабочим бинарем: KnownPath-улика, Detected,
    /// версия разобрана из прямого запуска бинаря.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn known_path_binary_is_detected_with_known_path_evidence() {
        use std::io::Write as _;

        let dir = unique_temp_dir("ok");
        let bin = dir.join("respond.cmd");
        let mut f = std::fs::File::create(&bin).unwrap();
        f.write_all(b"@echo off\r\necho 7.7.7\r\n").unwrap();
        drop(f);

        let mut def = empty_rules_def("fake-knownpath-ok");
        def.detection.version_probes = vec![vec!["respond".to_string()]];
        def.detection.known_paths = vec![dir.to_string_lossy().into_owned()];

        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        assert!(
            matches!(result.detection, DetectionOutcome::Detected),
            "известный путь с рабочим бинарем обязан детектироваться: {:?}",
            result.detection
        );
        assert_eq!(result.installs.len(), 1);
        assert_eq!(result.installs[0].evidence, EvidenceKind::KnownPath);
        assert_eq!(result.installs[0].parsed_version.as_deref(), Some("7.7.7"));
        assert!(!result.installs[0].reachable_via_path, "каталог вне PATH");
        assert_eq!(
            result.state,
            ToolState::InstalledHealthy {
                version: "7.7.7".to_string()
            }
        );

        let _ = std::fs::remove_file(&bin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Бинарь есть в известном каталоге, но не отвечает на пробу:
    /// явная улика сломанного исполнения → PathBroken (и НЕ NotDetected).
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn known_path_broken_binary_is_path_broken_not_not_detected() {
        use std::io::Write as _;

        let dir = unique_temp_dir("broken");
        let bin = dir.join("failing.cmd");
        let mut f = std::fs::File::create(&bin).unwrap();
        f.write_all(b"@echo off\r\nexit /b 3\r\n").unwrap();
        drop(f);

        let mut def = empty_rules_def("fake-knownpath-broken");
        def.detection.version_probes = vec![vec!["failing".to_string()]];
        def.detection.known_paths = vec![dir.to_string_lossy().into_owned()];
        def.path_entries = vec![dir.join("bin").to_string_lossy().into_owned()];

        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        assert!(
            matches!(result.detection, DetectionOutcome::Detected),
            "бинарь найден — это обнаружение, а не отсутствие: {:?}",
            result.detection
        );
        match &result.state {
            ToolState::PathBroken { reason } => {
                assert!(
                    reason.contains("failing"),
                    "улика обязана указывать на бинарь: {reason}"
                );
            }
            other => panic!("ожидали PathBroken, получили {other:?}"),
        }
        // След установки сохранён для объяснения вердикта.
        assert_eq!(result.installs.len(), 1);
        assert!(result.installs[0].location.contains("broken"));
        assert!(result.error.is_none());

        let _ = std::fs::remove_file(&bin);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Исполняемый файл есть в PATH, но команда падает: PathBroken
    /// с сохранённым местом находки (robocopy всегда в System32,
    /// без аргументов завершается ненулевым кодом).
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn silent_on_path_command_failure_is_path_broken() {
        let mut def = empty_rules_def("fake-silent");
        def.detection.version_probes = vec![vec!["robocopy".to_string()]];

        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        assert!(matches!(result.detection, DetectionOutcome::Detected));
        match &result.state {
            ToolState::PathBroken { reason } => {
                assert!(reason.contains("PATH"), "причина: {reason}");
            }
            other => panic!("ожидали PathBroken, получили {other:?}"),
        }
        // Место находки сохранено в следе (объяснимость вердикта).
        assert_eq!(result.installs.len(), 1);
        assert!(
            result.installs[0].reachable_via_path,
            "бинарь найден именно через PATH"
        );
        assert!(!result.installs[0].location.is_empty());
    }

    /// Ключ реестра без рабочего бинаря: след установки НЕ является
    /// PathBroken — честное нейтральное состояние, улики сохранены.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn footprint_only_without_broken_evidence_is_not_path_broken() {
        const EXISTING_KEY: &str = "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion";
        let mut def = empty_rules_def("fake-footprint");
        def.detection.registry_keys = vec![EXISTING_KEY.to_string()];

        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        assert!(
            matches!(result.detection, DetectionOutcome::Detected),
            "след реестра — улика обнаружения: {:?}",
            result.detection
        );
        assert_ne!(
            result.state,
            ToolState::PathBroken {
                reason: "никогда".to_string()
            },
            "след без бинаря не имеет права быть PathBroken"
        );
        assert_ne!(result.state, ToolState::Missing, "след ≠ отсутствие");
        assert_eq!(
            result.state,
            ToolState::InstalledHealthUnknown {
                version: String::new()
            }
        );
        // Версия неизвестна честно; улика объясняет вердикт.
        assert_eq!(result.version_assessment, VersionAssessment::Unknown);
        assert_eq!(result.installs.len(), 1);
        assert_eq!(result.installs[0].evidence, EvidenceKind::Footprint);
        assert_eq!(result.installs[0].location, EXISTING_KEY);
        assert!(!result.installs[0].reachable_via_path);
    }

    /// Health-проверки выполняются при любых уликах установки независимо
    /// от разбора версии (контракт §4.3): у следа из реестра версии нет
    /// вообще, но проверки обязаны отработать.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn health_checks_run_for_footprint_independently_of_version() {
        let mut def = empty_rules_def("fake-footprint-health");
        def.detection.registry_keys =
            vec!["HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion".to_string()];
        def.health_checks = vec![HealthCheck {
            label: "echo ok".to_string(),
            command: vec![
                "cmd".to_string(),
                "/c".to_string(),
                "echo".to_string(),
                "ok".to_string(),
            ],
        }];

        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        let health = result.health.expect("улики есть — проверки должны идти");
        assert_eq!(health.state, HealthState::Healthy);
        assert_eq!(health.results.len(), 1, "проверка реально выполнялась");
        assert!(health.results[0].passed);
        assert!(!health.results[0].process_failed);
        assert_eq!(
            result.state,
            ToolState::InstalledHealthUnknown {
                version: String::new()
            }
        );
    }

    /// Текст manual_install не может перекричать найденный след:
    /// «ставьте вручную» ложно, когда установка уже на диске.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn manual_install_text_yields_to_footprint_evidence() {
        let mut def = empty_rules_def("fake-manual-footprint");
        def.manual_install = Some("Ставится только вручную".to_string());
        def.detection.registry_keys =
            vec!["HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion".to_string()];

        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        assert!(matches!(result.detection, DetectionOutcome::Detected));
        assert_ne!(
            result.state,
            ToolState::ManualInstall {
                reason: "Ставится только вручную".to_string()
            },
            "след найден — ручная инструкция была бы ложью"
        );
        assert!(matches!(
            result.state,
            ToolState::InstalledHealthUnknown { .. }
        ));
    }

    // ------------------------------------------------------------
    // Происхождение
    // ------------------------------------------------------------

    #[tokio::test]
    async fn bundled_tool_provenance_names_host() {
        let mut def = empty_rules_def("fake-bundled-prov");
        def.bundled_with = Some("node".to_string());
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "3.0.0".to_string(),
        ]];

        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        assert_eq!(
            result.provenance,
            Provenance::BundledWith {
                tool: "node".to_string()
            }
        );
    }

    #[tokio::test]
    async fn dual_tool_not_found_locally_has_docker_provenance() {
        let def = empty_rules_def("fake-dual-prov");

        let ctx = ScanContext {
            process_entries: vec![],
            persisted_entries: vec![],
            managed_tools: Default::default(),
            dual_tools: ["fake-dual-prov".to_string()].into_iter().collect(),
            os_name: "windows".to_string(),
        };
        let result = scan_tool(&def, &ctx).await;

        assert_eq!(result.state, ToolState::DockerManaged);
        assert_eq!(result.provenance, Provenance::Docker);
    }

    fn scan_result_stub(state: ToolState) -> ToolScanResult {
        ToolScanResult {
            tool_id: "x".to_string(),
            display: "X".to_string(),
            category: "utility".to_string(),
            icon: None,
            detection: DetectionOutcome::NotDetected,
            installs: vec![],
            path_findings: vec![],
            health: None,
            applicability: PlatformApplicability::Installable,
            capabilities: ToolPlatformCapabilities::default(),
            provenance: Provenance::Unknown,
            bundled_with: None,
            version_assessment: VersionAssessment::Unknown,
            state,
            error: None,
            canonical_install: None,
            version_selected_because: String::new(),
            duration_ms: 0,
        }
    }

    #[test]
    fn rules_validation_helper() {
        let def = fake_def("node");
        assert!(rules_have_any_evidence(&def.detection));
        assert!(!rules_have_any_evidence(&DetectionRules::default()));
    }

    // ------------------------------------------------------------
    // Фаза 2: честность версий/PATH/логов
    // ------------------------------------------------------------

    /// `java -version` и `kotlinc -version` пишут версию в STDERR:
    /// пустой stdout при коде 0 — это ОТВЕТ, а не «сломанный PATH».
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn stderr_version_output_is_detected_not_path_broken() {
        let mut def = empty_rules_def("fake-stderr-version");
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "21.0.2".to_string(),
            "1>&2".to_string(),
        ]];
        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        assert!(
            matches!(result.detection, DetectionOutcome::Detected),
            "ответ в stderr при коде 0 — обнаружение: {:?}",
            result.detection
        );
        assert!(
            !matches!(result.state, ToolState::PathBroken { .. }),
            "пустой stdout не делает PATH сломанным"
        );
        assert_eq!(result.installs[0].parsed_version.as_deref(), Some("21.0.2"));
    }

    /// Алиасные пробы одной установки (python/python3 → один бинарь)
    /// сливаются в ОДНУ улику; разные версии/места сохраняются все.
    #[test]
    fn duplicate_alias_installs_are_deduped() {
        let mk = |location: &str, v: Option<&str>| DetectedInstall {
            raw_version: v.unwrap_or_default().to_string(),
            parsed_version: v.map(str::to_string),
            location: location.to_string(),
            evidence: EvidenceKind::VersionProbe,
            reachable_via_path: true,
            path_scope: None,
            probe_log: None,
        };

        // Тот же каталог (с точностью до регистра/слешей) + та же версия.
        let merged = dedupe_installs(vec![
            mk("C:\\Python312\\python.exe", Some("3.12.1")),
            mk("c:/python312/PYTHON.exe", Some("3.12.1")),
        ]);
        assert_eq!(merged.len(), 1, "алиас одной установки — дубликат");

        // Разные версии в одном месте или разные места — разные установки.
        let kept = dedupe_installs(vec![
            mk("C:\\Python312\\python.exe", Some("3.12.1")),
            mk("C:\\Python313\\python.exe", Some("3.13.0")),
        ]);
        assert_eq!(kept.len(), 2, "разные места — разные установки");
    }

    /// Каноническая установка: наибольшая разобранная версия; объяснение
    /// обязательно; без разобранных версий — первая ответившая проба.
    #[test]
    fn canonical_selection_prefers_highest_version_and_explains() {
        let mk = |v: &str| DetectedInstall {
            raw_version: format!("tool {v}"),
            parsed_version: Some(v.to_string()),
            location: format!("C:\\tools\\{v}\\bin"),
            evidence: EvidenceKind::KnownPath,
            reachable_via_path: false,
            path_scope: None,
            probe_log: None,
        };

        let (index, reason) = select_canonical_install(&[mk("3.10.0"), mk("3.13.1"), mk("3.11.9")])
            .expect("есть разобранные версии");
        assert_eq!(index, 1, "наибольшая версия — каноническая");
        assert!(
            reason.contains("наибольш") && reason.contains("2"),
            "объяснение называет причину и число установок: {reason}"
        );

        // Без разбора версии — сырой вывод первой ответившей пробы,
        // объяснение помечает «непарсировано».
        let raw_only = [DetectedInstall {
            raw_version: "build 42".to_string(),
            parsed_version: None,
            location: String::new(),
            evidence: EvidenceKind::VersionProbe,
            reachable_via_path: true,
            path_scope: None,
            probe_log: None,
        }];
        let (_, reason) = select_canonical_install(&raw_only).unwrap();
        assert!(reason.contains("сырой"), "честно про непарсинг: {reason}");

        assert_eq!(select_canonical_install(&[]), None);
    }

    /// Классификация PATH по слоям: процесс ≠ постоянный ≠ вне PATH.
    #[test]
    fn path_scope_distinguishes_process_persisted_and_outside() {
        #[cfg(target_os = "windows")]
        let (on_process, persisted_only, outside) = (
            "C:\\Tools\\Good\\bin\\t.exe",
            "C:\\Tools\\Persisted\\bin\\t.exe",
            "C:\\Tools\\Hidden\\bin\\t.exe",
        );
        #[cfg(not(target_os = "windows"))]
        let (on_process, persisted_only, outside) = (
            "/opt/good/bin/t",
            "/opt/persisted/bin/t",
            "/opt/hidden/bin/t",
        );
        let process = vec![parent_of(on_process)];
        let persisted = vec![parent_of(persisted_only)];

        use super::super::models::PathScope;
        assert_eq!(
            classify_path_scope(on_process, &process, &persisted),
            PathScope::ProcessPath
        );
        assert_eq!(
            classify_path_scope(persisted_only, &process, &persisted),
            PathScope::PersistedPathOnly,
            "в постоянном PATH, но не в процессе — отдельный класс"
        );
        assert_eq!(
            classify_path_scope(outside, &process, &persisted),
            PathScope::OutsidePath
        );
    }

    #[cfg(target_os = "windows")]
    fn parent_of(path: &str) -> String {
        std::path::Path::new(path)
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }
    #[cfg(not(target_os = "windows"))]
    fn parent_of(path: &str) -> String {
        std::path::Path::new(path)
            .parent()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    /// Каталог: отсутствие pip НЕ делает Python нездоровым — pip это
    /// ОТДЕЛЬНЫЙ инструмент каталога со своей python-пробой; проверки
    /// здоровья Python не содержат pip-утверждения.
    #[test]
    fn python_health_checks_do_not_include_pip_assertion() {
        let python = fake_def("python");
        assert!(
            !python.health_checks.is_empty(),
            "у Python есть собственные проверки (venv)"
        );
        for check in &python.health_checks {
            assert!(
                !check.label.to_lowercase().contains("pip"),
                "pip-проверка не может ронять здоровье Python: {}",
                check.label
            );
        }
        // pip — самостоятельный тул с ПРАВИЛЬНОЙ python-пробой.
        let pip = fake_def("pip");
        let joined: Vec<String> = pip
            .detection
            .version_probes
            .iter()
            .map(|p| p.join(" "))
            .collect();
        assert!(
            joined.iter().any(|p| p == "python -m pip --version"),
            "python-проба pip обязана быть в каталоге: {joined:?}"
        );
    }

    /// Каталог: у docker СЛОИ здоровья разделены (cli ≠ daemon) —
    /// «docker --version» сам по себе не является проверкой daemon'а.
    #[test]
    fn docker_health_checks_cover_cli_and_daemon_layers_separately() {
        let docker = fake_def("docker");
        let labels: Vec<&str> = docker
            .health_checks
            .iter()
            .map(|c| c.label.as_str())
            .collect();
        assert!(
            labels.iter().any(|l| l.contains("cli")),
            "должен быть отдельный слой CLI: {labels:?}"
        );
        assert!(
            labels.iter().any(|l| l.contains("daemon")),
            "должен быть отдельный слой daemon/API: {labels:?}"
        );
        let cli = docker
            .health_checks
            .iter()
            .find(|c| c.label.contains("cli"))
            .unwrap();
        assert_ne!(
            cli.command,
            docker
                .health_checks
                .iter()
                .find(|c| c.label.contains("daemon"))
                .unwrap()
                .command,
            "слои обязаны проверяться разными командами"
        );
    }

    /// Результат проверки здоровья хранит ПОЛНЫЙ лог: команду, код выхода,
    /// потоки вывода — по нему UI показывает всё без усечения в единственной
    /// копии.
    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn health_result_retains_command_identity_exit_code_and_streams() {
        let mut def = empty_rules_def("fake-health-log");
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "1".to_string(),
        ]];
        def.health_checks = vec![HealthCheck {
            label: "exit code visible".to_string(),
            command: vec![
                "cmd".to_string(),
                "/c".to_string(),
                "echo".to_string(),
                "out-line".to_string(),
                "&".to_string(),
                "echo".to_string(),
                "err-line".to_string(),
                "1>&2".to_string(),
                "&".to_string(),
                "exit".to_string(),
                "/b".to_string(),
                "5".to_string(),
            ],
        }];

        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;
        let health = result.health.expect("проверка объявлена");
        let check = &health.results[0];

        assert_eq!(check.command.first().map(String::as_str), Some("cmd"));
        assert_eq!(check.exit_code, Some(5), "код выхода сохранён");
        assert!(check.stdout.contains("out-line"), "stdout полный");
        assert!(check.stderr.contains("err-line"), "stderr полный");
        assert!(!check.timed_out);
    }
}
