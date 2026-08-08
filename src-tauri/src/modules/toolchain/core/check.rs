// ============================================================
// Сборка отчёта проверки окружения (check.rs)
// ============================================================
// Проходит по запрошенным id инструментов, детектит каждый
// (discovery.rs) и собирает EnvironmentCheck — ответ команды
// tc_check_environment для экрана «Environment» в мастере.
//
// Правила «что попадает в отчёт»:
//   - неизвестный id (нет в tools.json) — пропускаем с логом;
//   - Missing у тула, который нельзя поставить на этой ОС
//     (msvc-build-tools на Linux) — не требование;
//   - Missing у тула, который идёт в комплекте с другим
//     (npm bundled_with node) — не требование;
//   - всё остальное — требование со статусом.
//
// free_space_mb на этапе 2 всегда 0 (проверка диска — этап 5):
// 0 означает «свободное место не проверялось» → enough_space=true.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;
use tokio::time::timeout;
use crate::modules::toolchain::models::*;
use crate::modules::toolchain::platforms;

use super::discovery;

/// Предельное время всей проверки окружения. Каждая проба внутри
/// detect_tool уже ограничена (PROBE_TIMEOUT_SECS), но страховка на
/// случай экзотики (антивирус, сетевые диски, зависшие процессы):
/// после дедлайна незавершённые проверки отменяются, отчёт отдаётся
/// по уже собранным данным — проверка НИКОГДА не висит бесконечно.
const CHECK_DEADLINE: Duration = Duration::from_secs(90);

/// Callback прогресса: вызывается после проверки каждого инструмента
/// (см. CheckProgressEvent). Ядро не знает про Tauri — команда оборачивает
/// callback в app.emit, тесты собирают события в вектор.
pub type ProgressFn = Arc<dyn Fn(CheckProgressEvent) + Send + Sync>;

/// Полная проверка окружения под требования проекта.
/// `requested` — упорядоченный список id (из requirements::resolve),
/// `free_space_mb` — свободное место на диске (0 = не проверялось).
pub async fn run_check(
    definitions: &[ToolDefinition],
    requested: &[String],
    free_space_mb: u64,
    on_progress: Option<ProgressFn>,
) -> EnvironmentCheck {
    let os = platforms::current_platform().os_name();

    let mut set = JoinSet::new();

    let mut total_size_mb: u64 = 0;
    let mut needs_admin_any = false;

    for (index, id) in requested.iter().enumerate() {
        let Some(def) = definitions.iter().find(|d| &d.id == id) else {
            eprintln!("[toolchain] неизвестный инструмент: {id}");
            continue;
        };
        let def_clone = def.clone();

        set.spawn(async move {
            let status = discovery::detect_tool(&def_clone).await;
            (def_clone, status, index)
        });
    }

    let total = requested.len();
    let mut temp_requirements: Vec<(usize, ToolRequirement)> = Vec::new();
    let mut done = 0usize;

    // Вся сборка ограничена дедлайном: зависшие пробы не могут
    // заморозить шаг Environment в мастере.
    let collect = async {
        while let Some(res) = set.join_next().await {
            let Ok((def, status, index)) = res else {
                eprintln!("[toolchain] задача обнаружения прервана: {res:?}");
                continue;
            };
            done += 1;
            if let Some(cb) = &on_progress {
                cb(CheckProgressEvent {
                    done,
                    total,
                    tool_id: def.id.clone(),
                    display: def.display.clone(),
                    status: status.clone(),
                });
            }

            // Тул не устанавливается на этой ОС — при отсутствии не требование.
            if matches!(status, ToolStatus::Missing) && !def.installable() {
                continue;
            }
            // Тул входит в комплект другого (npm приходит с node) — не требование.
            if matches!(status, ToolStatus::Missing) && def.bundled_with.is_some() {
                continue;
            }

            // Скачивать нужно только то, чего нет или что устарело.
            if !status.is_ok() {
                total_size_mb += def.size_mb as u64;
                if def.needs_admin {
                    needs_admin_any = true;
                }
            }

            let desc = source_description(&def);
            temp_requirements.push((index, ToolRequirement {
                tool_id: def.id,
                display: def.display,
                category: def.category,
                status,
                size_mb: def.size_mb,
                needs_admin: def.needs_admin,
                source_description: desc,
            }));
        }
    };

    if let Err(_) = timeout(CHECK_DEADLINE, collect).await {
        eprintln!(
            "[toolchain] проверка окружения превысила лимит {}с, отдаю частичный отчёт ({done}/{} инструментов)",
            CHECK_DEADLINE.as_secs(),
            total
        );
        set.abort_all();
    }

    temp_requirements.sort_by_key(|pair| pair.0);

    
    let requirements: Vec<ToolRequirement> = temp_requirements.into_iter().map(|pair| pair.1).collect();
    let all_ready = requirements.iter().all(|r| r.status.is_ok());
    let enough_space = free_space_mb == 0 || free_space_mb >= total_size_mb;

    EnvironmentCheck {
        os,
        requirements,
        total_size_mb,
        free_space_mb,
        enough_space,
        needs_admin_any,
        all_ready,
    }
}

/// Человекочитаемое описание способа установки на текущей ОС
/// (берётся первый источник в порядке приоритета из tools.json).
fn source_description(def: &ToolDefinition) -> String {
    if !def.installable() {
        return "Отдельная установка не требуется".to_string();
    }

    let os = platforms::current_platform().os_name();
    let sources: &[InstallSource] = match os.as_str() {
        "windows" => &def.sources.windows,
        "linux" => &def.sources.linux,
        "macos" => &def.sources.macos,
        _ => &[],
    };

    let Some(first) = sources.first() else {
        return "Установка на этой ОС не предусмотрена".to_string();
    };

    match first.kind {
        InstallSourceKind::PkgManager => format!("Менеджер пакетов: {}", first.id),
        InstallSourceKind::Official => format!("Официальный установщик: {}", first.id),
        InstallSourceKind::Script => format!("Скрипт установки: {}", first.id),
    }
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Фейковое определение: гарантированно отсутствующий бинарник,
    /// один источник в windows. Используется в детерминированных тестах
    /// (в отличие от реального tools.json, где всё зависит от машины).
    fn fake_def(id: &str) -> ToolDefinition {
        ToolDefinition {
            id: id.to_string(),
            category: "utility".to_string(),
            display: "Fake Tool".to_string(),
            description: "тестовый инструмент".to_string(),
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
                    id: "Fake.Fake".to_string(),
                    url: None,
                    args: vec![],
                    extra_args: vec![],
                    dynamic_args: false,
                    install_dir: None,
                    needs_admin: None,
                }],
                linux: vec![],
                macos: vec![],
            },
            size_mb: 10,
            needs_admin: true,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
        }
    }

    #[tokio::test]
    async fn unknown_tool_id_is_ignored() {
        let check = run_check(&[], &["no-such-tool".to_string()], 0, None).await;
        assert!(check.requirements.is_empty());
        assert!(check.all_ready);
    }

    #[tokio::test]
    async fn missing_installable_tool_becomes_requirement() {
        let defs = vec![fake_def("fake-tool")];
        let check = run_check(&defs, &["fake-tool".to_string()], 0, None).await;

        assert_eq!(check.requirements.len(), 1);
        let req = &check.requirements[0];
        assert!(matches!(req.status, ToolStatus::Missing));
        assert_eq!(req.size_mb, 10);
        assert!(req.needs_admin);
        assert_eq!(req.source_description, "Менеджер пакетов: Fake.Fake");

        assert_eq!(check.total_size_mb, 10);
        assert!(check.needs_admin_any);
        assert!(!check.all_ready);
        assert!(check.enough_space); // free_space_mb = 0 → «не проверялось»
    }

    #[tokio::test]
    async fn missing_non_installable_tool_is_skipped() {
        // Определение без источников установки: sqlite на Linux
        // не ставится менеджером — его отсутствие не блокирует проект.
        let mut def = fake_def("fake-info");
        def.sources = InstallSources::default();

        let check = run_check(&[def], &["fake-info".to_string()], 0, None).await;
        assert!(check.requirements.is_empty());
        assert!(check.all_ready);
    }

    #[tokio::test]
    async fn missing_bundled_tool_is_skipped() {
        let mut def = fake_def("fake-bundled");
        def.bundled_with = Some("fake-host".to_string());

        let check = run_check(&[def], &["fake-bundled".to_string()], 0, None).await;
        assert!(check.requirements.is_empty());
        assert!(check.all_ready);
    }

    #[tokio::test]
    async fn free_space_is_compared_when_known() {
        let defs = vec![fake_def("fake-tool")];
        // 5 МБ свободно, нужно 10 → мало места
        let check = run_check(&defs, &["fake-tool".to_string()], 5, None).await;
        assert!(!check.enough_space);
        assert_eq!(check.free_space_mb, 5);

        // 20 МБ свободно, нужно 10 → хватает
        let check = run_check(&defs, &["fake-tool".to_string()], 20, None).await;
        assert!(check.enough_space);
    }

    #[tokio::test]
    async fn progress_events_reported_per_tool() {
        let defs = vec![fake_def("fake-tool"), fake_def("fake-tool-2")];
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let events_cb = Arc::clone(&events);
        let cb: ProgressFn = Arc::new(move |ev| {
            events_cb.lock().unwrap().push((ev.tool_id, ev.done, ev.total));
        });

        let check = run_check(
            &defs,
            &["fake-tool".to_string(), "fake-tool-2".to_string()],
            0,
            Some(cb),
        )
        .await;
        let evs = events.lock().unwrap();
        assert_eq!(evs.len(), 2, "по одному событию на инструмент: {evs:?}");
        assert_eq!(evs[0], ("fake-tool".to_string(), 1, 2));
        assert_eq!(evs[1], ("fake-tool-2".to_string(), 2, 2));
        // финальный отчёт совпадает с последним прогрессом
        assert_eq!(check.requirements.len(), 2);
    }
}
