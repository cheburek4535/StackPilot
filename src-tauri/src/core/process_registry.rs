//! Глобальный реестр процессов, запущенных именно StackPilot.
//!
//! Сюда регистрируются только те процессы, которые приложение породило само
//! (workspace/devlauncher-менеджер процессов и ProcessRunner project_creator) —
//! чужие процессы никогда не попадают в список. При закрытии приложения
//! пользователь может завершить их (диалог в самом приложении или настройка
//! «всегда/никогда»).
//!
//! Ограничение: реестр хранит PID, а не дескриптор процесса. Крайне редкий
//! сценарий переиспользования PID другим процессом между запуском и выходом
//! не отслеживается — на практике окно жизни реестра измеряется минутами.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::Duration;
use tauri::Emitter;
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

/// Процесс для диалога выхода: только то, что нужно показать (никаких
/// хвостов логов и огромных полей — вкладка рисует понятные карточки).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExitAskProcess {
    pub pid: u32,
    /// Человекочитаемый заголовок (метка шага профиля, команда для
    /// project_creator).
    pub title: String,
    /// Краткая командная строка «command args…» (обёрнута, не длиннее 120).
    pub command: String,
    /// Группа для иконки в диалоге: docker/node/python/jvm/compiled/shell/other.
    pub kind: &'static str,
    pub source: &'static str,
    pub started_at: String,
}

/// Событие `sp:exit-request` — список процессов StackPilot, которые надо
/// решить завершить/оставить перед выходом.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExitAskPayload {
    pub processes: Vec<ExitAskProcess>,
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

/// Группа процесса для понятной иконки в диалоге выхода: по команде
/// определяем, что пользователь увидит («Docker», «Node.js», «Python»,
/// «JVM», «сборка», «терминал» или «процесс»).
fn kind_of(p: &SpawnedProcessInfo) -> &'static str {
    let hay = format!("{} {}", p.command, p.args.join(" ")).to_ascii_lowercase();
    if hay.contains("docker") || hay.contains("podman") {
        "docker"
    } else if hay.contains("python")
        || hay.contains("uvicorn")
        || hay.contains("gunicorn")
        || hay.contains("flask")
        || hay.contains("django")
        || hay.contains("daphne")
    {
        "python"
    } else if hay.contains("npm")
        || hay.contains("npx")
        || hay.contains("pnpm")
        || hay.contains("yarn")
        || hay.contains("bun")
        || hay.contains("node")
        || hay.contains("nest")
        || hay.contains("next")
        || hay.contains("vite")
        || hay.contains("nuxt")
        || hay.contains("expo ")
    {
        "node"
    } else if hay.contains("gradle")
        || hay.contains("gradlew")
        || hay.contains("mvn")
        || hay.contains("java")
        || hay.contains("kotlin")
        || hay.contains("spring")
        || hay.contains("ktor")
    {
        "jvm"
    } else if hay.contains("cargo")
        || hay.contains("rust")
        || hay.contains("go run")
        || hay.contains("dotnet")
        || hay.contains("qmake")
        || hay.contains("cmake")
        || hay.contains("zig ")
    {
        "compiled"
    } else if hay.contains("cmd")
        || hay.contains("powershell")
        || hay.contains("pwsh")
        || hay.contains("sh -")
        || hay.contains("bash")
    {
        "shell"
    } else {
        "other"
    }
}

/// Подготовить краткую командную строку для диалога (без многострочных
/// скриптов — только первый аргумент-фрагмент, обрезанный до 120 символов).
fn exit_command_line(p: &SpawnedProcessInfo) -> String {
    let mut line = p.command.clone();
    let trimmed_args: Vec<&String> = p
        .args
        .iter()
        .filter(|a| !a.contains('\n'))
        .take(4)
        .collect();
    for a in &trimmed_args {
        line.push(' ');
        line.push_str(a);
    }
    if line.chars().count() > 120 {
        let cut: String = line.chars().take(117).collect();
        line = format!("{cut}…");
    }
    line
}

fn exit_ask_process(p: &SpawnedProcessInfo) -> ExitAskProcess {
    ExitAskProcess {
        pid: p.pid,
        title: p.label.clone(),
        command: exit_command_line(p),
        kind: kind_of(p),
        source: p.source,
        started_at: p.started_at.clone(),
    }
}

/// Завершить дерево процесса по PID. Windows: `taskkill /F /T` (всё дерево)
/// со скрытым окном консоли — завершение должно быть бесшумным, без
/// вспышек терминалов; Unix: SIGTERM → короткая пауза → SIGKILL.
fn terminate_by_pid(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        let status = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(crate::platform::CREATE_NO_WINDOW)
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

// ---------------------------------------------------------------------------
// Запрос выхода: диалог показывается В ПРИЛОЖЕНИИ (вкладка UI), а не
// нативным окном. Цепочка: закрытие окна (или exit из другого места) →
// prevent → событие sp:exit-request с процессным списком → ответ вкладки
// командой resolve_exit_request(terminate) / cancel_exit_request().
// ---------------------------------------------------------------------------

/// Имя события диалога выхода (стабильное, документируется в контракте).
pub const EXIT_REQUEST_EVENT: &str = "sp:exit-request";

/// Сколько ждать ответа вкладки, прежде чем выйти без завершения процессов
/// (страховка от мёртвого вебвью — приложение не должно зависнуть).
const EXIT_ASK_TIMEOUT_SECS: u64 = 20;

/// Вопрос «завершить процессы?» на столе: true, пока вкладка не ответила.
static EXIT_ASK_PENDING: OnceLock<Mutex<bool>> = OnceLock::new();
/// Истёк ли уже таймаут ответа (повторный спавн таймера не нужен).
static EXIT_ASK_TIMEOUT_ARMED: AtomicBool = AtomicBool::new(false);

fn set_exit_ask_pending(pending: bool) {
    if let Ok(mut guard) = EXIT_ASK_PENDING.get_or_init(|| Mutex::new(false)).lock() {
        *guard = pending;
    }
    if !pending {
        EXIT_ASK_TIMEOUT_ARMED.store(false, Ordering::SeqCst);
    }
}

fn is_exit_ask_pending() -> bool {
    EXIT_ASK_PENDING
        .get_or_init(|| Mutex::new(false))
        .lock()
        .map(|g| *g)
        .unwrap_or(false)
}

fn quit_behavior(app: &tauri::AppHandle) -> String {
    app.try_state::<crate::core::settings::SettingsState>()
        .and_then(|s| s.0.get_settings().ok())
        .map(|s| s.quit_process_behavior)
        .unwrap_or_else(|| "ask".to_string())
}

/// Зафиксировать намерение выйти: флаг гасит повторный ExitRequested
/// (app.exit() снова генерирует запрос, и без флага мы бы ушли в диалог).
static EXIT_FINALIZING: AtomicBool = AtomicBool::new(false);

/// Эмитировать вопрос в приложение. Если вкладка не слушает (mёртвое
/// вебвью) — выходим без завершения процессов, не зависая.
fn request_inapp_confirmation(app: &tauri::AppHandle) {
    let procs = list();
    if procs.is_empty() {
        return;
    }
    log::info!("quit: asking UI about {} running process(es)", procs.len());
    set_exit_ask_pending(true);
    let payload = ExitAskPayload {
        processes: procs.iter().map(exit_ask_process).collect(),
    };
    if let Err(e) = app.emit(EXIT_REQUEST_EVENT, payload) {
        log::warn!("quit: cannot reach UI ({e}); exiting without terminating processes");
        set_exit_ask_pending(false);
        exit_app(app, false);
        return;
    }
    // Страховка: если вкладка не ответит за отведённое время, выходим
    // БЕЗ завершения процессов (тихо убивать серверы нельзя — пользователь
    // мог захотеть их оставить; а «завершить всегда» выбирается в
    // настройках отдельно).
    if EXIT_ASK_TIMEOUT_ARMED.swap(true, Ordering::SeqCst) {
        return;
    }
    let app_for_timeout = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(EXIT_ASK_TIMEOUT_SECS)).await;
        if is_exit_ask_pending() {
            log::warn!(
                "quit: UI did not answer within {EXIT_ASK_TIMEOUT_SECS}s; exiting without \
                 terminating processes"
            );
            exit_app(&app_for_timeout, false);
        }
    });
}

/// Применить решение пользователя: terminate — завершить процессы тихо и
/// выйти; иначе просто выйти, оставив их работать.
fn exit_app(app: &tauri::AppHandle, terminate: bool) {
    if EXIT_FINALIZING.swap(true, Ordering::SeqCst) {
        // Уже финализируемся — повторный вызов (таймер + клик, двойной вызов
        // команды) ничего не делает.
        return;
    }
    set_exit_ask_pending(false);
    if terminate {
        let killed = terminate_all();
        log::info!("quit: terminated {killed} process(es) by user choice");
    }
    app.exit(0);
}

/// Обработчик `RunEvent::WindowEvent::CloseRequested` (клик по «Х» /
/// Alt+F4 на главном окне): «always» завершает процессы и даёт закрыться,
/// «never» — просто закрывает, «ask» — показываем диалог в приложении.
pub fn handle_window_close_request(app: &tauri::AppHandle, api: tauri::CloseRequestApi) {
    if EXIT_FINALIZING.load(Ordering::SeqCst) {
        return;
    }
    match quit_behavior(app).as_str() {
        "always" => {
            let killed = terminate_all();
            log::info!("quit: terminated {killed} process(es) (setting: always)");
        }
        "never" => {
            log::debug!("quit: processes left running (setting: never)");
        }
        _ => {
            if list().is_empty() {
                log::debug!("quit: no StackPilot processes running");
                return;
            }
            api.prevent_close();
            request_inapp_confirmation(app);
        }
    }
}

/// Обработчик `RunEvent::ExitRequested` — запасной путь, когда выход
/// запрашивается не закрытием окна (system shutdown, app.exit извне).
/// Тот же диалог через вкладку; если вкладки уже нет — выходим без
/// завершения (см. таймаут в request_inapp_confirmation).
pub fn handle_exit_request(
    app: &tauri::AppHandle,
    api: tauri::ExitRequestApi,
    _code: Option<i32>,
) {
    // Уже финализируемся по выбору пользователя — пропускаем молча.
    if EXIT_FINALIZING.load(Ordering::SeqCst) {
        return;
    }
    match quit_behavior(app).as_str() {
        "always" => {
            let killed = terminate_all();
            log::info!("quit: terminated {killed} process(es) (setting: always)");
        }
        "never" => {
            log::debug!("quit: processes left running (setting: never)");
        }
        _ => {
            if list().is_empty() {
                log::debug!("quit: no StackPilot processes running");
                return;
            }
            api.prevent_exit();
            if is_exit_ask_pending() {
                // Диалог уже открыт (окно держим закрытым через
                // handle_window_close_request) — повторный запрос не нужен.
                return;
            }
            request_inapp_confirmation(app);
        }
    }
}

/// Команда из диалога выхода: `terminate=true` — тихо завершить процессы
/// StackPilot и выйти; `false` — выйти, оставив процессы работать.
#[tauri::command]
pub fn resolve_exit_request(app: tauri::AppHandle, terminate: bool) -> Result<(), String> {
    log::info!(
        "quit: UI answered the exit request (terminate={terminate}, {} process(es) listed)",
        list().len()
    );
    exit_app(&app, terminate);
    Ok(())
}

/// Команда из диалога выхода: пользователь передумал — остаёмся в
/// приложении, окно продолжает работать.
#[tauri::command]
pub fn cancel_exit_request() -> Result<(), String> {
    log::debug!("quit: UI aborted the exit request");
    set_exit_ask_pending(false);
    Ok(())
}

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

    #[test]
    fn kind_classifies_common_processes() {
        clear();
        let register_test = |label: &str, cmd: &str, args: Vec<&str>| {
            let p = SpawnedProcessInfo {
                pid: 1,
                label: label.into(),
                command: cmd.into(),
                args: args.iter().map(|a| a.to_string()).collect(),
                source: "workspace",
                started_at: "12:00:00".into(),
            };
            kind_of(&p).to_string()
        };
        assert_eq!(register_test("compose", "docker compose up", vec![]), "docker");
        assert_eq!(register_test("compose2", "docker-compose", vec!["-f", "x"]), "docker");
        assert_eq!(register_test("api", "npm run start:dev", vec![]), "node");
        assert_eq!(register_test("nest", "nest start --watch", vec![]), "node");
        assert_eq!(register_test("uvicorn", ".venv\\Scripts\\python.exe", vec!["-m", "uvicorn"]), "python");
        assert_eq!(register_test("spring", "gradlew.bat", vec!["bootRun"]), "jvm");
        assert_eq!(register_test("dotnet", "dotnet", vec!["run"]), "compiled");
        assert_eq!(register_test("sh", "cmd", vec!["/C", "echo hi"]), "shell");
        assert_eq!(register_test("misc", "shiny", vec!["--gen"]), "other");
        clear();
    }

    #[test]
    fn exit_command_line_truncates_and_flattens() {
        clear();
        let p = SpawnedProcessInfo {
            pid: 7,
            label: "x".into(),
            command: "python".into(),
            args: vec!["-m".into(), "very\\long\\path\\with\\many\\segments\\a".repeat(30)],
            source: "project_creator",
            started_at: "12:00:00".into(),
        };
        let line = exit_command_line(&p);
        assert!(line.chars().count() <= 120, "{line}");
        assert!(!line.contains('\n'));
        clear();
    }

    #[test]
    fn pending_flag_toggles() {
        set_exit_ask_pending(false);
        assert!(!is_exit_ask_pending());
        set_exit_ask_pending(true);
        assert!(is_exit_ask_pending());
        set_exit_ask_pending(false);
        assert!(!is_exit_ask_pending());
    }
}