// ============================================================
// Проверка здоровья инструментов (health.rs) — этап 6
// ============================================================
// discovery отвечает «установлен ли инструмент» (проба версии,
// пути, реестр). health отвечает на следующий вопрос: «работает
// ли он на самом деле» — прогоняет health_checks из tools.json
// (node → npm/npx, docker → живой ли daemon и т.п.).
//
// Семантика ToolHealth.state (явные состояния, не выводимые из
// пустоты списка проверок):
//   - Missing/PathBroken            → Unavailable (ok=false);
//   - установлен, проверок нет      → NotChecked («не знаем», ok=false);
//   - установлен, все проверки ок   → Healthy (ok=true);
//   - установлен, что-то упало      → Failed (ok=false).
//
// Запуск команд — через discovery::run_capture (таймаут и обработка
// ошибок уже внутри): единственное место, где процессы запускаются.

use crate::modules::toolchain::models::*;

use super::discovery;

/// Полная проверка здоровья одного инструмента.
pub async fn check_tool(def: &ToolDefinition, status: &ToolStatus) -> ToolHealth {
    // Тул не работает — health_checks не помогут.
    if matches!(status, ToolStatus::Missing | ToolStatus::PathBroken { .. }) {
        return ToolHealth {
            state: HealthState::Unavailable,
            ok: false,
            tool_id: def.id.clone(),
            display: def.display.clone(),
            icon: def.icon.clone(),
            checks: vec![HealthCheckResult {
                label: "инструмент".to_string(),
                ok: false,
                detail: "не установлен / сломан путь".to_string(),
            }],
        };
    }

    // Проверок нет — состояние «не проверяли». Это НЕ «нездоров»:
    // отсутствие данных не должно выглядеть как отрицательный вердикт.
    if def.health_checks.is_empty() {
        return ToolHealth {
            state: HealthState::NotChecked,
            ok: false,
            tool_id: def.id.clone(),
            display: def.display.clone(),
            icon: def.icon.clone(),
            checks: Vec::new(),
        };
    }

    let mut checks = Vec::new();
    let mut all_ok = true;

    for check in &def.health_checks {
        let Some((program, args)) = check.command.split_first() else {
            // Пустой command — ошибка в tools.json, но падать нельзя.
            all_ok = false;
            checks.push(HealthCheckResult {
                label: check.label.clone(),
                ok: false,
                detail: "команда пустая (ошибка в tools.json)".to_string(),
            });
            continue;
        };

        match discovery::run_capture(program, args).await {
            Some(out) => checks.push(HealthCheckResult {
                label: check.label.clone(),
                ok: true,
                detail: out,
            }),
            None => {
                all_ok = false;
                checks.push(HealthCheckResult {
                    label: check.label.clone(),
                    ok: false,
                    detail: format!("команда не выполнилась: {}", check.command.join(" ")),
                });
            }
        }
    }

    ToolHealth {
        state: if all_ok {
            HealthState::Healthy
        } else {
            HealthState::Failed
        },
        ok: all_ok,
        tool_id: def.id.clone(),
        display: def.display.clone(),
        icon: def.icon.clone(),
        checks,
    }
}

/// Полный health-отчёт по окружению: все инструменты каталога.
/// score — процент здоровых СРЕДИ инструментов с health_checks
/// (тул без проверок не штрафует оценку — его просто нечего мерить).
pub async fn run_health_report(definitions: &[ToolDefinition]) -> HealthReport {
    let mut tools = Vec::new();
    let mut checked = 0usize;
    let mut healthy = 0usize;

    for def in definitions {
        let status = discovery::detect_tool(def).await;
        let health = check_tool(def, &status).await;
        if !def.health_checks.is_empty() {
            checked += 1;
            if health.ok {
                healthy += 1;
            }
        }
        tools.push(health);
    }

    let score = if checked == 0 {
        0
    } else {
        (healthy * 100 / checked) as u8
    };

    HealthReport {
        tools,
        score,
        scanned_at: super::console::timestamp(),
    }
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Определение с заданными health_checks (pattern — check.rs::fake_def).
    /// Версионная проба = cmd /c echo: detect_tool считает тул
    /// установленным, поэтому тесты отчёта видят Installed.
    fn def_with_checks(checks: Vec<Vec<String>>) -> ToolDefinition {
        ToolDefinition {
            id: "fake".to_string(),
            category: "utility".to_string(),
            display: "Fake Tool".to_string(),
            description: "тестовый инструмент".to_string(),
            icon: None,
            detection: DetectionRules {
                version_probes: vec![vec![
                    "cmd".to_string(),
                    "/c".to_string(),
                    "echo".to_string(),
                    "1.2.3".to_string(),
                ]],
                known_paths: vec![],
                registry_keys: vec![],
                ..Default::default()
            },
            versions: Default::default(),
            sources: Default::default(),
            size_mb: 0,
            needs_admin: false,
            path_entries: vec![],
            bundled_with: None,
            manual_install: None,
            extended: Default::default(),
            health_checks: checks
                .into_iter()
                .map(|command| HealthCheck {
                    label: command.join(" "),
                    command,
                })
                .collect(),
            notes: None,
        }
    }

    #[tokio::test]
    async fn healthy_tool_all_checks_ok() {
        let def = def_with_checks(vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "hi".to_string(),
        ]]);
        let health = check_tool(
            &def,
            &ToolStatus::Installed {
                version: "1".to_string(),
            },
        )
        .await;

        assert!(health.ok);
        assert_eq!(health.state, HealthState::Healthy);
        assert_eq!(health.checks.len(), 1);
        assert!(health.checks[0].ok);
        assert_eq!(health.checks[0].detail, "hi");
    }

    #[tokio::test]
    async fn failing_check_marks_unhealthy() {
        let def = def_with_checks(vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "exit".to_string(),
            "1".to_string(),
        ]]);
        let health = check_tool(
            &def,
            &ToolStatus::Installed {
                version: "1".to_string(),
            },
        )
        .await;

        assert!(!health.ok);
        assert_eq!(health.state, HealthState::Failed);
        assert_eq!(health.checks.len(), 1);
        assert!(!health.checks[0].ok);
        assert!(health.checks[0].detail.contains("не выполнилась"));
    }

    #[tokio::test]
    async fn mixed_checks_all_must_pass() {
        let def = def_with_checks(vec![
            vec![
                "cmd".to_string(),
                "/c".to_string(),
                "echo".to_string(),
                "ok".to_string(),
            ],
            vec![
                "cmd".to_string(),
                "/c".to_string(),
                "exit".to_string(),
                "1".to_string(),
            ],
        ]);
        let health = check_tool(
            &def,
            &ToolStatus::Installed {
                version: "1".to_string(),
            },
        )
        .await;

        assert!(!health.ok, "одна упавшая проверка рушит весь тул");
        assert_eq!(health.checks.len(), 2);
    }

    #[tokio::test]
    async fn missing_tool_is_unavailable_without_checks() {
        let def = def_with_checks(vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
        ]]);
        let health = check_tool(&def, &ToolStatus::Missing).await;

        assert!(!health.ok);
        assert_eq!(health.state, HealthState::Unavailable);
        assert_eq!(health.checks.len(), 1, "одна запись «не установлен»");
        assert_eq!(health.checks[0].detail, "не установлен / сломан путь");
    }

    #[tokio::test]
    async fn path_broken_is_unavailable() {
        let def = def_with_checks(vec![vec!["cmd".to_string()]]);
        let health = check_tool(
            &def,
            &ToolStatus::PathBroken {
                reason: "нет в PATH".to_string(),
            },
        )
        .await;

        assert!(!health.ok);
        assert_eq!(health.state, HealthState::Unavailable);
    }

    #[test]
    fn no_checks_is_not_checked_not_unhealthy() {
        // Пустой список проверок — «не проверяли» (NotChecked),
        // а не отрицательный вердикт. ok=false сохранён для старого
        // фронтенда, но state различает смысл.
        let def = def_with_checks(vec![]);
        let health = futures_block_on(check_tool(
            &def,
            &ToolStatus::Installed {
                version: "1".to_string(),
            },
        ));

        assert!(!health.ok);
        assert_eq!(health.state, HealthState::NotChecked);
        assert!(health.checks.is_empty());
    }

    #[test]
    fn empty_command_marks_failed() {
        let def = def_with_checks(vec![vec![]]);
        let health = futures_block_on(check_tool(
            &def,
            &ToolStatus::Installed {
                version: "1".to_string(),
            },
        ));

        assert!(!health.ok);
        assert_eq!(
            health.checks[0].detail,
            "команда пустая (ошибка в tools.json)"
        );
    }

    #[test]
    fn update_available_tool_is_checked() {
        // Устаревший тул всё равно проверяется на здоровье
        let def = def_with_checks(vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "v1".to_string(),
        ]]);
        let health = futures_block_on(check_tool(
            &def,
            &ToolStatus::UpdateAvailable {
                installed: "1".to_string(),
                recommended: "2".to_string(),
            },
        ));

        assert!(health.ok);
        assert_eq!(health.state, HealthState::Healthy);
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn report_score_counts_checked_only() {
        let mut installed = def_with_checks(vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "echo".to_string(),
            "hi".to_string(),
        ]]);
        installed.id = "checked-ok".to_string();
        let mut failed = def_with_checks(vec![vec![
            "cmd".to_string(),
            "/c".to_string(),
            "exit".to_string(),
            "1".to_string(),
        ]]);
        failed.id = "checked-bad".to_string();
        let mut unchecked = def_with_checks(vec![]);
        unchecked.id = "unchecked".to_string();

        // unchecked не штрафует score: 1 из 2 = 50
        let report = run_health_report(&[installed, failed, unchecked]).await;
        assert_eq!(report.tools.len(), 3);
        assert_eq!(report.score, 50);
        assert!(!report.scanned_at.is_empty());
    }

    /// Маленький хелпер: прогнать async-функцию в синхронном тесте.
    /// (В этом модуле часть тестов не дергает процессы — им не нужен
    /// tokio-рантайм, но check_tool всё равно async.)
    fn futures_block_on<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(fut)
    }
}
