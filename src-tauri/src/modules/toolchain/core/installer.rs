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

// ------------------------------------------------------------
// Execution Type: чем «запускать» источник
// ------------------------------------------------------------
// Раньше движок слепо вызывал Command::new(temp_path) для любого
// скачанного файла. Для git-репозиториев, .phar и скриптов это
// давало «%1 не является приложением Win32» (os error 193).
// Теперь каждый источник получает явный/вычисляемый способ
// исполнения (ExecutionKind), и команда собирается под него.

/// Определяет способ исполнения источника: явное значение из tools.json
/// (InstallSource.execution) или автоопределение по URL и расширению
/// скачанного файла.
///
/// Порядок автоопределения:
///   1. URL оканчивается на `.git`        → GitClone (flutter и т.п.);
///   2. расширение .phar                  → Phar  (composer через php);
///   3. .ps1 / .sh                        → Script (интерпретатор);
///   4. .zip/.tgz/.gz/.tar                → Archive (распаковка);
///   5. всё остальное                     → Exe (запуск напрямую).
fn resolve_execution(source: &InstallSource, offline_path: Option<&Path>) -> ExecutionKind {
    if let Some(kind) = source.execution {
        if kind != ExecutionKind::Auto {
            return kind;
        }
    }

    if let Some(url) = source.url.as_ref() {
        let tail = url.split(['?', '#']).next().unwrap_or(url).trim_end_matches('/');
        if tail.to_ascii_lowercase().ends_with(".git") {
            return ExecutionKind::GitClone;
        }
    }

    let path = offline_path.or_else(|| source.url.as_deref().map(Path::new));
    let Some(path) = path else {
        return ExecutionKind::Exe;
    };
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "phar" => ExecutionKind::Phar,
        "ps1" | "sh" => ExecutionKind::Script,
        "zip" | "tgz" | "gz" | "tar" => ExecutionKind::Archive,
        _ => ExecutionKind::Exe,
    }
}

/// Каталог, куда клонируется git-источник: явный install_dir или первый
/// известный путь без glob (для flutter — %USERPROFILE%/flutter из
/// known_paths). Хвост «/bin» срезается — клонируется корень SDK.
fn git_clone_target(def: &ToolDefinition, source: &InstallSource) -> Result<String, String> {
    if let Some(dir) = source.install_dir.as_ref() {
        return Ok(path_service::expand_env_vars(dir));
    }
    for known in &def.detection.known_paths {
        let expanded = path_service::expand_env_vars(known);
        if expanded.contains('*') {
            continue;
        }
        let p = Path::new(&expanded);
        let is_bin = p
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f.eq_ignore_ascii_case("bin"));
        if is_bin {
            if let Some(parent) = p.parent() {
                return Ok(parent.to_string_lossy().into_owned());
            }
        }
        return Ok(expanded);
    }
    Err(format!(
        "{}: git-источник требует install_dir или известный путь без glob в tools.json",
        source.id
    ))
}

/// Имена бинарников тула из проб версии (elixir → «elixir», «elixir.bat»).
/// Нужны, чтобы отличить настоящий каталог bin распакованного архива
/// от прочих папок.
fn probe_binaries(def: &ToolDefinition) -> Vec<String> {
    def.detection
        .version_probes
        .iter()
        .filter_map(|p| p.first().map(|b| b.to_ascii_lowercase()))
        .collect()
}

/// Есть ли в каталоге бинарник с одним из имён (с учётом расширений ОС).
fn contains_binary(dir: &Path, names: &[String]) -> bool {
    let Ok(listing) = std::fs::read_dir(dir) else {
        return false;
    };
    let files: Vec<String> = listing
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    if files.is_empty() {
        return false;
    }
    let exts: &[&str] = if cfg!(target_os = "windows") {
        &["", ".exe", ".bat", ".cmd"]
    } else {
        &[""]
    };
    names.iter().any(|n| {
        exts.iter().any(|e| {
            files
                .iter()
                .any(|f| f.eq_ignore_ascii_case(&format!("{n}{e}")))
        })
    })
}

/// Ищет каталог `bin` (без учёта регистра) внутри распакованного архива,
/// который содержит бинарник инструмента: elixir-otp-29/bin, gradle-9.1.0/bin
/// и т.п. Обход — в ширину по уровням (глубина ≤ 4), чтобы найти самый
/// «верхний» подходящий bin.
fn find_bin_dir(root: &Path, names: &[String]) -> Option<PathBuf> {
    let mut frontier: Vec<PathBuf> = vec![root.to_path_buf()];
    for _ in 0..4 {
        let mut next: Vec<PathBuf> = Vec::new();
        for dir in &frontier {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let is_bin = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case("bin"));
                if is_bin && contains_binary(&path, names) {
                    return Some(path);
                }
                next.push(path);
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    None
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
/// - Official/Script: тип исполнения решается через resolve_execution:
///   * GitClone (flutter.git) — `git clone <url> <каталог>`: файл не
///     качается и не запускается (иначе os error 193);
///   * Phar (.phar, composer-installer) — `php <файл> <args...>`;
///   * Script (.ps1/.sh) — powershell -File / bash;
///   * Archive (.zip/.tgz) — распаковка в install_dir;
///   * Exe — .msi через msiexec, .msix через App Installer, иначе
///     запуск напрямую (erlang-exe дополнительно получает флаги
///     тихой установки NSIS).
///
/// `password` — постгрес-пароль при dynamic_args: true.
fn build_install_command(
    def: &ToolDefinition,
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

        InstallSourceKind::Official | InstallSourceKind::Script => {
            let exec = resolve_execution(source, offline_path);

            // Git-репозиторий: не скачиваем и не запускаем —
            // клонируем напрямую. url уходит в команду как есть.
            if matches!(exec, ExecutionKind::GitClone) {
                let Some(url) = source.url.as_ref() else {
                    return Err("git-источник без url".to_string());
                };
                let target = git_clone_target(def, source)?;
                let mut args = vec!["clone".to_string()];
                // Терпим старые tools.json, где «clone» уже лежал в args.
                args.extend(source.args.iter().filter(|a| a.as_str() != "clone").cloned());
                args.push(url.clone());
                args.push(target);
                return Ok(InstallCommand {
                    program: "git".to_string(),
                    args,
                });
            }

            // Файл для исполнения: скачанный установщик, либо (Official)
            // локальный url — удобно для тестов и оффлайн-инсталляторов.
            let path: PathBuf = match (offline_path, &source.kind) {
                (Some(p), _) => p.to_path_buf(),
                (None, InstallSourceKind::Script) => {
                    return Err(format!("Скрипт {} не скачан", source.id));
                }
                (None, InstallSourceKind::Official) => {
                    let Some(url) = source.url.as_ref() else {
                        return Err("Официальная установка без url".to_string());
                    };
                    PathBuf::from(url)
                }
                _ => unreachable!(),
            };

            let mut dynamic = Vec::new();
            if source.dynamic_args {
                if let Some(pw) = password {
                    dynamic.push(format!("--superpassword {pw}"));
                    dynamic.push(format!("--password {pw}"));
                }
            }

            match exec {
                // PHP-скрипт (composer.phar и т.п.): CreateProcess его не
                // понимает («%1 не является приложением Win32», os error 193).
                // Запускаем через php, путь к файлу — первым аргументом.
                // %VAR% в аргументах (--install-dir=%APPDATA%/...) раскрываем
                // заранее: CreateProcess переменные не подставляет.
                ExecutionKind::Phar => {
                    let mut php_args = vec![path.to_string_lossy().into_owned()];
                    php_args.extend(
                        source
                            .args
                            .iter()
                            .map(|a| path_service::expand_env_vars(a)),
                    );
                    Ok(InstallCommand {
                        program: "php".to_string(),
                        args: php_args,
                    })
                }

                // Скрипты интерпретаторов: .ps1 → powershell -File,
                // .sh → bash (на Unix скрипт исполняется напрямую —
                // shebang). Прочее (.bat/.cmd) — напрямую.
                ExecutionKind::Script => {
                    let ext = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    #[cfg(target_os = "windows")]
                    {
                        if ext == "ps1" {
                            let mut args = vec![
                                "-NoProfile".to_string(),
                                "-ExecutionPolicy".to_string(),
                                "Bypass".to_string(),
                                "-File".to_string(),
                                path.to_string_lossy().into_owned(),
                            ];
                            args.extend(source.args.iter().cloned());
                            return Ok(InstallCommand {
                                program: "powershell".to_string(),
                                args,
                            });
                        }
                        if ext == "sh" {
                            let mut args = vec![path.to_string_lossy().into_owned()];
                            args.extend(source.args.iter().cloned());
                            return Ok(InstallCommand {
                                program: "bash".to_string(),
                                args,
                            });
                        }
                    }
                    let mut args = source.args.clone();
                    args.extend(dynamic);
                    Ok(InstallCommand {
                        program: path.to_string_lossy().into_owned(),
                        args,
                    })
                }

                // Архивы: распаковка в install_dir, а не запуск.
                ExecutionKind::Archive => {
                    let Some(dir) = source.install_dir.as_ref() else {
                        return Err(format!(
                            "{}: zip/tar-источник требует install_dir в tools.json",
                            source.id
                        ));
                    };
                    let dir = path_service::expand_env_vars(dir);
                    match path.extension().and_then(|e| e.to_str()) {
                        // zip (gradle, maven, elixir): распаковка без прав —
                        // в каталог из install_dir. %VAR% раскрываем здесь:
                        // PowerShell (в отличие от cmd) синтаксис %LOCALAPPDATA%
                        // не понимает.
                        //
                        // Fallback на tar: Expand-Archive падает на zip MongoDB
                        // (741МБ, битые записи PDB) — встроенный tar (Windows 10+)
                        // такие архивы распаковывает.
                        Some(ext) if ext.eq_ignore_ascii_case("zip") => {
                            let script = format!(
                                r#"$ErrorActionPreference = 'Stop'
$dir = {1}
try {{
    Expand-Archive -Path {0} -DestinationPath $dir -Force
}} catch {{
    Write-Output "tc:warn Expand-Archive не сработал ($($_.Exception.Message)) — пробуем tar"
    tar -xf {2} -C $dir
    if ($LASTEXITCODE -ne 0) {{
        Write-Output "tc:error tar -xf не смог распаковать архив"
        exit 1
    }}
}}
Get-ChildItem -Path $dir -Recurse -Include *.bat -File | ForEach-Object {{
    $t = [System.IO.File]::ReadAllText($_.FullName, [System.Text.Encoding]::UTF8)
    $t = ($t -replace "`r`n", "`n") -replace "`n", "`r`n"
    [System.IO.File]::WriteAllText($_.FullName, $t, (New-Object System.Text.UTF8Encoding $false))
}}
"#,
                                ps_quote(&path.to_string_lossy()),
                                ps_quote(&dir),
                                ps_quote(&path.to_string_lossy())
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
                        _ => {
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
                    }
                }

                // Бинарь/инсталлятор: msi → msiexec, msix → App Installer,
                // exe → запуск напрямую.
                ExecutionKind::Exe => {
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
                        // Прочие (exe): запуск напрямую.
                        _ => {
                            let mut args = source.args.clone();
                            args.extend(dynamic);
                            // NSIS-инсталлятор Erlang/OTP: без /S в неинтерактивной
                            // сессии падает с кодом 1; /v"/qn" передаёт флаги
                            // тихой установки внутреннему MSI. Оба флага
                            // принудительные, даже если их забыли в tools.json.
                            if source.id == "erlang-exe" {
                                if !args.iter().any(|a| a == "/S") {
                                    args.insert(0, "/S".to_string());
                                }
                                if !args.iter().any(|a| a.starts_with("/v")) {
                                    args.push(r#"/v"/qn""#.to_string());
                                }
                            }
                            Ok(InstallCommand {
                                program: path.to_string_lossy().into_owned(),
                                args,
                            })
                        }
                    }
                }

                ExecutionKind::GitClone | ExecutionKind::Auto => {
                    unreachable!("resolve_execution всегда возвращает конкретный тип")
                }
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
///
/// Fallback тихий: причина сбоя промежуточного источника уходит
/// только в DEBUG-лог (stdout пользователя не пугаем). Единственное
/// сообщение об ошибке шлётся в UI, только когда НЕ сработали вообще
/// все источники, — тогда одним tc:error со списком причин.
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

    let mut failures: Vec<String> = Vec::new();
    for source in os_sources.iter() {
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
                // Тихий fallback: причина сбоя промежуточного источника —
                // только в DEBUG-лог, в UI не уходит (там потом будет
                // tc:ok, если запасной источник сработает).
                debug_log(&format!(
                    "[toolchain] источник `{}` для {tool_id} не сработал: {e}",
                    source.id
                ));
                failures.push(format!("«{}»: {e}", source.id));
            }
        }
        if abort.load(Ordering::SeqCst) {
            return TaskState::Skipped {
                reason: "Отменено пользователем".to_string(),
            };
        }
    }

    // Все источники исчерпаны — только теперь одно сообщение об ошибке
    // со списком причин (а не спам после каждого упавшего источника).
    let reason = failures.join("; ");
    sink.emit(console::event(
        ToolchainEventType::TaskProgress {
            line: format!(
                "tc:error Не удалось установить «{}» ни одним из {} источников: {reason}",
                def.display,
                os_sources.len()
            ),
        },
        index,
        total,
        &task_id,
        &tool_id,
    ));

    TaskState::Failed { error: reason }
}

/// DEBUG-лог установки: пишется в stderr только при DEVLAUNCHER_DEBUG=1.
/// Промежуточные сбои источников при fallback живут здесь — пользователь
/// в UI видит только итоговый результат (tc:ok / tc:error), а не каждый
/// неудавшийся способ установки.
fn debug_log(msg: &str) {
    if std::env::var("DEVLAUNCHER_DEBUG").is_ok() {
        eprintln!("{msg}");
    }
}

/// `dotnet workload install maui` — обязательный шаг для MAUI-проектов:
/// сам SDK не даёт шаблон `dotnet new maui` («Не найдены шаблоны...»).
/// Запускается сразу после подтверждённой установки SDK. Сбой не
/// валит установку dotnet: workload повторится при следующей проверке
/// окружения (см. check.rs::ensure_maui_workload).
async fn install_maui_workload(
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    sink: &Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
) {
    let maui_args = vec!["workload".to_string(), "install".to_string(), "maui".to_string()];
    sink.emit(console::event(
        ToolchainEventType::TaskProgress {
            line: "tc:info dotnet workload install maui (шаблоны .NET MAUI)".to_string(),
        },
        index,
        total,
        task_id,
        tool_id,
    ));
    match console::piped_run(
        "dotnet",
        &maui_args,
        index,
        total,
        task_id,
        tool_id,
        sink,
        abort,
    )
    .await
    {
        Ok(res) if res.success => {
            sink.emit(console::event(
                ToolchainEventType::TaskProgress {
                    line: "tc:ok .NET MAUI workload установлен".to_string(),
                },
                index,
                total,
                task_id,
                tool_id,
            ));
        }
        Ok(res) => {
            sink.emit(console::event(
                ToolchainEventType::TaskProgress {
                    line: format!(
                        "tc:warn dotnet workload install maui не завершился (код {}) — шаблоны MAUI могут быть недоступны",
                        res.code
                    ),
                },
                index,
                total,
                task_id,
                tool_id,
            ));
        }
        Err(e) => {
            sink.emit(console::event(
                ToolchainEventType::TaskProgress {
                    line: format!("tc:warn dotnet workload install maui: {e}"),
                },
                index,
                total,
                task_id,
                tool_id,
            ));
        }
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

    // Тип исполнения решается ДО скачивания: git-репозитории (flutter.git)
    // в temp-файл не качаются — нечего «запускать» (os error 193),
    // они клонируются напрямую в целевой каталог.
    let exec = resolve_execution(source, None);
    let mut skip_run = false;

    if matches!(exec, ExecutionKind::GitClone) {
        let target = PathBuf::from(git_clone_target(def, source)?);
        if target.join(".git").is_dir() {
            sink.emit(console::event(
                ToolchainEventType::TaskProgress {
                    line: format!(
                        "tc:info {} уже склонирован ({}) — проверяю установку",
                        def.display,
                        target.display()
                    ),
                },
                index,
                total,
                task_id,
                tool_id,
            ));
            skip_run = true;
        } else {
            if target.exists() {
                sink.emit(console::event(
                    ToolchainEventType::TaskProgress {
                        line: format!(
                            "tc:warn каталог {} существует без .git — удаляю и клонирую заново",
                            target.display()
                        ),
                    },
                    index,
                    total,
                    task_id,
                    tool_id,
                ));
                std::fs::remove_dir_all(&target).map_err(|e| {
                    format!(
                        "Не удалось очистить каталог {}: {e}",
                        target.display()
                    )
                })?;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    format!("Не удалось создать каталог {}: {e}", parent.display())
                })?;
            }
        }
    } else if matches!(source.kind, InstallSourceKind::Official | InstallSourceKind::Script) {
        // Источники с http-URL качаем заранее (фаза Downloading, с прогрессом).
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

    let cmd = build_install_command(def, source, offline_path.as_deref(), password.as_deref())?;

    if !skip_run {
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
    }

    // PATH: установщик (winget/msi/exe) написал свои каталоги в реестр,
    // но текущий процесс об этом не знает. Добавляем явные path_entries
    // из tools.json (glob `PostgreSQL/*/bin` резолвится в конкретный
    // каталог) и обновляем PATH процесса — иначе verify не найдёт
    // свежеустановленный бинарник, хотя он стоит.
    if !def.path_entries.is_empty() || matches!(exec, ExecutionKind::Archive) {
        sink.emit(console::event(
            ToolchainEventType::TaskPhaseChanged { phase: TaskPhase::UpdatingPath },
            index,
            total,
            task_id,
            tool_id,
        ));

        // Архивы (elixir-otp-29.zip и т.п.) распаковываются с вложенной
        // папкой: бинарник оказывается в <install_dir>/<pkg>/bin, а не
        // в самом install_dir. Ищем конкретный каталог bin с бинарником
        // тула и добавляем ИМЕННО его в PATH — glob-запись `*/bin` из
        // tools.json на него завязана.
        if matches!(exec, ExecutionKind::Archive) {
            if let Some(install_dir) = source.install_dir.as_ref() {
                let root = path_service::expand_env_vars(install_dir);
                if let Some(bin) = find_bin_dir(Path::new(&root), &probe_binaries(def)) {
                    let bin_str = bin.to_string_lossy().into_owned();
                    if let Err(e) = path_service::add_to_user_path(&[bin_str]).await {
                        eprintln!("[toolchain] не удалось добавить {bin:?} в PATH для {tool_id}: {e}");
                    }
                }
            }
        }

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
        ToolStatus::Installed { version } => {
            // .NET MAUI: SDK сам по себе не даёт шаблон `dotnet new maui` —
            // нужен workload. Ставим сразу после подтверждённой установки.
            if def.id == "dotnet" {
                install_maui_workload(index, total, task_id, tool_id, sink, Arc::clone(abort)).await;
            }
            Ok((version, password))
        }
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
                    execution: None,
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

    /// «Голое» определение без правил обнаружения — для тестов
    /// build_install_command, которым нужен только параметр def.
    fn bare_def() -> ToolDefinition {
        ToolDefinition {
            id: "bare".to_string(),
            category: "utility".to_string(),
            display: "Bare".to_string(),
            description: "тестовый инструмент".to_string(),
            icon: None,
            detection: DetectionRules {
                version_probes: vec![],
                known_paths: vec![],
                registry_keys: vec![],
            },
            versions: Default::default(),
            sources: InstallSources::default(),
            size_mb: 0,
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
                icon: None,
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
        let cmd = build_install_command(&git, source, None, None).unwrap();

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
        let cmd = build_install_command(&pg, source, None, Some("0123456789abcdef")).unwrap();
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
                    execution: None,
        };
        let cmd = build_install_command(&bare_def(), &source, None, None).unwrap();
        assert_eq!(cmd.program, "C:/Tools/setup.exe");
        assert_eq!(cmd.args, vec!["--quiet".to_string()]);
    }

    #[test]
    fn erlang_exe_forces_silent_flags() {
        // NSIS-инсталлятор Erlang/OTP: без /S в неинтерактивной сессии
        // падает с кодом 1, а /v"/qn" передаёт флаги тихой установки
        // внутреннему MSI. Оба флага добавляются принудительно, даже
        // если их забыли в tools.json.
        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "erlang-exe".to_string(),
            url: Some("https://example.com/otp_win64_29.0.5.exe".to_string()),
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: None,
        };
        let cmd = build_install_command(&bare_def(), &source, None, None).unwrap();
        assert_eq!(cmd.args[0], "/S", "/S должен идти первым: {:?}", cmd.args);
        assert!(cmd.args.iter().any(|a| a == "/S"), "нет /S: {:?}", cmd.args);
        assert!(
            cmd.args.iter().any(|a| a.starts_with("/v")),
            "нет /v флага для внутреннего MSI: {:?}",
            cmd.args
        );
        assert_eq!(cmd.args.last().unwrap(), r#"/v"/qn""#);

        // /S и /v уже в tools.json — не должно быть дублей.
        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "erlang-exe".to_string(),
            url: Some("https://example.com/otp_win64_29.0.5.exe".to_string()),
            args: vec!["/S".to_string(), r#"/v"/qn""#.to_string()],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: None,
        };
        let cmd = build_install_command(&bare_def(), &source, None, None).unwrap();
        assert_eq!(cmd.args.iter().filter(|a| *a == "/S").count(), 1);
        assert_eq!(cmd.args.iter().filter(|a| a.starts_with("/v")).count(), 1);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn official_phar_runs_via_php() {
        // composer.phar — PHP-скрипт: CreateProcess его не понимает
        // (os error 193, «%1 не является приложением Win32»). На
        // Windows команда обязана запускаться через php с путём
        // к .phar в качестве аргумента.
        let source = InstallSource {
            kind: InstallSourceKind::Script,
            id: "composer-installer".to_string(),
            url: Some("https://getcomposer.org/download/latest-stable/composer.phar".to_string()),
            args: vec!["--quiet".to_string()],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: None,
        };
        let phar = std::env::temp_dir().join("tc-tool-composer.phar");
        let cmd = build_install_command(&bare_def(), &source, Some(&phar), None).unwrap();
        assert_eq!(cmd.program, "php", "phar обязан идти через php");
        assert_eq!(cmd.args[0], phar.to_string_lossy());
        assert!(cmd.args.iter().any(|a| a == "--quiet"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn phar_args_expand_env_vars() {
        // --install-dir=%APPDATA%/Composer: CreateProcess переменные не
        // подставляет — движок обязан раскрыть их сам, иначе php получит
        // буквальный «%APPDATA%».
        let source = InstallSource {
            kind: InstallSourceKind::Script,
            id: "composer-installer".to_string(),
            url: Some("https://getcomposer.org/installer".to_string()),
            args: vec!["--install-dir=%APPDATA%/Composer".to_string()],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: Some("composer.phar".to_string()),
            execution: Some(ExecutionKind::Phar),
        };
        let phar = std::env::temp_dir().join("composer.phar");
        let cmd = build_install_command(&bare_def(), &source, Some(&phar), None).unwrap();
        assert_eq!(cmd.program, "php");
        assert!(
            !cmd.args.iter().any(|a| a.contains("%APPDATA%")),
            "%APPDATA% должен быть раскрыт: {:?}",
            cmd.args
        );
        if let Ok(appdata) = std::env::var("APPDATA") {
            assert!(
                cmd.args.iter().any(|a| a.contains(&appdata)),
                "раскрытый APPDATA в аргументах: {:?}",
                cmd.args
            );
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn script_ps1_runs_via_powershell_file() {
        let source = InstallSource {
            kind: InstallSourceKind::Script,
            id: "dotnet-install".to_string(),
            url: Some("https://dot.net/v1/dotnet-install.ps1".to_string()),
            args: vec!["-Channel".to_string(), "10.0".to_string()],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: None,
        };
        let script = std::env::temp_dir().join("tc-tool-dotnet-install.ps1");
        let cmd = build_install_command(&bare_def(), &source, Some(&script), None).unwrap();
        assert_eq!(cmd.program, "powershell");
        let file_pos = cmd.args.iter().position(|a| a == "-File").unwrap();
        assert_eq!(cmd.args[file_pos + 1], script.to_string_lossy());
        // Аргументы источника (канал/версия) НЕ должны теряться
        assert!(cmd.args.iter().any(|a| a == "-Channel"), "args: {:?}", cmd.args);
        assert!(cmd.args.iter().any(|a| a == "10.0"), "args: {:?}", cmd.args);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn script_sh_runs_via_bash() {
        // .sh-скрипт на Windows не исполняется напрямую (os error 193) —
        // обязателен вызов через bash.
        let source = InstallSource {
            kind: InstallSourceKind::Script,
            id: "setup-sh".to_string(),
            url: Some("https://example.com/setup.sh".to_string()),
            args: vec!["--silent".to_string()],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: None,
        };
        let script = std::env::temp_dir().join("tc-tool-setup.sh");
        let cmd = build_install_command(&bare_def(), &source, Some(&script), None).unwrap();
        assert_eq!(cmd.program, "bash");
        assert_eq!(cmd.args[0], script.to_string_lossy());
        assert!(cmd.args.iter().any(|a| a == "--silent"));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn script_sh_runs_directly_on_unix() {
        // На Unix .sh исполняется напрямую (shebang-строка).
        let source = InstallSource {
            kind: InstallSourceKind::Script,
            id: "setup-sh".to_string(),
            url: Some("https://example.com/setup.sh".to_string()),
            args: vec!["--silent".to_string()],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: None,
        };
        let script = std::env::temp_dir().join("tc-tool-setup.sh");
        let cmd = build_install_command(&bare_def(), &source, Some(&script), None).unwrap();
        assert_eq!(cmd.program, script.to_string_lossy());
        assert_eq!(cmd.args, vec!["--silent".to_string()]);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn flutter_git_source_clones_instead_of_running() {
        // flutter.git — репозиторий: запускать скачанный файл нельзя
        // (os error 193) — команда обязана быть `git clone <url> <target>`.
        let flutter = defs::load_definitions()
            .into_iter()
            .find(|d| d.id == "flutter")
            .unwrap();
        let source = &flutter.sources.windows[0];
        let cmd = build_install_command(&flutter, source, None, None).unwrap();

        assert_eq!(cmd.program, "git");
        assert_eq!(cmd.args[0], "clone");
        let url = source.url.clone().unwrap();
        assert!(
            cmd.args.iter().any(|a| a == &url),
            "url репозитория в команде: {:?}",
            cmd.args
        );
        let target = cmd.args.last().unwrap();
        assert!(
            Path::new(target).is_absolute(),
            "target клонирования — абсолютный путь: {target}"
        );
        assert!(
            target.to_ascii_lowercase().ends_with("flutter"),
            "target — каталог flutter: {target}"
        );
    }

    #[test]
    fn git_clone_target_prefers_install_dir() {
        let mut def = bare_def();
        def.detection.known_paths = vec!["%USERPROFILE%/flutter".to_string()];
        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "flutter-git".to_string(),
            url: Some("https://github.com/flutter/flutter.git".to_string()),
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: Some("%LOCALAPPDATA%/flutter".to_string()),
            needs_admin: None,
            file_name: None,
            execution: Some(ExecutionKind::GitClone),
        };
        let target = git_clone_target(&def, &source).unwrap();
        let expected = path_service::expand_env_vars("%LOCALAPPDATA%/flutter");
        assert_eq!(target, expected);
    }

    #[test]
    fn git_clone_target_falls_back_to_known_path() {
        let mut def = bare_def();
        def.detection.known_paths = vec!["%USERPROFILE%/flutter".to_string()];
        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "flutter-git".to_string(),
            url: Some("https://github.com/flutter/flutter.git".to_string()),
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: Some(ExecutionKind::GitClone),
        };
        let target = git_clone_target(&def, &source).unwrap();
        let expected = path_service::expand_env_vars("%USERPROFILE%/flutter");
        assert_eq!(target, expected);
    }

    #[test]
    fn git_clone_target_strips_bin_suffix() {
        let mut def = bare_def();
        def.detection.known_paths = vec!["%LOCALAPPDATA%/Programs/sdk/bin".to_string()];
        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "sdk-git".to_string(),
            url: Some("https://example.com/sdk.git".to_string()),
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: Some(ExecutionKind::GitClone),
        };
        let target = git_clone_target(&def, &source).unwrap();
        let expected = path_service::expand_env_vars("%LOCALAPPDATA%/Programs/sdk");
        assert_eq!(target, expected);
    }

    #[test]
    fn find_bin_dir_locates_packaged_bin() {
        // Имитация elixir-otp-29.zip: %TEMP%/tc-elixir-test/elixir-otp-29/bin/elixir.bat
        let root = std::env::temp_dir().join(format!("tc-elixir-test-{}", std::process::id()));
        let bin = root.join("elixir-otp-29").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("elixir.bat"), "@echo 1.20.3\r\n").unwrap();
        std::fs::write(bin.join("elixir"), "#!/bin/sh\r\n").unwrap();

        let found = find_bin_dir(&root, &["elixir".to_string(), "elixir.bat".to_string()]);
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(found, Some(bin), "должен найти вложенный bin с elixir.bat");
    }

    #[test]
    fn find_bin_dir_ignores_bin_without_tool() {
        let root = std::env::temp_dir().join(format!("tc-nobin-test-{}", std::process::id()));
        let bin = root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("something-else.exe"), "x").unwrap();

        let found = find_bin_dir(&root, &["elixir".to_string()]);
        std::fs::remove_dir_all(&root).unwrap();

        assert_eq!(found, None, "bin без бинарника тула не считается");
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
                    execution: None,
        };
        let tgz = std::env::temp_dir().join("tc-tool-kafka.tgz");
        let cmd = build_install_command(&bare_def(), &source, Some(&tgz), None).unwrap();
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
                    execution: None,
        };
        let cmd = build_install_command(&bare_def(), &source, None, None).unwrap();
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
                    execution: None,
        };
        let zip = std::env::temp_dir().join("tc-tool-foo.zip");
        let cmd = build_install_command(&bare_def(), &source, Some(&zip), None).unwrap();
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
        // .bat-обёртки (elixir и др. GitHub-архивы) нормализуются в CRLF —
        // иначе cmd их не читает
        assert!(
            script.contains("`r`n") && script.contains("*.bat"),
            "нет CRLF-нормализации bat: {script}"
        );
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
                    execution: None,
        };
        let zip = std::env::temp_dir().join("tc-tool-bad.zip");
        let err = build_install_command(&bare_def(), &source, Some(&zip), None).unwrap_err();
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
                    execution: None,
        };
        let bundle = std::env::temp_dir().join("tc-tool-app.msixbundle");
        let cmd = build_install_command(&bare_def(), &source, Some(&bundle), None).unwrap();
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
                    execution: None,
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

        // Fallback тихий: промежуточный сбой НЕ должен попадать в UI
        // (tc:info о смене источника) — там только итоговый tc:ok.
        let events: Vec<ToolchainEventType> = sink
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.event_type.clone())
            .collect();
        assert!(
            !events.iter().any(|e| matches!(
                e,
                ToolchainEventType::TaskProgress { line } if line.starts_with("tc:info Источник")
            )),
            "промежуточный сбой не должен светиться в UI: {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(
                e,
                ToolchainEventType::TaskProgress { line } if line.starts_with("tc:error")
            )),
            "tc:error при успешном fallback быть не должно: {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(
                e,
                ToolchainEventType::TaskProgress { line } if line.starts_with("tc:ok Установлено через источник «local-cmd»")
            )),
            "нет tc:ok с источником успеха: {events:?}"
        );
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn all_sources_failed_emits_single_error() {
        // Оба источника падают — в UI уходит ровно ОДНО tc:error
        // со списком причин (не спам после каждого источника).
        let mut def = echo_def("total-fail-tool");
        let bad = InstallSource {
            kind: InstallSourceKind::Official,
            id: "bad-1".to_string(),
            url: Some("cmd.exe".to_string()),
            args: vec!["/c".to_string(), "exit".to_string(), "1".to_string()],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
                file_name: None,
                execution: None,
        };
        let bad2 = InstallSource {
            kind: InstallSourceKind::Official,
            id: "bad-2".to_string(),
            url: Some("cmd.exe".to_string()),
            args: vec!["/c".to_string(), "exit".to_string(), "2".to_string()],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
                file_name: None,
                execution: None,
        };
        def.sources.windows = vec![bad, bad2];

        let mut plan = one_task_plan("total-fail-tool");
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();

        execute_plan(&[def], &mut plan, trait_sink, no_abort()).await;

        assert!(matches!(plan.tasks[0].state, TaskState::Failed { .. }));

        let events: Vec<ToolchainEventType> = sink
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.event_type.clone())
            .collect();
        let errors: Vec<&String> = events
            .iter()
            .filter_map(|e| match e {
                ToolchainEventType::TaskProgress { line } if line.starts_with("tc:error") => Some(line),
                _ => None,
            })
            .collect();
        assert_eq!(errors.len(), 1, "должно быть одно tc:error: {events:?}");
        assert!(
            errors[0].contains("«bad-1»") && errors[0].contains("«bad-2»"),
            "tc:error должен перечислить оба источника: {}",
            errors[0]
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
