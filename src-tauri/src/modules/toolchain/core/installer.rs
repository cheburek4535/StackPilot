// ============================================================
// Исполнение плана установки (installer.rs)
// ============================================================
// Этап 3–4: берёт InstallPlan и выполняет задачи по очереди
// (последовательно — установщики не любят конкуренцию).
//
// Схема одной задачи:
//   1. TaskStarted;
//   2. (Official/Script, http-URL) Downloading — console::download
//      с прогрессом (tc:dl) и таймаутом;
//   3. Installing — запуск установщика; если задача помечена
//      needs_admin — через console::run_elevated (UAC-подтверждение),
//      иначе console::piped_run. Строки вывода → TaskProgress;
//   4. Verifying — повторное обнаружение (discovery::detect_tool);
//   5. TaskCompleted → Success/Failed/Skipped.
//
// Отмена: флаг abort (Arc<AtomicBool>) проверяется перед задачей,
// а в piped_run — и во время исполнения (процесс убивается).
// Отменённая задача помечается Skipped{reason: "Отменено"}.
//
// Windows-first: Linux/macOS-задачи помечаются Skipped —
// их установка появится позже (sudo/brew обёртки).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::modules::toolchain::models::*;
use crate::modules::toolchain::platforms;

use super::console::{self, ps_quote, EventSink};
use super::discovery;
use super::path_service;

// ------------------------------------------------------------
// Команды установки
// ------------------------------------------------------------

/// Готовая к запуску команда: бинарь + аргументы.
#[derive(Debug)]
struct InstallCommand {
    program: String,
    args: Vec<String>,
}

/// Генерирует пароль для БД (PostgreSQL). 16 hex-символов от
/// наносекунд системного времени — достаточно для локальной
/// dev-базы; настоящая генерация/хранение — этап 5 (metadata).
fn generate_db_password() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let mut pw = format!("{nanos:x}");
    while pw.len() > 16 {
        pw.truncate(16);
    }
    while pw.len() < 16 {
        pw.insert(0, '0');
    }
    pw
}

/// Собирает команду установки конкретного источника.
///
/// - PkgManager (winget): `winget install --id <id> <args...>` +
///   при dynamic_args — `--override "--superpassword <pw> --password <pw>"`;
/// - Official: если URL http — ожидает уже скачанный файл в
///   offline_path; .msi запускается через msiexec, .exe напрямую;
///   локальный путь (без http) запускается сразу — удобно для тестов
///   и оффлайн-инсталляторов;
/// - Script: скачанный скрипт/бинарь; .ps1 — через powershell.
///
/// `password` — постгрес-пароль при dynamic_args: true.
fn build_install_command(
    source: &InstallSource,
    offline_path: Option<&Path>,
    password: Option<&str>,
) -> Result<InstallCommand, String> {
    match source.kind {
        InstallSourceKind::PkgManager => {
            let mut args = vec![
                "install".to_string(),
                "--id".to_string(),
                source.id.clone(),
            ];
            args.extend(source.args.iter().cloned());

            if source.dynamic_args {
                let Some(pw) = password else {
                    return Err(format!(
                        "{}: dynamic_args требует пароль, а он не сгенерирован",
                        source.id
                    ));
                };
                args.push("--override".to_string());
                args.push(format!("--superpassword {pw} --password {pw}"));
            }

            args.extend(source.extra_args.iter().cloned());
            Ok(InstallCommand {
                program: "winget".to_string(),
                args,
            })
        }

        InstallSourceKind::Official => {
            let Some(url) = source.url.as_ref() else {
                return Err("Официальная установка без url".to_string());
            };
            let path = match offline_path {
                Some(p) => p.to_path_buf(),
                None => PathBuf::from(url),
            };

            let mut dynamic = Vec::new();
            if source.dynamic_args {
                if let Some(pw) = password {
                    dynamic.push(format!("--superpassword {pw}"));
                    dynamic.push(format!("--password {pw}"));
                }
            }

            match path.extension().and_then(|e| e.to_str()) {
                // MSI-пакеты (Node.js) запускаются через msiexec
                Some(ext) if ext.eq_ignore_ascii_case("msi") => {
                    let mut args = vec!["/i".to_string(), path.to_string_lossy().into_owned()];
                    args.extend(source.args.iter().cloned());
                    args.extend(dynamic);
                    // Подробный MSI-лог: при сбое тихой установки это
                    // единственный способ узнать настоящую причину.
                    let log = std::env::temp_dir().join(format!("tc-{}-msi.log", source.id));
                    args.push("/l*v".to_string());
                    args.push(log.to_string_lossy().into_owned());
                    Ok(InstallCommand {
                        program: "msiexec".to_string(),
                        args,
                    })
                }
                // zip-архивы (gradle, maven): распаковка без прав — в каталог
                // из install_dir (обычно %LOCALAPPDATA%/Programs/<tool>).
                // %VAR% раскрываем здесь: PowerShell (в отличие от cmd)
                // синтаксис %LOCALAPPDATA% не понимает.
                //
                // Fallback на tar: Expand-Archive падает на zip MongoDB
                // (741МБ, битые записи PDB) — встроенный tar (Windows 10+)
                // такие архивы распаковывает.
                Some(ext) if ext.eq_ignore_ascii_case("zip") => {
                    let Some(dir) = source.install_dir.as_ref() else {
                        return Err(format!(
                            "{}: zip-источник требует install_dir в tools.json",
                            source.id
                        ));
                    };
                    let dir = path_service::expand_env_vars(dir);
                    let script = format!(
                        r#"$ErrorActionPreference = 'Stop'
try {{
    Expand-Archive -Path {} -DestinationPath {} -Force
}} catch {{
    Write-Output "tc:warn Expand-Archive не сработал ($($_.Exception.Message)) — пробуем tar"
    tar -xf {} -C {}
    if ($LASTEXITCODE -ne 0) {{
        Write-Output "tc:error tar -xf не смог распаковать архив"
        exit 1
    }}
}}
"#,
                        ps_quote(&path.to_string_lossy()),
                        ps_quote(&dir),
                        ps_quote(&path.to_string_lossy()),
                        ps_quote(&dir)
                    );
                    Ok(InstallCommand {
                        program: "powershell".to_string(),
                        args: vec![
                            "-NoProfile".to_string(),
                            "-ExecutionPolicy".to_string(),
                            "Bypass".to_string(),
                            "-Command".to_string(),
                            script,
                        ],
                    })
                }
                // tgz/tar.gz (kafka): распаковка через встроенный tar —
                // Expand-Archive такие архивы не понимает.
                Some(ext) if ext.eq_ignore_ascii_case("tgz")
                    || ext.eq_ignore_ascii_case("gz")
                    || ext.eq_ignore_ascii_case("tar.gz") => {
                    let Some(dir) = source.install_dir.as_ref() else {
                        return Err(format!(
                            "{}: tar-источник требует install_dir в tools.json",
                            source.id
                        ));
                    };
                    let dir = path_service::expand_env_vars(dir);
                    let script = format!(
                        r#"$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path {1} | Out-Null
tar -xf {0} -C {1}
if ($LASTEXITCODE -ne 0) {{
    Write-Output "tc:error tar -xf не смог распаковать архив (код $LASTEXITCODE)"
    exit 1
}}
# архивы из Unix-мира содержат bat с LF-only переносами, cmd их не понимает —
# нормализуем в CRLF
Get-ChildItem -Path {1} -Recurse -Include *.bat -File | ForEach-Object {{
    $t = [System.IO.File]::ReadAllText($_.FullName, [System.Text.Encoding]::UTF8)
    $t = ($t -replace "`r`n", "`n") -replace "`n", "`r`n"
    [System.IO.File]::WriteAllText($_.FullName, $t, (New-Object System.Text.UTF8Encoding $false))
}}
"#,
                        ps_quote(&path.to_string_lossy()),
                        ps_quote(&dir)
                    );
                    Ok(InstallCommand {
                        program: "powershell".to_string(),
                        args: vec![
                            "-NoProfile".to_string(),
                            "-ExecutionPolicy".to_string(),
                            "Bypass".to_string(),
                            "-Command".to_string(),
                            script,
                        ],
                    })
                }
                // MSIX/msixbundle (winget): установка через App Installer,
                // без прав администратора, per-user. Бандл DesktopAppInstaller
                // требует VCLibs и WindowsAppRuntime 1.8 — их официальный
                // набор лежит в DesktopAppInstaller_Dependencies.zip того же
                // релиза winget-cli; ставим их первыми (мимо Windows Store,
                // skip уже установленных — иначе 0x80073D06).
                Some(ext) if ext.eq_ignore_ascii_case("msix")
                    || ext.eq_ignore_ascii_case("msixbundle") => {
                    let bundle = path.to_string_lossy();
                    let script = format!(
                        r#"$ErrorActionPreference = 'Stop'
try {{
  if (Get-AppxPackage -Name Microsoft.DesktopAppInstaller) {{
    Write-Output 'winget уже установлен'
    exit 0
  }}
  Write-Output 'Скачивание зависимостей winget (DesktopAppInstaller_Dependencies.zip)...'
  $deps = Join-Path $env:TEMP 'tc-winget-deps'
  $archZip = Join-Path $deps 'deps.zip'
  $archDir = Join-Path $deps 'extracted'
  New-Item -ItemType Directory -Force -Path $deps | Out-Null
  if (-not (Test-Path $archZip)) {{
    Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/microsoft/winget-cli/releases/download/v1.29.280/DesktopAppInstaller_Dependencies.zip' -OutFile $archZip
  }}
  if (-not (Test-Path $archDir)) {{
    Expand-Archive -Path $archZip -DestinationPath $archDir -Force
  }}
  $arch = if ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {{ 'arm64' }} elseif ($env:PROCESSOR_ARCHITECTURE -eq 'x86') {{ 'x86' }} else {{ 'x64' }}
  $pkgDir = Join-Path $archDir $arch
  if (-not (Test-Path $pkgDir)) {{
    Write-Output "tc:error в архиве зависимостей нет папки для архитектуры $arch"
    exit 1
  }}
  Get-ChildItem $pkgDir -Filter *.appx | ForEach-Object {{
    try {{
      Add-AppxPackage $_.FullName
      Write-Output "зависимость установлена: $($_.Name)"
    }} catch {{
      if ($_.Exception.Message -match '0x80073D06') {{
        Write-Output "зависимость уже установлена (пропускаем): $($_.Name)"
      }} else {{
        Write-Output "tc:warn зависимость $($_.Name): $($_.Exception.Message)"
      }}
    }}
  }}
  Add-AppxPackage {}
  if (Get-AppxPackage -Name Microsoft.DesktopAppInstaller) {{
    Write-Output 'winget установлен'
    exit 0
  }}
  Write-Output 'tc:error Add-AppxPackage не зарегистрировал App Installer'
  exit 1
}} catch {{
  Write-Output "tc:error $($_.Exception.Message)"
  exit 1
}}
"#,
                        ps_quote(&bundle)
                    );
                    Ok(InstallCommand {
                        program: "powershell".to_string(),
                        args: vec![
                            "-NoProfile".to_string(),
                            "-ExecutionPolicy".to_string(),
                            "Bypass".to_string(),
                            "-Command".to_string(),
                            script,
                        ],
                    })
                }
                _ => {
                    let mut args = source.args.clone();
                    args.extend(dynamic);
                    Ok(InstallCommand {
                        program: path.to_string_lossy().into_owned(),
                        args,
                    })
                }
            }
        }

        InstallSourceKind::Script => {
            let Some(path) = offline_path else {
                return Err(format!("Скрипт {} не скачан", source.id));
            };
            let is_ps1 = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("ps1"));
            if is_ps1 {
                Ok(InstallCommand {
                    program: "powershell".to_string(),
                    args: vec![
                        "-NoProfile".to_string(),
                        "-ExecutionPolicy".to_string(),
                        "Bypass".to_string(),
                        "-File".to_string(),
                        path.to_string_lossy().into_owned(),
                    ],
                })
            } else {
                Ok(InstallCommand {
                    program: path.to_string_lossy().into_owned(),
                    args: source.args.clone(),
                })
            }
        }

        // QtOnline обрабатывается целиком в qt_installer (в try_install_source
        // происходит ранний выход) — сюда команда не доходит.
        InstallSourceKind::QtOnline => Err(format!(
            "{}: репозиторий Qt ставится отдельным установщиком",
            source.id
        )),
    }
}

/// Имя временного файла для скачиваемого установщика.
fn download_dest(tool_id: &str, url: &str) -> std::path::PathBuf {
    let name = url.split(['/', '?', '#']).next_back().unwrap_or("installer");
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
        .collect();
    let safe = if cleaned.is_empty() {
        "installer".to_string()
    } else {
        cleaned
    };
    std::env::temp_dir().join(format!("tc-{tool_id}-{safe}"))
}

// ------------------------------------------------------------
// Исполнение
// ------------------------------------------------------------

/// Выполняет план: обновляет состояния задач на месте и шлёт события
/// в sink. Возвращает сгенерированные секреты (tool_id → пароль БД).
/// abort — флаг отмены: установка прерывается на ближайшей задаче.
pub async fn execute_plan(
    definitions: &[ToolDefinition],
    plan: &mut InstallPlan,
    sink: Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) -> HashMap<String, String> {
    let total = plan.tasks.len();
    let mut secrets = HashMap::new();

    for (i, task) in plan.tasks.iter_mut().enumerate() {
        let tool_id = task.tool_id.clone();
        let task_id = task.task_id.clone();

        let Some(def) = definitions.iter().find(|d| d.id == tool_id) else {
            task.state = TaskState::Skipped {
                reason: "Инструмент не найден в каталоге".to_string(),
            };
            continue;
        };

        if abort.load(Ordering::SeqCst) {
            for rest in &mut plan.tasks[i..] {
                rest.state = TaskState::Skipped {
                    reason: "Отменено пользователем".to_string(),
                };
            }
            break;
        }

        let state = run_task(def, task, i, total, &sink, &mut secrets, &abort).await;
        task.state = state.clone();

        sink.emit(console::event(
            ToolchainEventType::TaskCompleted { state },
            i,
            total,
            &task_id,
            &tool_id,
        ));
    }

    let success_count = plan
        .tasks
        .iter()
        .filter(|t| matches!(t.state, TaskState::Success { .. }))
        .count();
    let failed: Vec<String> = plan
        .tasks
        .iter()
        .filter(|t| matches!(t.state, TaskState::Failed { .. }))
        .map(|t| t.display.clone())
        .collect();
    sink.emit(console::event(
        ToolchainEventType::AllCompleted { success_count, failed },
        0,
        total,
        "",
        "",
    ));

    secrets
}

/// Одна задача установки. Возвращает финальное состояние;
/// TaskCompleted сверху эмитит execute_plan, здесь — только
/// Start/Phase/Progress.
///
/// Источники установки (tools.json) пробуются ПО ПОРЯДКУ: если
/// первый не сработал (npm-глобал упал), переходим ко второму
/// (cargo install). Успех — первого же удачного источника.
async fn run_task(
    def: &ToolDefinition,
    task: &InstallTask,
    index: usize,
    total: usize,
    sink: &Arc<dyn EventSink>,
    secrets: &mut HashMap<String, String>,
    abort: &Arc<AtomicBool>,
) -> TaskState {
    let tool_id = def.id.clone();
    let task_id = task.task_id.clone();
    sink.emit(console::event(ToolchainEventType::TaskStarted, index, total, &task_id, &tool_id));

    if abort.load(Ordering::SeqCst) {
        return TaskState::Skipped {
            reason: "Отменено пользователем".to_string(),
        };
    }

    // Linux/macOS — заглушки (суда и цели репозитория).
    if platforms::current_platform().os_name() != "windows" {
        return TaskState::Skipped {
            reason: "Установка на этой ОС появится позже".to_string(),
        };
    }

    let os_sources: &[InstallSource] = match platforms::current_platform().os_name().as_str() {
        "windows" => &def.sources.windows,
        "linux" => &def.sources.linux,
        "macos" => &def.sources.macos,
        _ => &[],
    };
    if os_sources.is_empty() {
        return TaskState::Skipped {
            reason: "Нет источника установки для этой ОС".to_string(),
        };
    }

    let mut last_error: Option<String> = None;
    for (si, source) in os_sources.iter().enumerate() {
        // Прозрачность для пользователя: почему переключились на запасной
        // источник — раньше это видели только в терминале разработчика.
        if si > 0 {
            let reason = last_error.as_deref().unwrap_or("неизвестная причина");
            sink.emit(console::event(
                ToolchainEventType::TaskProgress {
                    line: format!("tc:info Источник «{}» не сработал: {reason} — пробуем «{}»", os_sources[si - 1].id, source.id),
                },
                index,
                total,
                &task_id,
                &tool_id,
            ));
        }
        match try_install_source(def, source, task, index, total, &task_id, &tool_id, sink, abort).await
        {
            Ok((version, secret)) => {
                if let Some(pw) = secret {
                    secrets.insert(tool_id.clone(), pw);
                }
                sink.emit(console::event(
                    ToolchainEventType::TaskProgress {
                        line: format!("tc:ok Установлено через источник «{}» (версия {version})", source.id),
                    },
                    index,
                    total,
                    &task_id,
                    &tool_id,
                ));
                return TaskState::Success { version };
            }
            Err(e) => {
                eprintln!("[toolchain] источник `{}` для {tool_id} не сработал: {e}", source.id);
                last_error = Some(e);
            }
        }
        if abort.load(Ordering::SeqCst) {
            return TaskState::Skipped {
                reason: "Отменено пользователем".to_string(),
            };
        }
    }

    TaskState::Failed {
        error: last_error.unwrap_or_else(|| "Ни один источник установки не сработал".to_string()),
    }
}

/// Пытается установить инструмент ОДНИМ источником.
/// Возвращает Ok((версия, пароль)) при подтверждённой установке
/// или Err(описание) — источник не сработал, пробуем следующий.
async fn try_install_source(
    def: &ToolDefinition,
    source: &InstallSource,
    task: &InstallTask,
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    sink: &Arc<dyn EventSink>,
    abort: &Arc<AtomicBool>,
) -> Result<(String, Option<String>), String> {
    // QtOnline (Qt из официального репозитория) — свой конвейер:
    // пакетов несколько, каждый качается и распаковывается отдельно.
    if matches!(source.kind, InstallSourceKind::QtOnline) {
        return super::qt_installer::install_qt_online(
            def,
            source,
            &task.install_options,
            index,
            total,
            task_id,
            tool_id,
            sink,
            Arc::clone(abort),
        )
        .await;
    }

    // dynamic_args (PostgreSQL): пароль нужен ещё до запуска установщика.
    let password = source.dynamic_args.then(generate_db_password);
    let mut offline_path: Option<std::path::PathBuf> = None;

    // Источники с http-URL качаем заранее (фаза Downloading, с прогрессом).
    if matches!(source.kind, InstallSourceKind::Official | InstallSourceKind::Script) {
        if let Some(url) = source.url.as_ref() {
            if url.starts_with("http://") || url.starts_with("https://") {
                sink.emit(console::event(
                    ToolchainEventType::TaskPhaseChanged { phase: TaskPhase::Downloading },
                    index,
                    total,
                    task_id,
                    tool_id,
                ));
                let dest = match &source.file_name {
                    Some(name) => std::env::temp_dir().join(name),
                    None => download_dest(tool_id, url),
                };
                if let Err(e) = console::download(url, &dest, index, total, task_id, tool_id, sink, Arc::clone(abort)).await
                {
                    return Err(e);
                }
                offline_path = Some(dest);
            }
        }
    }

    let cmd = build_install_command(source, offline_path.as_deref(), password.as_deref())?;

    sink.emit(console::event(
        ToolchainEventType::TaskPhaseChanged { phase: TaskPhase::Installing },
        index,
        total,
        task_id,
        tool_id,
    ));

    // needs_admin → UAC-элевация; остальные запускаются как есть.
    // У источника может быть своё значение (zip-распаковка не требует UAC,
    // даже если у инструмента в целом needs_admin=true).
    // resolve_command: .cmd/.bat-бинари (npm) оборачивает в cmd /c.
    let needs_admin = source.needs_admin.unwrap_or(def.needs_admin);
    let (program, args) = platforms::resolve_command(&cmd.program, &cmd.args);
    let run = if needs_admin {
        console::run_elevated(&program, &args, index, total, task_id, tool_id, sink, Arc::clone(abort)).await
    } else {
        console::piped_run(&program, &args, index, total, task_id, tool_id, sink, Arc::clone(abort)).await
    };

    let res = match run {
        Ok(r) => r,
        Err(e) => return Err(e),
    };
    if res.aborted {
        return Err("Отменено пользователем".to_string());
    }
    if !res.success {
        // winget: пакет уже установлен, доступных обновлений нет —
        // НЕ сбой установки, а подтверждение, что цель достигнута
        // (0x8A150011 = -1978335189 «уже установлен», 0x8A150015 =
        // -1978335193 «обновление недоступно»). Текста «Найден
        // существующий установленный пакет...» достаточно, чтобы
        // не тратить время на fallback-источники (dart, firebase).
        // Итоговый вердикт выносит verify ниже.
        const WINGET_ALREADY_INSTALLED: i32 = -1978335189;
        const WINGET_UPGRADE_NOT_AVAILABLE: i32 = -1978335193;
        let winget_already_installed = matches!(source.kind, InstallSourceKind::PkgManager)
            && matches!(res.code, WINGET_ALREADY_INSTALLED | WINGET_UPGRADE_NOT_AVAILABLE);
        if !winget_already_installed {
            // tc:error-строка из скрипта (download/run_elevated) — настоящая
            // причина сбоя; код процесса — лишь дополнение к ней.
            return match res.error_line {
                Some(line) => Err(format!("{line} (код {})", res.code)),
                None => Err(format!("Установщик завершился с кодом {}", res.code)),
            };
        }
        sink.emit(console::event(
            ToolchainEventType::TaskProgress {
                line: format!(
                    "tc:info Источник «{}»: пакет уже установлен, обновлений нет — проверяю",
                    source.id
                ),
            },
            index,
            total,
            task_id,
            tool_id,
        ));
    }

    // PATH: установщик (winget/msi/exe) написал свои каталоги в реестр,
    // но текущий процесс об этом не знает. Добавляем явные path_entries
    // из tools.json (glob `PostgreSQL/*/bin` резолвится в конкретный
    // каталог) и обновляем PATH процесса — иначе verify не найдёт
    // свежеустановленный бинарник, хотя он стоит.
    if !def.path_entries.is_empty() {
        sink.emit(console::event(
            ToolchainEventType::TaskPhaseChanged { phase: TaskPhase::UpdatingPath },
            index,
            total,
            task_id,
            tool_id,
        ));
        if let Err(e) = path_service::add_to_user_path(&def.path_entries).await {
            // PATH не критичен для установки — логируем и продолжаем.
            eprintln!("[toolchain] не удалось добавить PATH для {tool_id}: {e}");
        }
    }
    if let Err(e) = path_service::sync_process_path().await {
        eprintln!("[toolchain] не удалось обновить PATH процесса: {e}");
    }

    // Проверка: пересканируем инструмент тем же discovery. Проба
    // known_paths умеет находить бинарь и без PATH (postgres).
    sink.emit(console::event(
        ToolchainEventType::TaskPhaseChanged { phase: TaskPhase::Verifying },
        index,
        total,
        task_id,
        tool_id,
    ));
    match discovery::detect_tool(def).await {
        ToolStatus::Installed { version } => Ok((version, password)),
        _ => Err("Установка не подтвердилась (инструмент не найден)".to_string()),
    }
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::toolchain::defs;
    use std::sync::Mutex;

    /// Локальный приёмник событий (у console — свой; держим модули
    /// тестов независимыми).
    #[derive(Default)]
    struct TestSink {
        events: Mutex<Vec<ToolchainEvent>>,
    }

    impl EventSink for TestSink {
        fn emit(&self, event: ToolchainEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn no_abort() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    /// Определение-«заглушка»: установка — локальный cmd.exe (без сети),
    /// проверка — тоже cmd.exe, отвечающий версией.
    fn echo_def(tool_id: &str) -> ToolDefinition {
        ToolDefinition {
            id: tool_id.to_string(),
            category: "utility".to_string(),
            display: "Local Echo".to_string(),
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
            },
            versions: Default::default(),
            sources: InstallSources {
                windows: vec![InstallSource {
                    kind: InstallSourceKind::Official,
                    id: "local-cmd".to_string(),
                    url: Some("cmd.exe".to_string()),
                    // выводит строку (проверка стриминга), завершается кодом 0
                    args: vec![
                        "/c".to_string(),
                        "echo".to_string(),
                        "installing-local-echo".to_string(),
                    ],
                    extra_args: vec![],
                    dynamic_args: false,
                    install_dir: None,
                    needs_admin: None,
                    file_name: None,
                }],
                linux: vec![],
                macos: vec![],
            },
            size_mb: 1,
            needs_admin: false,
            path_entries: vec![],
            bundled_with: None,
            health_checks: vec![],
            notes: None,
            manual_install: None,
        }
    }

    fn one_task_plan(tool_id: &str) -> InstallPlan {
        InstallPlan {
            tasks: vec![InstallTask {
                task_id: tool_id.to_string(),
                tool_id: tool_id.to_string(),
                display: tool_id.to_string(),
                size_mb: 1,
                needs_admin: false,
                source_description: "test".to_string(),
                install_options: vec![],
                state: TaskState::Pending,
            }],
            total_size_mb: 1,
            os: "windows".to_string(),
        }
    }

    #[test]
    fn password_is_16_hex_chars() {
        let pw = generate_db_password();
        assert_eq!(pw.len(), 16);
        assert!(pw.chars().all(|c| c.is_ascii_hexdigit()), "не hex: {pw}");
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn winget_command_shape() {
        let git = defs::load_definitions()
            .into_iter()
            .find(|d| d.id == "git")
            .unwrap();
        let source = git.sources.windows.first().unwrap();
        let cmd = build_install_command(source, None, None).unwrap();

        assert_eq!(cmd.program, "winget");
        assert!(cmd.args.windows(2).any(|w| w[0] == "install" && w[1] == "--id"));
        assert!(cmd.args.iter().any(|a| a == "Git.Git"));
        // это PkgManager, не dynamic → --override не должно появиться
        assert!(!cmd.args.iter().any(|a| a == "--override"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn postgres_override_carries_password() {
        let pg = defs::load_definitions()
            .into_iter()
            .find(|d| d.id == "postgresql")
            .unwrap();
        let source = pg.sources.windows.first().unwrap();
        assert!(source.dynamic_args, "каталог починили?");
        let cmd = build_install_command(source, None, Some("0123456789abcdef")).unwrap();
        assert!(cmd.args.iter().any(|a| a == "--override"));
        assert!(cmd
            .args
            .iter()
            .any(|a| a.contains("--superpassword 0123456789abcdef")));
    }

    #[test]
    fn official_local_path_runs_directly() {
        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "local".to_string(),
            url: Some("C:/Tools/setup.exe".to_string()),
            args: vec!["--quiet".to_string()],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
                    file_name: None,
        };
        let cmd = build_install_command(&source, None, None).unwrap();
        assert_eq!(cmd.program, "C:/Tools/setup.exe");
        assert_eq!(cmd.args, vec!["--quiet".to_string()]);
    }

    #[test]
    fn official_tgz_expands_via_tar() {
        // kafka: tgz распаковывается через tar (Expand-Archive не умеет)
        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "kafka-tgz".to_string(),
            url: Some("https://example.com/kafka.tgz".to_string()),
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: Some("%LOCALAPPDATA%/Programs/kafka".to_string()),
            needs_admin: None,
                    file_name: None,
        };
        let tgz = std::env::temp_dir().join("tc-tool-kafka.tgz");
        let cmd = build_install_command(&source, Some(&tgz), None).unwrap();
        assert_eq!(cmd.program, "powershell");
        let script = cmd.args.last().unwrap();
        assert!(script.contains("tar -xf"), "нет tar -xf: {script}");
        assert!(
            script.contains("`r`n"),
            "нет CRLF-нормализации bat: {script}"
        );
        // %LOCALAPPDATA% должен быть раскрыт заранее (PS его не понимает)
        let expected = path_service::expand_env_vars("%LOCALAPPDATA%/Programs/kafka");
        assert!(script.contains(&expected), "раскрытый install_dir в скрипте: {script}");
        assert!(!script.contains("%LOCALAPPDATA%"), "сырой %VAR% в скрипте: {script}");
    }

    #[test]
    fn official_msi_runs_via_msiexec() {
        // .msi не выполняется напрямую — только через msiexec
        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "node-msi".to_string(),
            url: Some("node.msi".to_string()),
            args: vec!["/quiet".to_string()],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
                    file_name: None,
        };
        let cmd = build_install_command(&source, None, None).unwrap();
        assert_eq!(cmd.program, "msiexec");
        assert_eq!(cmd.args[0], "/i");
        assert_eq!(cmd.args[1], "node.msi");
        // диагностический MSI-лог должен быть добавлен автоматически
        assert!(cmd.args.iter().any(|a| a == "/l*v"), "нет /l*v: {:?}", cmd.args);
        assert!(
            cmd.args.iter().any(|a| a.ends_with("tc-node-msi-msi.log")),
            "нет пути к MSI-логу: {:?}",
            cmd.args
        );
    }

    #[test]
    fn official_zip_expands_via_powershell() {
        // gradle/maven: zip распаковывается в install_dir, а не запускается
        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "gradle-zip".to_string(),
            url: Some("https://example.com/gradle-9.1.0-bin.zip".to_string()),
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: Some("%LOCALAPPDATA%/Programs/gradle".to_string()),
            needs_admin: None,
                    file_name: None,
        };
        let zip = std::env::temp_dir().join("tc-tool-foo.zip");
        let cmd = build_install_command(&source, Some(&zip), None).unwrap();
        assert_eq!(cmd.program, "powershell");
        let script = cmd.args.last().unwrap();
        assert!(script.contains("Expand-Archive"), "скрипт: {script}");
        // Fallback на tar для «тяжёлых» архивов (mongodb zip)
        assert!(script.contains("tar -xf"), "нет tar-fallback: {script}");
        // %LOCALAPPDATA% должен быть раскрыт заранее (PS его не понимает)
        // и не должен попасть в скрипт как есть
        let expected = path_service::expand_env_vars("%LOCALAPPDATA%/Programs/gradle");
        assert!(script.contains(&expected), "раскрытый install_dir в скрипте: {script}");
        assert!(!script.contains("%LOCALAPPDATA%"), "сырой %VAR% в скрипте: {script}");
        assert!(script.contains("foo.zip"), "путь к архиву в скрипте: {script}");
    }

    #[test]
    fn official_zip_without_install_dir_is_rejected() {
        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "bad-zip".to_string(),
            url: Some("https://example.com/x.zip".to_string()),
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
                    file_name: None,
        };
        let zip = std::env::temp_dir().join("tc-tool-bad.zip");
        let err = build_install_command(&source, Some(&zip), None).unwrap_err();
        assert!(err.contains("install_dir"), "ошибка: {err}");
    }

    #[test]
    fn official_msixbundle_installs_via_appx() {
        // winget (MSIX) ставится через App Installer, не «запуском»
        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "winget-msix".to_string(),
            url: Some("https://example.com/App.msixbundle".to_string()),
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
                    file_name: None,
        };
        let bundle = std::env::temp_dir().join("tc-tool-app.msixbundle");
        let cmd = build_install_command(&source, Some(&bundle), None).unwrap();
        assert_eq!(cmd.program, "powershell");
        let script = cmd.args.last().unwrap();
        assert!(script.contains("Add-AppxPackage"));
        // зависимости winget (VCLibs/WindowsAppRuntime 1.8) ставятся из
        // официального DesktopAppInstaller_Dependencies.zip до самого бандла
        assert!(script.contains("DesktopAppInstaller_Dependencies.zip"));
        assert!(script.contains("PROCESSOR_ARCHITECTURE"));
        assert!(script.contains("0x80073D06"));
        assert!(script.contains("Get-AppxPackage -Name Microsoft.DesktopAppInstaller"));
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn end_to_end_install_and_verify() {
        let def = echo_def("echo-tool");
        let mut plan = one_task_plan("echo-tool");
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();

        let secrets = execute_plan(&[def], &mut plan, trait_sink, no_abort()).await;

        assert!(secrets.is_empty());
        match &plan.tasks[0].state {
            TaskState::Success { version } => assert_eq!(version, "1.2.3"),
            other => panic!("ожидали Success, получили {other:?}"),
        }

        let events: Vec<ToolchainEventType> = sink
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.event_type.clone())
            .collect();
        assert!(events.iter().any(|e| matches!(e, ToolchainEventType::TaskStarted)));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ToolchainEventType::TaskProgress { .. })),
            "должны были стримиться строки out/err"
        );
        assert!(events.iter().any(|e| matches!(
            e,
            ToolchainEventType::TaskCompleted { state: TaskState::Success { .. } }
        )));
        assert!(events
            .iter()
            .any(|e| matches!(e, ToolchainEventType::AllCompleted { .. })));
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn failing_installer_marks_failed() {
        let mut def = echo_def("fail-tool");
        def.sources.windows[0].args = vec!["/c".to_string(), "exit".to_string(), "1".to_string()];

        let mut plan = one_task_plan("fail-tool");
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();

        execute_plan(&[def], &mut plan, trait_sink, no_abort()).await;

        assert!(matches!(plan.tasks[0].state, TaskState::Failed { .. }));
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn failed_source_falls_back_to_next() {
        // Первый источник падает (exit 1), второй — локальный cmd echo.
        let mut def = echo_def("fallback-tool");
        def.sources.windows = vec![
            InstallSource {
                kind: InstallSourceKind::Official,
                id: "bad-source".to_string(),
                url: Some("cmd.exe".to_string()),
                args: vec!["/c".to_string(), "exit".to_string(), "1".to_string()],
                extra_args: vec![],
                dynamic_args: false,
                install_dir: None,
                needs_admin: None,
                    file_name: None,
            },
            def.sources.windows[0].clone(),
        ];

        let mut plan = one_task_plan("fallback-tool");
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();

        execute_plan(&[def], &mut plan, trait_sink, no_abort()).await;

        match &plan.tasks[0].state {
            TaskState::Success { version } => assert_eq!(version, "1.2.3"),
            other => panic!("ожидали Success после fallback, получили {other:?}"),
        }

        // Пользователь должен видеть причину переключения на запасной источник
        let events: Vec<ToolchainEventType> = sink
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.event_type.clone())
            .collect();
        assert!(
            events.iter().any(|e| matches!(
                e,
                ToolchainEventType::TaskProgress { line } if line.starts_with("tc:info Источник «bad-source» не сработал")
            )),
            "нет tc:info о смене источника: {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                ToolchainEventType::TaskProgress { line } if line.starts_with("tc:ok Установлено через источник «local-cmd»")
            )),
            "нет tc:ok с источником успеха: {events:?}"
        );
    }

    #[tokio::test]
    async fn abort_before_task_marks_skipped() {
        let mut plan = one_task_plan("echo-tool");
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();
        let abort = Arc::new(AtomicBool::new(true));

        execute_plan(&[], &mut plan, trait_sink, abort).await;

        assert!(matches!(plan.tasks[0].state, TaskState::Skipped { .. }));
    }

    #[tokio::test]
    async fn unknown_tool_becomes_skipped() {
        let mut plan = one_task_plan("no-such-tool");
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();
        execute_plan(&[], &mut plan, trait_sink, no_abort()).await;
        assert!(matches!(plan.tasks[0].state, TaskState::Skipped { .. }));
    }
}
