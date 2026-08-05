// ============================================================
// Составление плана установки (planner.rs)
// ============================================================
// Этап 3: по отчёту проверки (EnvironmentCheck) строит InstallPlan —
// список задач, которые нужно выполнить, чтобы окружение стало
// готовым.
//
// Правила:
//   - задача создаётся только для «не готовых» требований
//     (Missing или UpdateAvailable — устаревшие идут на апгрейд);
//   - Installed тулы в план не попадают (работать уже можно);
//   - winget ставится ПЕРВЫМ: пока его нет, все остальные пакеты
//     на Windows поставить нечем.

use crate::modules::toolchain::models::*;

/// Строит план установки из отчёта проверки окружения.
/// Порядок требований сохраняется из check (языки → фреймворки →
/// тулы → флаги), кроме winget, который выносится вперёд.
pub fn build_plan(check: &EnvironmentCheck) -> InstallPlan {
    let mut tasks: Vec<InstallTask> = Vec::new();
    let mut total_size_mb: u64 = 0;

    for req in &check.requirements {
        if req.status.is_ok() {
            continue;
        }

        total_size_mb += req.size_mb as u64;
        tasks.push(InstallTask {
            // task_id равен tool_id: каждый инструмент участвует в плане один раз
            task_id: req.tool_id.clone(),
            tool_id: req.tool_id.clone(),
            display: req.display.clone(),
            size_mb: req.size_mb,
            needs_admin: req.needs_admin,
            source_description: req.source_description.clone(),
            state: TaskState::Pending,
        });
    }

    if let Some(i) = tasks.iter().position(|t| t.tool_id == "winget") {
        if i != 0 {
            let winget = tasks.remove(i);
            tasks.insert(0, winget);
        }
    }

    InstallPlan {
        tasks,
        total_size_mb,
        os: check.os.clone(),
    }
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn requirement(tool_id: &str, status: ToolStatus) -> ToolRequirement {
        ToolRequirement {
            tool_id: tool_id.to_string(),
            display: tool_id.to_string(),
            category: "utility".to_string(),
            status,
            size_mb: 10,
            needs_admin: false,
            source_description: "test".to_string(),
        }
    }

    fn check_with(requirements: Vec<ToolRequirement>) -> EnvironmentCheck {
        EnvironmentCheck {
            os: "windows".to_string(),
            requirements,
            total_size_mb: 0,
            free_space_mb: 0,
            enough_space: true,
            needs_admin_any: false,
            all_ready: false,
        }
    }

    #[test]
    fn installed_tools_are_not_scheduled() {
        let check = check_with(vec![requirement("git", ToolStatus::Installed {
            version: "2.48".to_string(),
        })]);
        let plan = build_plan(&check);
        assert!(plan.tasks.is_empty());
        assert_eq!(plan.total_size_mb, 0);
    }

    #[test]
    fn missing_and_outdated_become_tasks() {
        let check = check_with(vec![
            requirement("git", ToolStatus::Installed {
                version: "2.48".to_string(),
            }),
            requirement("node", ToolStatus::Missing),
            requirement("python", ToolStatus::UpdateAvailable {
                installed: "3.9".to_string(),
                recommended: "3.13".to_string(),
            }),
        ]);

        let plan = build_plan(&check);
        let ids: Vec<&str> = plan.tasks.iter().map(|t| t.tool_id.as_str()).collect();
        assert_eq!(ids, vec!["node", "python"]);
        assert_eq!(plan.total_size_mb, 20);
        assert!(plan.tasks.iter().all(|t| matches!(t.state, TaskState::Pending)));
    }

    #[test]
    fn winget_goes_first() {
        let check = check_with(vec![
            requirement("node", ToolStatus::Missing),
            requirement("winget", ToolStatus::Missing),
            requirement("git", ToolStatus::Missing),
        ]);

        let plan = build_plan(&check);
        assert_eq!(plan.tasks[0].tool_id, "winget");
        // порядок остальных не меняется (относительный)
        let rest: Vec<&str> = plan.tasks.iter().skip(1).map(|t| t.tool_id.as_str()).collect();
        assert_eq!(rest, vec!["node", "git"]);
    }

    #[test]
    fn winget_already_first_stays_put() {
        let check = check_with(vec![
            requirement("winget", ToolStatus::Missing),
            requirement("git", ToolStatus::Missing),
        ]);
        let plan = build_plan(&check);
        assert_eq!(plan.tasks[0].tool_id, "winget");
        assert_eq!(plan.tasks.len(), 2);
    }
}