// ============================================================
// Оценка окружения (domain/score.rs)
// ============================================================
// ДОКУМЕНТИРОВАННАЯ ФОРМУЛА (контракт §7):
//
//   Знаменатель — только ТРЕБУЕМЫЕ и ПРИМЕНИМЫЕ инструменты:
//     required = инструмент нужен для работы (не optional/docker/manual/
//                built-in/bundled-missing) И применим к платформе.
//
//   Вклад каждого инструмента в числитель (в сотых долях):
//     healthy  (InstalledHealthy)            → 100
//     degraded (UpdateAvailable)             →  50   «работает, но устарел»
//     missing / PathBroken / Unhealthy       →   0
//
//   ИСКЛЮЧАЮТСЯ из формулы (но считаются отдельно в ScoreSummary):
//     - unchecked: проверок здоровья нет → «не знаем», не «плохо»;
//     - scan_failed / scan_pending: неизвестно ≠ провал;
//     - optional / docker-managed / manual-only / built-in;
//     - not_applicable: неподдерживаемые на этой платформе;
//     - bundled-инструменты, отсутствующие вместе с носителем.
//
//   score = round(Σ вкладов / (counted_tools × 100) × 100), 0..100
//
// Гарантии, которые проверяют тесты:
//   - пустое окружение → score 0 при counted_tools = 0 (нет данных);
//   - unchecked/unsupported никогда не опускают оценку;
//   - degraded тянет вниз ровно наполовину.

use super::models::{PlatformApplicability, ScoreSummary, ToolScanResult, ToolState};

/// Классификация одного результата для сводки. Возвращает
/// (вклад_в_числитель_сотые, входит_в_знаменатель).
fn classify(result: &ToolScanResult) -> (u64, bool) {
    // Неприменимость решается раньше состояния: msvc-build-tools на Linux
    // не «missing», а «неприменим» — даже если формально Missing.
    if matches!(
        result.applicability,
        PlatformApplicability::UnsupportedOnPlatform
    ) && !matches!(result.state, ToolState::InstalledHealthy { .. })
    {
        return (0, false);
    }

    match &result.state {
        // Неизвестность любого рода не штрафует.
        ToolState::ScanPending => (0, false),
        ToolState::ScanFailed { .. } => (0, false),
        // Опциональные/ручные/docker/встроенные — вне знаменателя.
        ToolState::DockerManaged => (0, false),
        ToolState::ManualInstall { .. } => (0, false),
        ToolState::BuiltInSystem => (0, false),
        ToolState::InstallUnavailable => (0, false),
        ToolState::UnsupportedPlatform => (0, false),

        // Bundled-инструмент без носителя (npm без node) — не требование.
        ToolState::Missing if result.bundled_with.is_some() => (0, false),

        ToolState::Missing => (0, true),
        ToolState::PathBroken { .. } => (0, true),
        ToolState::InstalledUnhealthy { .. } => (0, true),
        ToolState::InstalledHealthUnknown { .. } => (0, false), // unchecked
        ToolState::InstalledHealthy { .. } => (100, true),
        ToolState::UpdateAvailable { .. } => (50, true),
    }
}

/// Считает сводку по готовым результатам скана (детерминированный порядок).
pub fn compute_score(tools: &[ToolScanResult]) -> ScoreSummary {
    let mut summary = ScoreSummary::default();
    let mut earned: u64 = 0;

    for result in tools {
        let (credit, counted) = classify(result);
        earned += credit;

        match &result.state {
            ToolState::ScanPending => summary.scan_failed += 1,
            ToolState::ScanFailed { .. } => summary.scan_failed += 1,
            ToolState::Missing => {
                if result.bundled_with.is_some() {
                    summary.not_applicable += 1;
                } else if matches!(
                    result.applicability,
                    PlatformApplicability::UnsupportedOnPlatform
                ) {
                    summary.not_applicable += 1;
                } else {
                    summary.missing_required += 1;
                }
            }
            ToolState::PathBroken { .. } => summary.broken_required += 1,
            ToolState::InstalledUnhealthy { .. } => summary.unhealthy_required += 1,
            ToolState::InstalledHealthUnknown { .. } => summary.unchecked += 1,
            ToolState::InstalledHealthy { .. } => summary.healthy_required += 1,
            ToolState::UpdateAvailable { .. } => summary.degraded += 1,
            ToolState::DockerManaged
            | ToolState::ManualInstall { .. }
            | ToolState::BuiltInSystem
            | ToolState::InstallUnavailable => summary.optional += 1,
            ToolState::UnsupportedPlatform => summary.not_applicable += 1,
        }

        if counted {
            summary.counted_tools += 1;
        }
    }

    let total_credit = summary.counted_tools as u64 * 100;
    summary.score = if total_credit == 0 {
        0
    } else {
        ((earned * 100 + total_credit / 2) / total_credit).min(100) as u8
    };
    summary
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::toolchain::domain::models::ToolState;
    use crate::modules::toolchain::domain::models::{
        DetectionOutcome, HealthOutcome, HealthState, PlatformApplicability, Provenance,
        VersionAssessment,
    };

    fn result(tool_id: &str, state: ToolState) -> ToolScanResult {
        ToolScanResult {
            tool_id: tool_id.to_string(),
            display: tool_id.to_string(),
            category: "utility".to_string(),
            icon: None,
            detection: DetectionOutcome::NotDetected,
            installs: vec![],
            path_findings: vec![],
            health: Some(HealthOutcome {
                state: HealthState::NoChecksDefined,
                results: vec![],
            }),
            applicability: PlatformApplicability::Installable,
            capabilities: Default::default(),
            provenance: Provenance::Unknown,
            bundled_with: None,
            version_assessment: VersionAssessment::Unknown,
            state,
            error: None,
            duration_ms: 0,
        }
    }

    #[test]
    fn empty_catalog_scores_zero_without_tools() {
        let summary = compute_score(&[]);
        assert_eq!(summary.score, 0);
        assert_eq!(summary.counted_tools, 0);
    }

    #[test]
    fn all_healthy_is_hundred() {
        let tools = vec![
            result(
                "a",
                ToolState::InstalledHealthy {
                    version: "1".into(),
                },
            ),
            result(
                "b",
                ToolState::InstalledHealthy {
                    version: "2".into(),
                },
            ),
        ];
        let summary = compute_score(&tools);
        assert_eq!(summary.score, 100);
        assert_eq!(summary.counted_tools, 2);
        assert_eq!(summary.healthy_required, 2);
    }

    #[test]
    fn missing_drops_score_proportionally() {
        let tools = vec![
            result(
                "a",
                ToolState::InstalledHealthy {
                    version: "1".into(),
                },
            ),
            result("b", ToolState::Missing),
        ];
        assert_eq!(compute_score(&tools).score, 50);
    }

    #[test]
    fn degraded_counts_half() {
        let tools = vec![
            result(
                "a",
                ToolState::InstalledHealthy {
                    version: "2".into(),
                },
            ),
            result(
                "b",
                ToolState::UpdateAvailable {
                    installed: "1".into(),
                    recommended: "2".into(),
                },
            ),
        ];
        let summary = compute_score(&tools);
        assert_eq!(summary.score, 75, "100 + 50 = 150 из 200");
        assert_eq!(summary.degraded, 1);
    }

    #[test]
    fn broken_and_unhealthy_count_zero() {
        let tools = vec![
            result("a", ToolState::PathBroken { reason: "x".into() }),
            result(
                "b",
                ToolState::InstalledUnhealthy {
                    version: "1".into(),
                },
            ),
        ];
        let summary = compute_score(&tools);
        assert_eq!(summary.score, 0);
        assert_eq!(summary.broken_required, 1);
        assert_eq!(summary.unhealthy_required, 1);
    }

    #[test]
    fn unchecked_tools_never_penalize() {
        // Только unchecked-инструменты: знаменатель пуст, оценка не падает.
        let tools = vec![result(
            "a",
            ToolState::InstalledHealthUnknown {
                version: "1".into(),
            },
        )];
        let summary = compute_score(&tools);
        assert_eq!(summary.score, 0);
        assert_eq!(summary.counted_tools, 0, "unchecked вне знаменателя");
        assert_eq!(summary.unchecked, 1);

        // Смешанный случай: unchecked не разбавляет здоровую картину.
        let mixed = vec![
            result(
                "h",
                ToolState::InstalledHealthy {
                    version: "1".into(),
                },
            ),
            result(
                "u",
                ToolState::InstalledHealthUnknown {
                    version: "1".into(),
                },
            ),
        ];
        assert_eq!(compute_score(&mixed).score, 100);
    }

    #[test]
    fn unsupported_platform_excluded_even_when_missing() {
        let mut na = result("msvc-like", ToolState::Missing);
        na.applicability = PlatformApplicability::UnsupportedOnPlatform;
        let tools = vec![
            result(
                "ok",
                ToolState::InstalledHealthy {
                    version: "1".into(),
                },
            ),
            na,
        ];
        let summary = compute_score(&tools);
        assert_eq!(summary.score, 100, "неприменимый missing не штрафует");
        assert_eq!(summary.not_applicable, 1);
        assert_eq!(summary.counted_tools, 1);
    }

    #[test]
    fn optional_and_manual_and_docker_outside_denominator() {
        let tools = vec![
            result("d", ToolState::DockerManaged),
            result("m", ToolState::ManualInstall { reason: "x".into() }),
            result("s", ToolState::BuiltInSystem),
            result("i", ToolState::InstallUnavailable),
        ];
        let summary = compute_score(&tools);
        assert_eq!(summary.score, 0);
        assert_eq!(summary.counted_tools, 0);
        assert_eq!(summary.optional, 4);
    }

    #[test]
    fn scan_failed_never_counts_as_failure() {
        let tools = vec![
            result(
                "f",
                ToolState::ScanFailed {
                    reason: "timeout".into(),
                },
            ),
            result("p", ToolState::ScanPending),
        ];
        let summary = compute_score(&tools);
        assert_eq!(summary.counted_tools, 0, "неизвестное исключено из формулы");
        assert_eq!(summary.scan_failed, 2);
    }

    #[test]
    fn bundled_missing_is_not_a_failure() {
        let mut npm = result("npm", ToolState::Missing);
        npm.bundled_with = Some("node".to_string());
        let tools = vec![
            result(
                "node",
                ToolState::InstalledHealthy {
                    version: "22".into(),
                },
            ),
            npm,
        ];
        let summary = compute_score(&tools);
        assert_eq!(summary.score, 100);
        assert_eq!(summary.not_applicable, 1);
    }

    #[test]
    fn rounding_is_half_up() {
        // 3 инструмента: 100+100+0 = 200/300 → 66.67 → 67.
        let tools = vec![
            result(
                "a",
                ToolState::InstalledHealthy {
                    version: "1".into(),
                },
            ),
            result(
                "b",
                ToolState::InstalledHealthy {
                    version: "1".into(),
                },
            ),
            result("c", ToolState::Missing),
        ];
        assert_eq!(compute_score(&tools).score, 67);
    }
}
