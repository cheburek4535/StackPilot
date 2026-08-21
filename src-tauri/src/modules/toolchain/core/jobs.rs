// ============================================================
// Журнал заданий установки (jobs.rs)
// ============================================================
// InstallSession живёт в памяти; без журнала перезапуск приложения
// посреди установки терял факт «задание шло» — статус молча исчезал.
//
// Журнал — маленький JSON рядом с state.json:
//   { "session": { started_at, status, plan } }
//
// Правила:
//   - при старте установки пишется Running;
//   - при завершении — терминальный статус (Completed/Failed/Cancelled);
//   - при загрузке приложения запись Running означает «приложение
//     умерло посреди задания» → recover_on_startup() переводит её в
//     Interrupted: ни «running», ни «успех» — честное
//     «прервано, можно посмотреть/перезапустить».
//
// Секреты в журнал НЕ пишутся никогда (только изолированное хранилище).

use std::path::{Path, PathBuf};

use crate::modules::toolchain::models::{InstallPlan, InstallSessionStatus};

const JOBS_FILE: &str = "jobs.json";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PersistedSession {
    pub started_at: String,
    pub status: InstallSessionStatus,
    pub plan: InstallPlan,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct JournalFile {
    #[serde(default)]
    session: Option<PersistedSession>,
}

/// Файловый журнал последней сессии установки.
pub struct JobJournal {
    path: PathBuf,
}

impl JobJournal {
    pub fn load(dir: &Path) -> Self {
        Self {
            path: dir.join(JOBS_FILE),
        }
    }

    /// Читает последнюю записанную сессию (None — журнала нет/битый).
    pub fn read(&self) -> Option<PersistedSession> {
        let raw = std::fs::read_to_string(&self.path).ok()?;
        let parsed: JournalFile = serde_json::from_str(&raw).ok()?;
        parsed.session
    }

    /// Перезаписывает журнал текущей сессией (атомарно, tmp+rename).
    pub fn write(
        &self,
        started_at: &str,
        status: InstallSessionStatus,
        plan: &InstallPlan,
    ) -> Result<(), String> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| format!("Нет родительского каталога для {}", self.path.display()))?;
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Не удалось создать {}: {e}", parent.display()))?;
        let file = JournalFile {
            session: Some(PersistedSession {
                started_at: started_at.to_string(),
                status,
                plan: plan.clone(),
            }),
        };
        let raw = serde_json::to_string_pretty(&file)
            .map_err(|e| format!("Журнал не сериализуется: {e}"))?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, raw)
            .map_err(|e| format!("Не удалось записать {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("Не удалось сохранить {}: {e}", self.path.display()))?;
        Ok(())
    }

    /// Восстановление после перезапуска: если в журнале осталась
    /// Running-сессия, она переписывается как Interrupted и возвращается
    /// восстановленный план. Гарантирует: после рестарта не существует
    /// «вечно идущей» установки.
    pub fn recover_on_startup(&self) -> Option<PersistedSession> {
        let mut session = self.read()?;
        if !matches!(session.status, InstallSessionStatus::Running) {
            return Some(session);
        }
        // Незавершённые задачи помечаются честным «прервано».
        for task in &mut session.plan.tasks {
            task.state = crate::modules::toolchain::models::TaskState::Skipped {
                reason: "Установка прервана перезапуском приложения".to_string(),
            };
        }
        session.status = InstallSessionStatus::Interrupted;
        if self
            .write(&session.started_at, session.status, &session.plan)
            .is_err()
        {
            eprintln!("[toolchain] не удалось зафиксировать Interrupted в журнале заданий");
        }
        Some(session)
    }
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::toolchain::models::{InstallTask, TaskState};

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tc-jobs-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_plan(tool_id: &str) -> InstallPlan {
        InstallPlan {
            tasks: vec![InstallTask {
                task_id: tool_id.to_string(),
                tool_id: tool_id.to_string(),
                display: tool_id.to_string(),
                icon: None,
                size_mb: 1,
                needs_admin: false,
                source_description: "test".to_string(),
                install_options: vec![],
                state: TaskState::Running {
                    phase: crate::modules::toolchain::models::TaskPhase::Installing,
                },
            }],
            total_size_mb: 1,
            os: "windows".to_string(),
            session_id: "s-1".to_string(),
        }
    }

    #[test]
    fn missing_journal_is_none() {
        let journal = JobJournal::load(&temp_dir("missing"));
        assert!(journal.read().is_none());
        assert!(journal.recover_on_startup().is_none());
    }

    #[test]
    fn running_session_recovers_as_interrupted() {
        let dir = temp_dir("interrupted");
        let journal = JobJournal::load(&dir);
        journal
            .write(
                "2026-08-21T10:00:00Z",
                InstallSessionStatus::Running,
                &sample_plan("node"),
            )
            .unwrap();

        let recovered = journal
            .recover_on_startup()
            .expect("сессия должна вернуться");
        assert!(matches!(
            recovered.status,
            InstallSessionStatus::Interrupted
        ));
        assert!(
            recovered
                .plan
                .tasks
                .iter()
                .all(|t| matches!(t.state, TaskState::Skipped { .. })),
            "незавершённые задачи помечаются прерванными"
        );

        // Повторное восстановление уже ничего не меняет (не Running).
        let again = journal.recover_on_startup().unwrap();
        assert!(matches!(again.status, InstallSessionStatus::Interrupted));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn terminal_states_survive_restart_unchanged() {
        let dir = temp_dir("terminal");
        let journal = JobJournal::load(&dir);
        journal
            .write(
                "2026-08-21T10:00:00Z",
                InstallSessionStatus::Completed,
                &sample_plan("git"),
            )
            .unwrap();

        let recovered = journal.recover_on_startup().unwrap();
        assert!(matches!(recovered.status, InstallSessionStatus::Completed));
        // Задачи терминальной сессии не трогаются.
        assert!(matches!(
            recovered.plan.tasks[0].state,
            TaskState::Running { .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_journal_is_tolerated() {
        let dir = temp_dir("broken");
        std::fs::write(dir.join(JOBS_FILE), "{ not json").unwrap();
        let journal = JobJournal::load(&dir);
        assert!(journal.read().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
