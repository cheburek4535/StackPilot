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
    PlatformApplicability, Provenance, ToolPlatformCapabilities, ToolScanResult, ToolState,
    VersionAssessment,
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
                    name.starts_with(&prefix) && name.ends_with(&suffix) && p.is_dir()
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

/// Результат детального опроса одного инструмента.
#[derive(Debug, Default)]
pub struct DetailedDetection {
    /// Все найденные установки/следы.
    pub installs: Vec<DetectedInstall>,
    /// Some(причина): опрос был неконclusive (таймауты/ошибки запуска),
    /// положительных улик нет → ScanFailed, а НЕ Missing.
    pub inconclusive: Option<String>,
    /// Бинарь есть в PATH, но молчит (PathBroken-улика).
    pub silent_on_path: Option<String>,
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

    // --- 1. Пробы версии через PATH ---
    for probe_cmd in &def.detection.version_probes {
        if probe_cmd.is_empty() {
            continue;
        }
        let out = probe::run_probe(&probe_cmd[0], &probe_cmd[1..], probe::PROBE_TIMEOUT).await;
        if out.timed_out {
            timeouts += 1;
            continue;
        }
        if let Some(err) = out.launch_error {
            launch_errors += 1;
            result.inconclusive = Some(format!(
                "проба `{}` не выполнилась: {err}",
                probe_cmd.join(" ")
            ));
            continue;
        }
        if out.success && !out.stdout.trim().is_empty() {
            let location = which_location(&probe_cmd[0]);
            let install =
                versioned_install(&out.stdout, location, EvidenceKind::VersionProbe, true);
            result.installs.push(install);
            break; // первая ответившая проба достаточна (паритет с discovery)
        }
        // Ответил ненулевым кодом/пусто — пробуем следующую пробу.
    }

    // Бинарь есть в PATH, но ни одна проба не ответила → PathBroken-улика.
    if result.installs.is_empty() {
        for probe_cmd in &def.detection.version_probes {
            if probe_cmd.is_empty() || is_shell_probe(&probe_cmd[0]) {
                continue;
            }
            if which::which(&probe_cmd[0]).is_ok() {
                result.silent_on_path = Some(format!(
                    "{} найден в PATH, но не отвечает на `{}`",
                    probe_cmd[0],
                    probe_cmd.join(" ")
                ));
                break;
            }
        }
    }

    // --- 2. Известные пути (все совпадения → несколько установок) ---
    for known in &def.detection.known_paths {
        for dir in glob_all(&expand_env(known)) {
            if let Some(install) = probe_known_dir(def, &dir, process_entries).await {
                result.installs.push(install);
            }
        }
    }

    // --- 3. Реестр (Windows; на других ОС reg отсутствует — чистый промах) ---
    if result.installs.is_empty() {
        for key in &def.detection.registry_keys {
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

/// Проба бинаря внутри известного каталога установки.
async fn probe_known_dir(
    def: &ToolDefinition,
    dir: &Path,
    process_entries: &[String],
) -> Option<DetectedInstall> {
    let exts: &[&str] = if cfg!(target_os = "windows") {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    let on_path = dir_on_path(dir, process_entries);

    for probe_cmd in &def.detection.version_probes {
        if probe_cmd.is_empty() {
            continue;
        }
        for ext in exts {
            let bin = dir.join(format!("{}{}", probe_cmd[0], ext));
            if !bin.is_file() {
                continue;
            }
            let out = probe::run_probe(
                &bin.to_string_lossy(),
                &probe_cmd[1..],
                probe::PROBE_TIMEOUT,
            )
            .await;
            if out.timed_out || out.launch_error.is_some() {
                continue;
            }
            if out.success && !out.stdout.trim().is_empty() {
                return Some(versioned_install(
                    &out.stdout,
                    bin.to_string_lossy().into_owned(),
                    EvidenceKind::KnownPath,
                    on_path,
                ));
            }
        }
    }

    // Каталог существует, но бинарь молчит — след установки без рабочей пробы.
    Some(DetectedInstall {
        raw_version: String::new(),
        parsed_version: None,
        location: dir.to_string_lossy().into_owned(),
        evidence: EvidenceKind::Footprint,
        reachable_via_path: on_path,
    })
}

/// Собирает DetectedInstall из сырого вывода пробы (версия парсится
/// устойчиво к мусору; непарсируемый вывод — всё равно улика установки).
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
        },
        None => DetectedInstall {
            raw_version: first_line,
            parsed_version: None,
            location,
            evidence,
            reachable_via_path,
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
/// Вердикт: все прошли → Healthy; процесс-отказ хотя бы одной проверки
/// (запуск/таймаут) → FailedToRun («не смогли проверить»); иначе —
/// Unhealthy. Деградация (Healthy при версии ниже рекомендуемой)
/// уточняется вызывающей стороной, которая знает оценку версии.
pub async fn run_health_checks(def: &ToolDefinition) -> HealthOutcome {
    if def.health_checks.is_empty() {
        return HealthOutcome {
            state: HealthState::NoChecksDefined,
            results: Vec::new(),
        };
    }

    let mut results = Vec::with_capacity(def.health_checks.len());
    for check in &def.health_checks {
        results.push(run_one_health_check(check).await);
    }

    let any_process_failure = results.iter().any(|r| r.process_failed);
    let all_passed = results.iter().all(|r| r.passed);
    let state = if all_passed {
        HealthState::Healthy
    } else if any_process_failure {
        HealthState::FailedToRun
    } else {
        HealthState::Unhealthy
    };

    HealthOutcome { state, results }
}

async fn run_one_health_check(check: &HealthCheck) -> HealthCheckResult {
    let started = Instant::now();
    let Some((program, args)) = check.command.split_first() else {
        return HealthCheckResult {
            label: check.label.clone(),
            passed: false,
            process_failed: true,
            detail: "команда пустая (ошибка в tools.json)".to_string(),
            duration_ms: 0,
        };
    };

    let out = probe::run_probe(program, args, HEALTH_CHECK_TIMEOUT).await;
    let duration_ms = started.elapsed().as_millis() as u64;

    if out.timed_out {
        return HealthCheckResult {
            label: check.label.clone(),
            passed: false,
            process_failed: true,
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
            passed: false,
            process_failed: true,
            detail: format!("не удалось запустить `{}`: {err}", check.command.join(" ")),
            duration_ms,
        };
    }
    if out.not_found {
        return HealthCheckResult {
            label: check.label.clone(),
            passed: false,
            process_failed: true,
            detail: format!("команда не найдена: {}", check.command.join(" ")),
            duration_ms,
        };
    }

    // Процесс выполнился: код выхода — утверждение из каталога.
    let detail_source = if out.stdout.is_empty() {
        out.stderr.clone()
    } else {
        out.stdout.clone()
    };
    let detail = if detail_source.is_empty() {
        format!("код выхода {}", out.exit_code.unwrap_or(0))
    } else {
        detail_source
    };

    HealthCheckResult {
        label: check.label.clone(),
        passed: out.success,
        process_failed: false,
        detail,
        duration_ms,
    }
}

// ------------------------------------------------------------
// Классификация и композиция состояния
// ------------------------------------------------------------

/// Применимость инструмента к платформе (чистая функция от данных каталога).
pub fn classify_applicability(
    def: &ToolDefinition,
    os_name: &str,
    is_dual_tool: bool,
) -> PlatformApplicability {
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
/// (правила контракта §3; нормализация manual-тулов — паритет check.rs).
pub fn compose_state(
    def: &ToolDefinition,
    detection: &DetectionOutcome,
    installs: &[DetectedInstall],
    health: Option<&HealthOutcome>,
    applicability: PlatformApplicability,
    assessment: VersionAssessment,
) -> ToolState {
    // Ошибка опроса важнее всего остального: неизвестно ≠ missing.
    if let DetectionOutcome::Failed { reason } = detection {
        return ToolState::ScanFailed {
            reason: reason.clone(),
        };
    }

    let working_install = installs.iter().find(|i| !i.raw_version.is_empty());
    let has_footprint_only = working_install.is_none() && !installs.is_empty();

    if working_install.is_none() {
        // Ничего работающего не нашли.
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
            PlatformApplicability::BuiltIn => ToolState::BuiltInSystem,
            _ => ToolState::Missing,
        };
    }

    // След без рабочей пробы: установка есть, бинарь не отвечает.
    if has_footprint_only {
        let footprint = installs[0].location.clone();
        let reason = format!("Установка найдена ({footprint}), но бинарник не отвечает на пробу");
        return ToolState::PathBroken { reason };
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

    if assessment == VersionAssessment::BelowMin {
        // Ниже минимума — это деградация с явной пометкой версии.
        return ToolState::UpdateAvailable {
            installed: version,
            recommended: def
                .versions
                .recommended
                .clone()
                .unwrap_or_else(|| "?".into()),
        };
    }
    if assessment == VersionAssessment::BelowRecommended {
        return ToolState::UpdateAvailable {
            installed: version,
            recommended: def
                .versions
                .recommended
                .clone()
                .unwrap_or_else(|| "?".into()),
        };
    }
    if health_failed {
        return ToolState::InstalledUnhealthy { version };
    }
    match health_state {
        Some(HealthState::Healthy) | Some(HealthState::Degraded) => {
            ToolState::InstalledHealthy { version }
        }
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
    /// Инструменты, записанные в state.json (StackPilot-managed).
    pub managed_tools: std::collections::HashSet<String>,
    /// Имена «двойных» docker-инструментов каталога.
    pub dual_tools: std::collections::HashSet<String>,
    /// Имя ОС (платформенный слой).
    pub os_name: String,
}

/// Полное сканирование одного инструмента: обнаружение + здоровье +
/// PATH-находки + композиция состояния. Только чтение машины.
pub async fn scan_tool(def: &ToolDefinition, ctx: &ScanContext) -> ToolScanResult {
    let started = Instant::now();

    let detailed = detect_detailed(def, &ctx.process_entries).await;

    // Детект-вердикт: Failed только если неконclusive и улик нет вообще.
    let detection = if let Some(reason) = &detailed.inconclusive {
        DetectionOutcome::Failed {
            reason: reason.clone(),
        }
    } else if detailed.installs.is_empty() && detailed.silent_on_path.is_none() {
        DetectionOutcome::NotDetected
    } else {
        DetectionOutcome::NotDetected // улики в installs/silent_on_path
    };

    // Здоровье: считаем только когда инструмент реально отвечает пробой.
    let responds = detailed.installs.iter().any(|i| !i.raw_version.is_empty());
    let mut health = if responds {
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

    // Silent-on-path → PathBroken-след (без рабочей пробы).
    let mut installs = detailed.installs;
    if let Some(reason) = &detailed.silent_on_path {
        if installs.is_empty() {
            installs.push(DetectedInstall {
                raw_version: String::new(),
                parsed_version: None,
                location: String::new(),
                evidence: EvidenceKind::Footprint,
                reachable_via_path: false,
            });
        }
        let _ = reason; // текст попадёт в state через compose_state ниже
    }

    let state = if let Some(reason) = &detailed.silent_on_path {
        if responds {
            compose_state(
                def,
                &detection,
                &installs,
                health.as_ref(),
                applicability,
                assessment,
            )
        } else {
            ToolState::PathBroken {
                reason: reason.clone(),
            }
        }
    } else {
        compose_state(
            def,
            &detection,
            &installs,
            health.as_ref(),
            applicability,
            assessment,
        )
    };

    // PATH-находки имеют смысл только для реально найденных установок.
    let path_findings = if responds {
        path_report::tool_path_findings(def, &installs, &ctx.process_entries)
    } else {
        Vec::new()
    };

    // Происхождение: только факты, без догадок.
    //   - запись в state.json → ставили мы;
    //   - встроенный в ОС → System;
    //   - найден и идёт в комплекте с другим (npm→node) → BundledWith;
    //   - двойной docker-инструмент, локально не найден → Docker
    //     (живёт в docker-compose проекта);
    //   - найден сам по себе → External; не найден → Unknown.
    // PackageManager здесь НЕ угадывается: надёжного «кто ставил» для
    // winget/apt на этапе скана нет — это остаётся Unknown/External.
    let provenance = if ctx.managed_tools.contains(&def.id) {
        Provenance::StackPilotManaged
    } else if matches!(state, ToolState::BuiltInSystem) {
        Provenance::System
    } else if responds && def.bundled_with.is_some() {
        Provenance::BundledWith {
            tool: def.bundled_with.clone().unwrap_or_default(),
        }
    } else if !responds && applicability == PlatformApplicability::DockerDefault {
        Provenance::Docker
    } else if responds {
        Provenance::External
    } else {
        Provenance::Unknown
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
        error: None,
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
#[cfg_attr(not(test), allow(dead_code))]
pub fn to_legacy_status(result: &ToolScanResult) -> ToolStatus {
    match &result.state {
        ToolState::Missing | ToolState::UnsupportedPlatform | ToolState::InstallUnavailable => {
            ToolStatus::Missing
        }
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
            managed_tools: Default::default(),
            dual_tools: Default::default(),
            os_name: "windows".to_string(),
        };
        let result = scan_tool(&def, &ctx).await;
        assert!(matches!(result.detection, DetectionOutcome::NotDetected));
        assert_eq!(result.state, ToolState::Missing);
        assert_eq!(result.applicability, PlatformApplicability::Installable);
    }

    #[tokio::test]
    async fn echo_probe_def_is_detected_healthy_unknown() {
        // Проба cmd /c echo отвечает → установлен; проверок здоровья нет →
        // InstalledHealthUnknown («не знаем»), а не unhealthy.
        let mut def = empty_rules_def("fake-echo");
        def.detection.version_probes = vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "5.5.5".to_string(),
        ]];
        let ctx = ScanContext::default();
        let result = scan_tool(&def, &ctx).await;

        assert_eq!(result.installs.len(), 1);
        assert_eq!(result.installs[0].parsed_version.as_deref(), Some("5.5.5"));
        assert_eq!(
            result.state,
            ToolState::InstalledHealthUnknown {
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

        assert_eq!(result.installs.len(), 1);
        assert!(result.installs[0].parsed_version.is_none());
        assert_eq!(result.version_assessment, VersionAssessment::Unparseable);
        assert!(matches!(
            result.state,
            ToolState::InstalledHealthUnknown { .. }
        ));
    }

    #[tokio::test]
    async fn no_checks_defined_is_not_unhealthy_and_excluded_from_score() {
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
        assert!(matches!(
            result.state,
            ToolState::InstalledHealthUnknown { .. }
        ));
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

        let ctx = ScanContext::default();
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
            duration_ms: 0,
        }
    }

    #[test]
    fn rules_validation_helper() {
        let def = fake_def("node");
        assert!(rules_have_any_evidence(&def.detection));
        assert!(!rules_have_any_evidence(&DetectionRules::default()));
    }
}
