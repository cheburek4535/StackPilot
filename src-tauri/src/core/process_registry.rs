//! Глобальный реестр процессов, запущенных именно StackPilot.
//!
//! Сюда регистрируются только те процессы, которые приложение породило само
//! (workspace/devlauncher-менеджер процессов и ProcessRunner project_creator) —
//! чужие процессы никогда не попадают в список. При закрытии приложения
//! пользователь может завершить их (диалог или настройка «всегда/никогда»).
//!
//! Ограничение: реестр хранит PID, а не дескриптор процесса. Крайне редкий
//! сценарий переиспользования PID другим процессом между запуском и выходом
//! не отслеживается — на практике окно жизни реестра измеряется минутами.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::Manager;

/// Один отслеживаемый процесс (детали для диалога при выходе).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpawnedProcessInfo {
    pub pid: u32,
    pub label: String,
    pub command: String,
    pub args: Vec<String>,
    /// Кто породил: "workspace" | "project_creator".
    pub source: &'static str,
    pub started_at: String,
}

static REGISTRY: OnceLock<Mutex<Vec<SpawnedProcessInfo>>> = OnceLock::new();

fn registry() -> &'static Mutex<Vec<SpawnedProcessInfo>> {
    REGISTRY.get_or_init(|| Mutex::new(Vec::new()))
}

/// Зарегистрировать порождённый процесс. Дубли по pid заменяются.
pub fn register(
    pid: u32,
    label: impl Into<String>,
    command: impl Into<String>,
    args: Vec<String>,
    source: &'static str,
) {
    if pid == 0 {
        return;
    }
    let info = SpawnedProcessInfo {
        pid,
        label: label.into(),
        command: command.into(),
        args,
        source,
        started_at: chrono::Local::now().format("%H:%M:%S").to_string(),
    };
    if let Ok(mut guard) = registry().lock() {
        guard.retain(|p| p.pid != pid);
        guard.push(info);
    }
}

/// Текущий список процессов StackPilot.
pub fn list() -> Vec<SpawnedProcessInfo> {
    registry()
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

/// Для тестов.
pub fn clear() {
    if let Ok(mut guard) = registry().lock() {
        guard.clear();
    }
}

/// Человекочитаемая строка процесса для диалога: `label (pid) — command args`.
fn describe(p: &SpawnedProcessInfo) -> String {
    let mut line = format!("{} (pid {}) — {}", p.label, p.pid, p.command);
    if !p.args.is_empty() {
        line.push(' ');
        line.push_str(&p.args.join(" "));
    }
    line
}

/// Завершить дерево процесса по PID. Windows: `taskkill /F /T` (всё дерево);
/// Unix: SIGTERM → короткая пауза → SIGKILL.
fn terminate_by_pid(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::process::Command;
        let status = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        matches!(status, Ok(s) if s.success())
    }
    #[cfg(not(target_os = "windows"))]
    {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
        unsafe {
            libc::kill(pid as i32, libc::SIGKILL);
        }
        true
    }
}

/// Завершить ВСЕ отслеживаемые процессы. Возвращает число завершённых.
pub fn terminate_all() -> usize {
    let procs = list();
    let mut killed = 0;
    for p in &procs {
        if terminate_by_pid(p.pid) {
            killed += 1;
        }
    }
    clear();
    killed
}

/// Обработчик `RunEvent::ExitRequested`: настройка «ask» показывает диалог с
/// перечнем процессов StackPilot; «always» завершает всё без вопросов;
/// «never» просто закрывает приложение.
pub fn handle_exit_request(
    app: &tauri::AppHandle,
    api: tauri::ExitRequestApi,
    _code: Option<i32>,
) {
    // Защита от рекурсии: после диалога мы вызываем app.exit(0), что снова
    // порождает ExitRequested — повторный запрос пропускаем молча.
    if EXIT_FINALIZING.swap(true, Ordering::SeqCst) {
        return;
    }

    let behavior = app
        .try_state::<crate::core::settings::SettingsState>()
        .and_then(|s| s.0.get_settings().ok())
        .map(|s| s.quit_process_behavior)
        .unwrap_or_default();

    match behavior.as_str() {
        "always" => {
            let killed = terminate_all();
            log::info!("quit: terminated {killed} StackPilot process(es) (setting: always)");
        }
        "never" => {
            log::debug!("quit: processes left running (setting: never)");
        }
        _ => {
            let procs = list();
            if procs.is_empty() {
                log::debug!("quit: no StackPilot processes running");
                return;
            }
            api.prevent_exit();
            let details: Vec<String> = procs.iter().map(describe).collect();
            let message = format!(
                "{} StackPilot-процесс(ов) ещё запущено:\n\n{}\n\nЗавершить их перед выходом?",
                procs.len(),
                details.join("\n")
            );
            log::info!("quit: asking about {} running process(es)", procs.len());
            use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
            let app_for_callback = app.clone();
            app.dialog()
                .message(message)
                .title("Закрытие StackPilot")
                .kind(MessageDialogKind::Warning)
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "Завершить и выйти".into(),
                    "Выйти без завершения".into(),
                ))
                .show(move |terminate: bool| {
                    if terminate {
                        let killed = terminate_all();
                        log::info!("quit: terminated {killed} process(es) by user choice");
                    } else {
                        log::info!("quit: user chose to leave processes running");
                    }
                    app_for_callback.exit(0);
                });
        }
    }
}

static EXIT_FINALIZING: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_dedupes_and_lists() {
        clear();
        register(1001, "dev server", "npm", vec!["run".into(), "dev".into()], "workspace");
        register(1002, "scaffold", "npx", vec!["create".into()], "project_creator");
        // Повторная регистрация того же pid заменяет запись, а не дублирует.
        register(1001, "dev server v2", "npm", vec!["run".into(), "dev".into()], "workspace");

        let procs = list();
        assert_eq!(procs.len(), 2, "{procs:?}");
        assert!(procs.iter().any(|p| p.pid == 1002 && p.source == "project_creator"));
        assert_eq!(
            procs.iter().find(|p| p.pid == 1001).map(|p| p.label.as_str()),
            Some("dev server v2")
        );

        clear();
        assert!(list().is_empty());
    }

    #[test]
    fn zero_pid_is_ignored() {
        clear();
        register(0, "ghost", "x", vec![], "workspace");
        assert!(list().is_empty());
    }
}