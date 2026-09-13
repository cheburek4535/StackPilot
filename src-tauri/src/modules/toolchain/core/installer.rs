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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::modules::toolchain::models::*;
use crate::modules::toolchain::platforms;

use super::archive;
use super::console::{self, ps_quote, EventSink};
use super::crypto;
use super::discovery;
use super::path_service;
use super::upstream;

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
// Редакция секретов в событиях
// ------------------------------------------------------------

/// Обёртка EventSink: вырезает известные секреты (пароль БД) из строк
/// прогресса и ошибок. Установщики иногда эхом повторяют аргументы
/// командной строки (--override "--superpassword ...") — пароль не
/// должен уехать в UI-лог или события.
pub struct RedactingSink {
    inner: Arc<dyn EventSink>,
    secrets: Mutex<Vec<String>>,
}

impl RedactingSink {
    pub fn new(inner: Arc<dyn EventSink>) -> Self {
        Self {
            inner,
            secrets: Mutex::new(Vec::new()),
        }
    }

    /// Регистрирует секрет для редакции (после генерации пароля).
    pub fn register_secret(&self, value: &str) {
        if value.len() >= 8 {
            if let Ok(mut list) = self.secrets.lock() {
                list.push(value.to_string());
            }
        }
    }

    pub fn redact(&self, text: &str) -> String {
        let Ok(list) = self.secrets.lock() else {
            return text.to_string();
        };
        let mut out = text.to_string();
        for secret in list.iter() {
            if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), "***");
            }
        }
        out
    }
}

impl EventSink for RedactingSink {
    fn emit(&self, mut event: ToolchainEvent) {
        if let ToolchainEventType::TaskProgress { line } = &mut event.event_type {
            *line = self.redact(line);
        }
        if let ToolchainEventType::Error { message } = &mut event.event_type {
            *message = self.redact(message);
        }
        if let ToolchainEventType::TaskCompleted {
            state: TaskState::Failed { error },
        } = &mut event.event_type
        {
            *error = self.redact(error);
        }
        self.inner.emit(event);
    }
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

    // Шаблонный URL (url_template) участвует в определении типа наравне
    // со статичным: расширение .zip/.tgz в шаблоне — признак архива.
    let url_candidate = source.url.as_deref().or(source.url_template.as_deref());
    if let Some(url) = url_candidate {
        let tail = url
            .split(['?', '#'])
            .next()
            .unwrap_or(url)
            .trim_end_matches('/');
        if tail.to_ascii_lowercase().ends_with(".git") {
            return ExecutionKind::GitClone;
        }
    }

    let path = offline_path.or_else(|| url_candidate.map(Path::new));
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

/// Пароль БД генерируется CSPRNG (crypto::generate_db_password):
/// прежняя версия из наносекунд времени была угадываемой.
/// Ошибка энтропии — ошибка установки, а не тихий слабый пароль.
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
            // Платформенная правда: PkgManager-источник исполняется тем
            // менеджером пакетов, который есть на ЭТОЙ ОС.
            let os = platforms::current_platform().os_name();
            match os.as_str() {
                "windows" => {
                    let mut args =
                        vec!["install".to_string(), "--id".to_string(), source.id.clone()];
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
                "macos" => {
                    // Homebrew: brew install <id> [args...]
                    // Homebrew must NOT use sudo.
                    let mut args = vec!["install".to_string(), source.id.clone()];
                    args.extend(source.args.iter().cloned());
                    args.extend(source.extra_args.iter().cloned());
                    Ok(InstallCommand {
                        program: "brew".to_string(),
                        args,
                    })
                }
                "linux" => {
                    // Detect the available package manager and build command.
                    build_linux_pkg_command(source, password)
                }
                other => Err(format!(
                    "Установка через менеджер пакетов на {other} не реализована (источник «{}»)",
                    source.id
                )),
            }
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
                args.extend(
                    source
                        .args
                        .iter()
                        .filter(|a| a.as_str() != "clone")
                        .cloned(),
                );
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
                    php_args.extend(source.args.iter().map(|a| path_service::expand_env_vars(a)));
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

                // Архивы: распаковка идёт ТОЛЬКО через безопасный слой
                // archive.rs (prevalidation + traversal-safe извлечение),
                // а не через «запуск команды»: см. try_install_source.
                ExecutionKind::Archive => Err(format!(
                    "{}: архивный источник распаковывается безопасным слоем archive.rs, команда не строится",
                    source.id
                )),

                // Бинарь/инсталлятор: msi → msiexec, msix → App Installer,
                // exe → запуск напрямую.
                ExecutionKind::Exe => {
                    match path.extension().and_then(|e| e.to_str()) {
                        // MSI-пакеты (Node.js) запускаются через msiexec
                        Some(ext) if ext.eq_ignore_ascii_case("msi") => {
                            let mut args =
                                vec!["/i".to_string(), path.to_string_lossy().into_owned()];
                            args.extend(source.args.iter().cloned());
                            args.extend(dynamic);
                            // Подробный MSI-лог: при сбое тихой установки это
                            // единственный способ узнать настоящую причину.
                            // Уникальное имя + уборка вместе с заданием.
                            let log =
                                console::tracked_temp_file(&format!("{}-msi", def.id), ".log");
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
                        //
                        // Целостность зависимостей: zip качается напрямую с
                        // официального релиза winget-cli (microsoft/winget-cli),
                        // внутри — appx-пакеты, ПОДПИСАННЫЕ Microsoft: Add-AppxPackage
                        // проверяет сигнатуру при установке, поэтому подмена
                        // пакета невозможна без разрыва подписи.
                        Some(ext)
                            if ext.eq_ignore_ascii_case("msix")
                                || ext.eq_ignore_ascii_case("msixbundle") =>
                        {
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
                                // Inline -Command: политика выполнения к
                                // командам-строкам не применяется (она
                                // касается .ps1-файлов), поэтому Bypass
                                // здесь не нужен.
                                args: vec![
                                    "-NoProfile".to_string(),
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
                    //
                    // Дополнительно: /D=path (последний аргумент!) принудительно
                    // задаёт каталог установки. NSIS использует сохранённый путь
                    // из реестра, если пользователь ранее выбирал другой каталог —
                    // это приводит к тому, что known_paths/glob не совпадает и
                    // бинарник не добавляется в PATH. /D=path гарантирует
                    // предсказуемое место установки.
                    if source.id == "erlang-exe" {
                        if !args.iter().any(|a| a == "/S") {
                            args.insert(0, "/S".to_string());
                        }
                        if !args.iter().any(|a| a.starts_with("/v")) {
                            args.push(r#"/v"/qn""#.to_string());
                        }
                        // Принудительный каталог: берём из install_dir источника
                        // или дефолт C:\Program Files\erlang.
                        if !args.iter().any(|a| a.starts_with("/D=")) {
                            let install_dir = source
                                .install_dir
                                .as_deref()
                                .unwrap_or("C:\\Program Files\\erlang");
                            let expanded = path_service::expand_env_vars(install_dir);
                            args.push(format!("/D={expanded}"));
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

/// Builds a Linux package manager command by detecting available managers.
/// Priority: apt-get (Debian/Ubuntu) > dnf (Fedora/RHEL) > pacman (Arch) > zypper (openSUSE).
/// Returns an error if no supported manager is found.
///
/// Design rules:
/// - Uses direct executable invocation, not shell string concatenation.
/// - Does not use sudo invisibly; returns a clear error if elevation is needed.
/// - Supports user-local installations (e.g., `pip install --user`).
fn build_linux_pkg_command(
    source: &InstallSource,
    _password: Option<&str>,
) -> Result<InstallCommand, String> {
    // Detect which package manager is actually available on this system.
    // We probe each one; the first found wins.
    let managers: &[(&str, &[&str])] = &[
        ("apt-get", &["apt-get", "install", "-y"]),
        ("dnf", &["dnf", "install", "-y"]),
        ("pacman", &["pacman", "-S", "--noconfirm"]),
        ("zypper", &["zypper", "install", "-y"]),
    ];

    for &(manager, base_args) in managers {
        if which_exists(manager) {
            let mut args: Vec<String> = base_args.iter().map(|s| s.to_string()).collect();
            args.push(source.id.clone());
            args.extend(source.args.iter().cloned());
            args.extend(source.extra_args.iter().cloned());
            return Ok(InstallCommand {
                program: manager.to_string(),
                args,
            });
        }
    }

    Err("Нет доступного менеджера пакетов на Linux (apt-get, dnf, pacman, zypper)".to_string())
}

/// Check if an executable exists in PATH (non-blocking, no timeout).
fn which_exists(name: &str) -> bool {
    which::which(name).is_ok()
}

// ------------------------------------------------------------
// Платформенная правда
// ------------------------------------------------------------

/// Whether automatic installation is supported on the current OS.
/// Returns true for all three platforms: Windows (winget/official),
/// macOS (brew), and Linux (apt-get/dnf/pacman/zypper).
/// UI uses this to decide whether to show install buttons.
pub fn install_execution_supported() -> bool {
    matches!(
        platforms::current_platform().os_name().as_str(),
        "windows" | "linux" | "macos"
    )
}

// ------------------------------------------------------------
// Исполнение
// ------------------------------------------------------------

/// Выполняет план: обновляет состояния задач на месте и шлёт события
/// в sink. Возвращает сгенерированные секреты (tool_id → пароль БД).
/// abort — флаг отмены: установка прерывается на ближайшей задаче.
/// session_id — идентификатор установки: помечает все события, чтобы
/// «хвосты» прежней установки не смешивались с текущей.
///
/// По завершении (успех/сбой/отмена) убирает временные файлы задания.
pub async fn execute_plan(
    definitions: &[ToolDefinition],
    plan: &mut InstallPlan,
    sink: Arc<dyn EventSink>,
    abort: Arc<AtomicBool>,
    session_id: &str,
    is_update: bool,
) -> HashMap<String, String> {
    // Редакция секретов на всём пути событий задания.
    let redactor = Arc::new(RedactingSink::new(sink));
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

        let state = run_task(
            def,
            task,
            definitions,
            i,
            total,
            &redactor,
            &mut secrets,
            &abort,
            session_id,
            is_update,
        )
        .await;
        task.state = state.clone();

        redactor.emit(console::event(
            ToolchainEventType::TaskCompleted { state },
            i,
            total,
            &task_id,
            &tool_id,
            session_id,
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
    redactor.emit(console::event(
        ToolchainEventType::AllCompleted {
            success_count,
            failed,
        },
        0,
        total,
        "",
        "",
        session_id,
    ));

    // Уборка временных артефактов задания — на любом исходе.
    console::cleanup_tracked_temp_files();

    secrets
}

/// Расширение файла из URL (последний сегмент пути без query/fragment),
/// если у источника нет file_name в tools.json: winget-бандл
/// Microsoft.DesktopAppInstaller_*.msixbundle и т.п. иначе качается
/// как .bin и «запускается» напрямую (os error 193). Принимает только
/// короткие буквенно-цифровые расширения — мусор из URL отсекается.
fn url_file_extension(url: &str) -> Option<&str> {
    let tail = url.split(['?', '#']).next()?.trim_end_matches('/');
    let name = tail.rsplit('/').next()?;
    let (_, ext) = name.rsplit_once('.')?;
    if ext.is_empty() || ext.len() > 10 || !ext.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    Some(ext)
}

/// Магические байты установщиков, по которым определяется реальный тип
/// скачанного файла, когда у URL нет расширения (API-редиректы вроде
/// Adoptium) и file_name в tools.json не задан:
/// - MSI/WiX — OLE-контейнер (D0 CF 11 E0 A1 B1 1A E1);
/// - exe (NSIS, ...) — PE-заголовок MZ (4D 5A).
fn sniff_installer_extension(path: &Path) -> Option<&'static str> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut head = [0u8; 8];
    let read = file.read(&mut head).ok()?;
    let head = &head[..read];
    if head.starts_with(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]) {
        return Some("msi");
    }
    if head.starts_with(&[0x4D, 0x5A]) {
        return Some("exe");
    }
    None
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
    definitions: &[ToolDefinition],
    index: usize,
    total: usize,
    sink: &Arc<RedactingSink>,
    secrets: &mut HashMap<String, String>,
    abort: &Arc<AtomicBool>,
    session_id: &str,
    is_update: bool,
) -> TaskState {
    let tool_id = def.id.clone();
    let task_id = task.task_id.clone();
    sink.emit(console::event(
        ToolchainEventType::TaskStarted,
        index,
        total,
        &task_id,
        &tool_id,
        session_id,
    ));

    if abort.load(Ordering::SeqCst) {
        return TaskState::Skipped {
            reason: "Отменено пользователем".to_string(),
        };
    }

    // Объявленные зависимости (tools.json extended.dependencies и
    // bundled_with): установка зависимого инструмента без них либо
    // невозможна (composer без php — «Не удалось запустить php»),
    // либо остаётся сломанной. Планировщик добавляет зависимости
    // задачами ПЕРЕД зависимыми; здесь — защита от краевых случаев
    // (зависимость не установилась, пришла чужим планом): падаем
    // сразу с понятной причиной, а не после бесполезного скачивания.
    if let Err(reason) = check_declared_dependencies(def, definitions).await {
        return TaskState::Failed { error: reason };
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
        match try_install_source(
            def, source, task, index, total, &task_id, &tool_id, session_id, sink, abort, is_update,
        )
        .await
        {
            Ok((version, secret)) => {
                if let Some(pw) = secret {
                    secrets.insert(tool_id.clone(), pw.clone());
                    // Пароль регистрируется для редакции: он не должен
                    // появиться в последующих строках вывода/ошибках.
                    sink.register_secret(&pw);
                }
                sink.emit(console::event(
                    ToolchainEventType::TaskProgress {
                        line: format!(
                            "tc:ok Установлено через источник «{}» (версия {version})",
                            source.id
                        ),
                    },
                    index,
                    total,
                    &task_id,
                    &tool_id,
                    session_id,
                ));
                return TaskState::Success { version };
            }
            Err(e) => {
                // Тихий fallback: причина сбоя промежуточного источника —
                // только в DEBUG-лог, в UI не уходит (там потом будет
                // tc:ok, если запасной источник сработает).
                debug_log(&sink.redact(&format!(
                    "[toolchain] источник `{}` для {tool_id} не сработал: {e}",
                    source.id
                )));
                failures.push(format!("«{}»: {}", source.id, sink.redact(&e)));
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
        session_id,
    ));

    TaskState::Failed { error: reason }
}

/// DEBUG-лог установки: пишется в stderr только при DEVLAUNCHER_DEBUG=1.
/// Промежуточные сбои источников при fallback живут здесь — пользователь
/// в UI видит только итоговый результат (tc:ok / tc:error), а не каждый
/// неудавшийся способ установки.
fn debug_log(msg: &str) {
    if std::env::var("DEVLAUNCHER_DEBUG").is_ok() {
        log::debug!("{msg}");
    }
}

/// `dotnet workload install maui` — обязательный шаг для MAUI-проектов:
/// сам SDK не даёт шаблон `dotnet new maui` («Не найдены шаблоны...»).
/// Запускается ТОЛЬКО здесь — внутри явного задания установки, сразу
/// после подтверждённой установки SDK. Из скана окружения (check.rs)
/// этот шаг удалён: сканы не мутируют машину. Сбой не валит установку
/// dotnet: workload повторится при следующей явной установке.
async fn install_maui_workload(
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    sink: &Arc<RedactingSink>,
    abort: Arc<AtomicBool>,
) {
    let maui_args = vec![
        "workload".to_string(),
        "install".to_string(),
        "maui".to_string(),
    ];
    sink.emit(console::event(
        ToolchainEventType::TaskProgress {
            line: "tc:info dotnet workload install maui (шаблоны .NET MAUI)".to_string(),
        },
        index,
        total,
        task_id,
        tool_id,
        session_id,
    ));
    let sink_dyn: Arc<dyn EventSink> = sink.clone();
    match console::piped_run(
        "dotnet", &maui_args, index, total, task_id, tool_id, session_id, &sink_dyn, abort,
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
                session_id,
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
                session_id,
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
                session_id,
            ));
        }
    }
}

/// Обязательные строки php.ini: (имя директивы, каноническая строка).
/// extension_dir — CRITICAL: без него PHP не находит dll в ext/,
/// и composer падает с «The zip extension and unzip/7z commands
/// are both missing». Остальные — HTTPS-скачивания composer
/// (openssl, curl), zip-архивы пакетов (zip), фреймворки
/// Laravel/Symfony (mbstring) и БД (pdo_sqlite).
const PHP_INI_REQUIRED: [(&str, &str); 6] = [
    ("extension_dir", "extension_dir = \"ext\""),
    ("zip", "extension=zip"),
    ("openssl", "extension=openssl"),
    ("curl", "extension=curl"),
    ("mbstring", "extension=mbstring"),
    ("pdo_sqlite", "extension=pdo_sqlite"),
];

/// Каталог установки PHP: первый реально существующий из path_entries
/// (%LOCALAPPDATA%/Programs/php или C:/Program Files/php).
fn php_install_dir(def: &ToolDefinition) -> Option<PathBuf> {
    def.path_entries
        .iter()
        .map(|e| PathBuf::from(path_service::expand_env_vars(e)))
        .find(|p| p.is_dir())
}

/// Каталог установки PHP для потока Composer: у composer-определения нет
/// path_entries PHP, поэтому каталог ищется по известным местам установки
/// (тот же список, что path_entries php в tools.json) и по `php` из PATH.
fn php_dir_for_composer() -> Option<PathBuf> {
    let known = ["%LOCALAPPDATA%/Programs/php", "C:/Program Files/php"];
    for entry in known {
        let p = PathBuf::from(path_service::expand_env_vars(entry));
        if p.join("php.exe").is_file() {
            return Some(p);
        }
    }
    if let Ok(path) = which::which("php") {
        return path.parent().map(Path::to_path_buf);
    }
    None
}

/// Приводит строку php.ini к каноническому виду обязательной директивы.
/// Распознаёт как активные, так и закомментированные строки
/// (`;extension_dir = "ext"`, `;extension=php_zip.dll`) и нормализует
/// их (раскомментирование + каноническое имя). Возвращает None, если
/// строка не про эту директиву.
fn canonical_php_ini_line(line: &str, key: &str) -> Option<&'static str> {
    let trimmed = line.trim();
    let bare = trimmed.strip_prefix(';').unwrap_or(trimmed).trim();
    let Some((directive, value)) = bare.split_once('=') else {
        return None;
    };
    let directive = directive.trim().to_ascii_lowercase();

    if key == "extension_dir" {
        return (directive == "extension_dir").then_some(PHP_INI_REQUIRED[0].1);
    }
    if directive != "extension" {
        return None;
    }

    // extension=zip / extension=php_zip.dll / extension=zip.dll
    let raw = value.trim().to_ascii_lowercase();
    let dll = raw.strip_suffix(".dll").unwrap_or(&raw);
    let name = dll.strip_prefix("php_").unwrap_or(dll);
    if name != key {
        return None;
    }
    PHP_INI_REQUIRED
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, canonical)| *canonical)
}

/// Строгая настройка php.ini локальной установки PHP на Windows
/// (этап после подтверждённой установки). Действия:
///   1. php.ini создаётся из php.ini-development (если ещё нет);
///   2. extension_dir = "ext" — без него PHP не видит ext/*.dll;
///   3. раскомментированы extension=zip/openssl/curl/mbstring/pdo_sqlite.
/// Идемпотентно: повторные запуски ничего не меняют. Сбой не валит
/// установку PHP — php.ini поправится при следующей проверке окружения.
async fn configure_php_ini(
    def: &ToolDefinition,
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    sink: &Arc<RedactingSink>,
) {
    let Some(php_dir) = php_install_dir(def) else {
        sink.emit(console::event(
            ToolchainEventType::TaskProgress {
                line: "tc:warn php.ini: каталог PHP не найден по path_entries — расширения не включены".to_string(),
            },
            index,
            total,
            task_id,
            tool_id,
            session_id,
        ));
        return;
    };
    configure_php_ini_at(&php_dir, index, total, task_id, tool_id, session_id, sink).await;
}

/// Сама настройка php.ini по фактическому каталогу PHP — переиспользуется
/// установкой PHP (configure_php_ini) и установкой Composer (без настроенного
/// php.ini голый PHP не имеет openssl/zip, и composer-установщик падает).
async fn configure_php_ini_at(
    php_dir: &Path,
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    sink: &Arc<RedactingSink>,
) {
    let ini = php_dir.join("php.ini");
    if !ini.exists() {
        let dev = php_dir.join("php.ini-development");
        let create_result = if dev.exists() {
            std::fs::copy(&dev, &ini)
                .map(|_| ())
                .map_err(|e| e.to_string())
        } else {
            std::fs::write(&ini, "; Created by StackPilot Toolchain\r\n").map_err(|e| e.to_string())
        };
        if let Err(e) = create_result {
            sink.emit(console::event(
                ToolchainEventType::TaskProgress {
                    line: format!(
                        "tc:warn php.ini: не удалось создать из php.ini-development: {e}"
                    ),
                },
                index,
                total,
                task_id,
                tool_id,
                session_id,
            ));
            return;
        }
    }

    // Идемпотентное переписывание: каждая директива нормализуется один
    // раз (первое вхождение), отсутствующие дописываются в конец.
    let Ok(content) = std::fs::read_to_string(&ini) else {
        sink.emit(console::event(
            ToolchainEventType::TaskProgress {
                line: format!("tc:warn php.ini: не читается {}", ini.display()),
            },
            index,
            total,
            task_id,
            tool_id,
            session_id,
        ));
        return;
    };

    let mut satisfied: HashSet<&str> = HashSet::new();
    let mut out_lines: Vec<String> = Vec::with_capacity(content.lines().count() + 2);
    for line in content.lines() {
        let mut rewritten: Option<String> = None;
        for (key, _) in PHP_INI_REQUIRED {
            if satisfied.contains(key) {
                continue;
            }
            if let Some(canonical) = canonical_php_ini_line(line, key) {
                rewritten = Some(canonical.to_string());
                satisfied.insert(key);
                break;
            }
        }
        out_lines.push(rewritten.unwrap_or_else(|| line.to_string()));
    }
    let mut appended = false;
    for (key, canonical) in PHP_INI_REQUIRED {
        if !satisfied.contains(key) {
            out_lines.push(canonical.to_string());
            satisfied.insert(key);
            appended = true;
        }
    }

    let new_content = out_lines.join("\r\n") + "\r\n";
    if new_content == content {
        sink.emit(console::event(
            ToolchainEventType::TaskProgress {
                line: format!("tc:info php.ini уже настроен ({})", ini.display()),
            },
            index,
            total,
            task_id,
            tool_id,
            session_id,
        ));
        return;
    }
    if let Err(e) = std::fs::write(&ini, &new_content) {
        sink.emit(console::event(
            ToolchainEventType::TaskProgress {
                line: format!(
                    "tc:warn php.ini: не удалось записать {}: {e}",
                    ini.display()
                ),
            },
            index,
            total,
            task_id,
            tool_id,
            session_id,
        ));
        return;
    }
    sink.emit(console::event(
        ToolchainEventType::TaskProgress {
            line: if appended {
                format!(
                    "tc:ok php.ini настроен: extension_dir=ext, включены zip/openssl/curl/mbstring/pdo_sqlite ({})",
                    ini.display()
                )
            } else {
                format!("tc:ok php.ini настроен (директивы приведены к каноническому виду: {})", ini.display())
            },
        },
        index,
        total,
        task_id,
        tool_id,
        session_id,
    ));
}

/// Создаёт composer.bat-шим рядом с composer.phar: phar — не исполняемый
/// файл (CreateProcess даёт os error 193), а без .bat-шима verify не
/// найдёт команду `composer`. Работает только с phar-источником
/// (getcomposer.org/installer с --install-dir), где установщик
/// composer.bat не создаёт.
async fn ensure_composer_bat_shim(
    source: &InstallSource,
    index: usize,
    total: usize,
    task_id: &str,
    tool_id: &str,
    session_id: &str,
    sink: &Arc<RedactingSink>,
) {
    let Some(dir_arg) = source
        .args
        .iter()
        .find_map(|a| a.strip_prefix("--install-dir="))
    else {
        return;
    };
    let dir = path_service::expand_env_vars(dir_arg);
    let phar = Path::new(&dir).join("composer.phar");
    let shim = Path::new(&dir).join("composer.bat");
    if !phar.is_file() || shim.exists() {
        return;
    }
    if let Err(e) = std::fs::write(&shim, "@echo off\r\nphp \"%~dp0composer.phar\" %*\r\n") {
        sink.emit(console::event(
            ToolchainEventType::TaskProgress {
                line: format!("tc:warn composer.bat: не удалось создать шим: {e}"),
            },
            index,
            total,
            task_id,
            tool_id,
            session_id,
        ));
    }
}

/// Проверяет объявленные зависимости инструмента (extended.dependencies
/// и bundled_with из tools.json) ПЕРЕД установкой. Если зависимость не
/// установлена и не работоспособна — Err с понятной причиной: зависимый
/// инструмент всё равно не заработает, скачивать его источники бессмысленно
/// (composer без php: оба источника падают кодом 1/«program not found»).
async fn check_declared_dependencies(
    def: &ToolDefinition,
    definitions: &[ToolDefinition],
) -> Result<(), String> {
    let mut deps: Vec<&str> = Vec::new();
    if let Some(host) = def.bundled_with.as_deref() {
        deps.push(host);
    }
    deps.extend(def.extended.dependencies.iter().map(String::as_str));

    for dep_id in deps {
        let Some(dep_def) = definitions.iter().find(|d| d.id == dep_id) else {
            continue;
        };
        match discovery::detect_tool(dep_def).await {
            ToolStatus::Installed { .. } | ToolStatus::UpdateAvailable { .. } => {}
            _ => {
                return Err(format!(
                    "«{}» требует «{}», которого нет на этой машине — установите сначала «{}»",
                    def.display, dep_def.display, dep_def.display
                ));
            }
        }
    }
    Ok(())
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
    session_id: &str,
    sink: &Arc<RedactingSink>,
    abort: &Arc<AtomicBool>,
    is_update: bool,
) -> Result<(String, Option<String>), String> {
    // QtOnline (Qt из официального репозитория) — свой конвейер:
    // пакетов несколько, каждый качается и распаковывается отдельно.
    if matches!(source.kind, InstallSourceKind::QtOnline) {
        // Опции установки приходят из выбора пользователя — валидируем
        // по белому списку модулей Qt. Неизвестная опция = ошибка задачи,
        // а не молчаливое игнорирование.
        const QT_ALLOWED_OPTIONS: [&str; 4] =
            ["qt-qml", "qt-widgets", "qt-webengine", "qt-kirigami"];
        for option in &task.install_options {
            if !QT_ALLOWED_OPTIONS.contains(&option.as_str()) {
                return Err(format!(
                    "Неизвестная опция установки Qt «{option}» (допустимо: {})",
                    QT_ALLOWED_OPTIONS.join(", ")
                ));
            }
        }
        // RedactingSink передаётся как dyn EventSink — редакция секретов
        // продолжает действовать и внутри Qt-конвейера.
        let sink_dyn: Arc<dyn EventSink> = sink.clone();
        return super::qt_installer::install_qt_online(
            def,
            source,
            &task.install_options,
            index,
            total,
            task_id,
            tool_id,
            session_id,
            &sink_dyn,
            Arc::clone(abort),
        )
        .await;
    }

    // dynamic_args (PostgreSQL): пароль нужен ещё до запуска установщика.
    // CSPRNG; отказ энтропии = отказ установки (не тихий слабый пароль).
    let password = if source.dynamic_args {
        Some(crypto::generate_db_password()?)
    } else {
        None
    };
    let mut offline_path: Option<std::path::PathBuf> = None;

    // RedactingSink реализует EventSink: консольные функции принимают
    // trait-object — редакция секретов продолжается и внутри них.
    let sink_dyn: Arc<dyn EventSink> = sink.clone();

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
                session_id,
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
                    session_id,
                ));
                std::fs::remove_dir_all(&target).map_err(|e| {
                    format!("Не удалось очистить каталог {}: {e}", target.display())
                })?;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Не удалось создать каталог {}: {e}", parent.display()))?;
            }
        }
    } else if matches!(
        source.kind,
        InstallSourceKind::Official | InstallSourceKind::Script
    ) {
        // Источники с http-URL качаем заранее (фаза Downloading, с прогрессом).
        // console::download проверяет схему (https) и целостность (sha256
        // из tools.json; без суммы — честный unverified-warning).
        //
        // Динамические версии: url_template + version_resolver резолвят
        // АКТУАЛЬНУЮ версию у официального апстрима при каждой установке
        // (обход кэша) — каталог не устаревает, ссылки не гниют. Статичный
        // url остаётся страховкой, если апстрим недоступен.
        let (effective_url, effective_sha256, effective_version) = {
            let eff = upstream::effective_source_url(
                source.version_resolver.as_ref(),
                source.url_template.as_deref(),
                source.url.as_deref(),
                source.sha256.as_deref(),
            )
            .await?;
            (eff.url, eff.sha256, eff.version)
        };
        if let Some(display_version) = &effective_version {
            sink.emit(console::event(
                ToolchainEventType::TaskProgress {
                    line: format!(
                        "tc:info Актуальная версия {tool_id}: {display_version} — скачивается свежий релиз",
                    ),
                },
                index,
                total,
                task_id,
                tool_id,
                session_id,
            ));
        }
        if let Some(url) = effective_url.as_ref() {
            if url.starts_with("http://") || url.starts_with("https://") {
                sink.emit(console::event(
                    ToolchainEventType::TaskPhaseChanged {
                        phase: TaskPhase::Downloading,
                    },
                    index,
                    total,
                    task_id,
                    tool_id,
                    session_id,
                ));
                // Уникальное имя файла на каждое задание: предсказуемый
                // общий путь вида tc-{tool}-{name} можно было бы перехватить.
                // Расширение берём из file_name в tools.json, а если его
                // нет — из URL (winget-бандл .msixbundle и т.п.). Иначе
                // файл упал бы в .bin и build_install_command «запустил»
                // его напрямую (os error 193, «%1 не является приложением
                // Win32»).
                let ext = source
                    .file_name
                    .as_deref()
                    .and_then(|n| Path::new(n).extension())
                    .and_then(|e| e.to_str())
                    .map(|e| format!(".{e}"))
                    .or_else(|| url_file_extension(url).map(|e| format!(".{e}")))
                    .unwrap_or_else(|| ".bin".to_string());
                let dest = console::tracked_temp_file(tool_id, &ext);
                if let Err(e) = console::download(
                    url,
                    &dest,
                    effective_sha256.as_deref(),
                    index,
                    total,
                    task_id,
                    tool_id,
                    session_id,
                    &sink_dyn,
                    Arc::clone(abort),
                )
                .await
                {
                    let _ = std::fs::remove_file(&dest);
                    return Err(e);
                }
                offline_path = Some(dest.clone());
                // У URL без расширения (API-редиректы: Adoptium и т.п.) и без
                // file_name в tools.json файл скачивается как .bin — такой
                // «установщик» нельзя запустить напрямую (msiexec не
                // вызывается, .bin падает с кодом 1). Определяем реальный
                // тип по магическим байтам и переименовываем, чтобы
                // build_install_command построил правильную команду
                // (msi → msiexec /i, exe → запуск). Файлы с явным
                // расширением (msixbundle/msi/exe/zip) не трогаем.
                if source.file_name.is_none()
                    && dest
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("bin"))
                {
                    if let Some(sniffed) = sniff_installer_extension(&dest) {
                        let mut typed = dest.clone();
                        typed.set_extension(sniffed);
                        if typed != dest {
                            if std::fs::rename(&dest, &typed).is_ok() {
                                offline_path = Some(typed);
                            }
                        }
                    }
                }
            }
        }
    }

    // Контракт §5, правило 5: архив проверяется на traversal ДО извлечения
    // (fail-closed: одна опасная запись бракует весь архив). ЕДИНСТВЕННЫЙ
    // распаковщик — безопасный слой archive.rs (prevalidation + скрипт
    // извлечения): собственных inline-скриптов распаковки здесь нет,
    // чтобы политика безопасности не разъезжалась по файлам.
    if matches!(exec, ExecutionKind::Archive) {
        let Some(offline) = offline_path.as_deref() else {
            return Err("Архивный источник не скачан".to_string());
        };
        let Some(dir) = source.install_dir.as_ref() else {
            return Err(format!(
                "{}: zip/tar-источник требует install_dir в tools.json",
                source.id
            ));
        };
        let dir = path_service::expand_env_vars(dir);
        let is_zip = offline
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("zip"))
            .unwrap_or(false);
        let sink_dyn: Arc<dyn EventSink> = sink.clone();
        let result = if is_zip {
            archive::extract_zip_safe(
                offline,
                Path::new(&dir),
                tool_id,
                task_id,
                index,
                total,
                session_id,
                &sink_dyn,
                Arc::clone(abort),
            )
            .await
        } else {
            archive::extract_tar_safe(
                offline,
                Path::new(&dir),
                tool_id,
                task_id,
                index,
                total,
                session_id,
                &sink_dyn,
                Arc::clone(abort),
            )
            .await
        };
        result.map_err(|e| format!("Архив отклонён (безопасность): {e}"))?;
        skip_run = true;
    }

    // Composer на Windows: предусловия, без которых оба источника падают
    // кодом 1, прямо в потоке установки composer:
    //   (а) php-установщик (getcomposer.org/installer) НЕ создаёт целевой
    //       каталог сам — `--install-dir` обязан существовать («The defined
    //       install dir ... does not exist.»);
    //   (б) голый PHP без настроенного php.ini не имеет openssl/zip/curl —
    //       установщик падает с «The openssl extension is missing». Та же
    //       конфигурация, что делает установка PHP (configure_php_ini),
    //       применяется здесь для уже установленного PHP.
    if def.id == "composer" && cfg!(target_os = "windows") {
        if let Some(dir_arg) = source
            .args
            .iter()
            .find_map(|a| a.strip_prefix("--install-dir="))
        {
            let dir = path_service::expand_env_vars(dir_arg);
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return Err(format!(
                    "Не удалось создать каталог установки Composer {}: {e}",
                    dir
                ));
            }
        }
        if let Some(php_dir) = php_dir_for_composer() {
            configure_php_ini_at(&php_dir, index, total, task_id, tool_id, session_id, sink).await;
        }
    }

    // Команда исполнения. Archive-источник команды НЕ имеет: распаковка
    // уже выполнена безопасным слоем archive.rs выше (skip_run=true),
    // а build_install_command для архивов сознательно не строит «запуск»
    // (запускать распакованный архив нельзя). Вызов её здесь — ошибка
    // «команда не строится» ПРИ УСПЕШНОЙ распаковке, что и было корнем
    // сбоя zip/tgz-инструментов (kafka, maven, gradle, kotlin, zig, dart).
    let cmd = if matches!(exec, ExecutionKind::Archive) {
        None
    } else {
        Some(build_install_command(
            def,
            source,
            offline_path.as_deref(),
            password.as_deref(),
        )?)
    };

    if let Some(cmd) = cmd {
        if !skip_run {
            sink.emit(console::event(
                ToolchainEventType::TaskPhaseChanged {
                    phase: TaskPhase::Installing,
                },
                index,
                total,
                task_id,
                tool_id,
                session_id,
            ));

            // needs_admin → UAC-элевация; остальные запускаются как есть.
            // У источника может быть своё значение (zip-распаковка не требует UAC,
            // даже если у инструмента в целом needs_admin=true).
            // resolve_command: .cmd/.bat-бинари (npm) оборачивает в cmd /c.
            let needs_admin = source.needs_admin.unwrap_or(def.needs_admin);
            let (program, args) = platforms::resolve_command(&cmd.program, &cmd.args);
            let run = if needs_admin {
                console::run_elevated(
                    &program,
                    &args,
                    index,
                    total,
                    task_id,
                    tool_id,
                    session_id,
                    &sink_dyn,
                    Arc::clone(abort),
                )
                .await
            } else {
                console::piped_run(
                    &program,
                    &args,
                    index,
                    total,
                    task_id,
                    tool_id,
                    session_id,
                    &sink_dyn,
                    Arc::clone(abort),
                )
                .await
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
                    && matches!(
                        res.code,
                        WINGET_ALREADY_INSTALLED | WINGET_UPGRADE_NOT_AVAILABLE
                    );
                if !winget_already_installed {
                    // tc:error-строка из скрипта (download/run_elevated) — настоящая
                    // причина сбоя; код процесса — лишь дополнение к ней.
                    return match res.error_line {
                        Some(line) => Err(format!("{} (код {})", sink.redact(&line), res.code)),
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
                    session_id,
                ));
            }
        }
    }

    // PATH: установщик (winget/msi/exe) написал свои каталоги в реестр,
    // но текущий процесс об этом не знает. Добавляем явные path_entries
    // из tools.json (glob `PostgreSQL/*/bin` резолвится в конкретный
    // каталог) и обновляем PATH процесса — иначе verify не найдёт
    // свежеустановленный бинарник, хотя он стоит.
    //
    // PkgManager-источники (winget/brew/apt) сами управляют PATH:
    // winget пишет в реестр, brew/apt ставят в стандартные каталоги.
    // path_entries в tools.json — для Windows-инсталляторов; на
    // Linux/macOS они содержат Windows-пути, которые не проходят
    // проверку is_absolute_entry — пропускаем, чтобы не логировать
    // ложные ошибки и не показывать обновление PATH в UI.
    let is_pkg_manager = matches!(source.kind, InstallSourceKind::PkgManager);
    if (!def.path_entries.is_empty() && !is_pkg_manager) || matches!(exec, ExecutionKind::Archive) {
        sink.emit(console::event(
            ToolchainEventType::TaskPhaseChanged {
                phase: TaskPhase::UpdatingPath,
            },
            index,
            total,
            task_id,
            tool_id,
            session_id,
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
                        log::error!(
                            "[toolchain] не удалось добавить {bin:?} в PATH для {tool_id}: {e}"
                        );
                    }
                }
            }
        }

        if let Err(e) = path_service::add_to_user_path(&def.path_entries).await {
            // PATH не критичен для установки — логируем и продолжаем.
            log::error!("[toolchain] не удалось добавить PATH для {tool_id}: {e}");
        }

        // NSIS-инсталлятор Erlang/OTP: после установки добавляем реальный
        // путь bin/ в PATH. Это покрывает случай, когда:
        // 1. Пользователь previously chose нестандартный путь → NSIS запомнил
        //    его и установил туда → glob в path_entries не совпал.
        // 2. /D=path установил в предсказуемое место, но path_entries ещё
        //    не обновлён в реестре пользователя.
        // Читаем InstallLocation из реестра — это единственный надёжный
        // способ узнать реальный путь NSIS/MSI установщика.
        if def.id == "erlang" {
            if let Some(reg_bin) = discovery::registry_install_bin_path(def).await {
                if let Err(e) = path_service::add_to_user_path(&[reg_bin.clone()]).await {
                    log::error!(
                        "[toolchain] не удалось добавить реестровый путь {reg_bin} в PATH для {tool_id}: {e}"
                    );
                }
            }
        }
    }
    if let Err(e) = path_service::sync_process_path().await {
        log::error!("[toolchain] не удалось обновить PATH процесса: {e}");
    }

    // composer.phar (phar-источник) не является исполняемым файлом —
    // рядом с ним создаётся composer.bat-шим, иначе verify не найдёт
    // команду `composer` («%1 не является приложением Win32»).
    if matches!(exec, ExecutionKind::Phar) && def.id == "composer" {
        ensure_composer_bat_shim(source, index, total, task_id, tool_id, session_id, sink).await;
    }

    // Первый запуск SDK (bootstrap из tools.json): flutter после git
    // clone качает Dart SDK и строит снапшот тула МИНУТАМИ. Без этого
    // шага verify и последующие сканы упираются в таймаут пробы 10с —
    // «Установка не подтвердилась» при реально установленном SDK.
    // Выполняется стримингом (piped_run): прогресс виден, отмена жива,
    // искусственного таймаута нет — в отличие от проб скана.
    if let Some(bootstrap) = source.bootstrap.as_deref() {
        if bootstrap.is_empty() {
            return Err(format!("{}: bootstrap в tools.json пустой", def.id));
        }
        sink.emit(console::event(
            ToolchainEventType::TaskPhaseChanged {
                phase: TaskPhase::Installing,
            },
            index,
            total,
            task_id,
            tool_id,
            session_id,
        ));
        sink.emit(console::event(
            ToolchainEventType::TaskProgress {
                line: format!(
                    "tc:info {}: первый запуск SDK (`{}`) — инициализация может занять несколько минут",
                    def.display,
                    bootstrap.join(" ")
                ),
            },
            index,
            total,
            task_id,
            tool_id,
            session_id,
        ));
        let (program, args) = platforms::resolve_command(&bootstrap[0], &bootstrap[1..]);
        let run = console::piped_run(
            &program,
            &args,
            index,
            total,
            task_id,
            tool_id,
            session_id,
            &sink_dyn,
            Arc::clone(abort),
        )
        .await;
        let res = match run {
            Ok(r) => r,
            Err(e) => return Err(e),
        };
        if res.aborted {
            return Err("Отменено пользователем".to_string());
        }
        if !res.success {
            return match res.error_line {
                Some(line) => Err(format!(
                    "Инициализация {} не завершилась: {} (код {})",
                    def.display,
                    sink.redact(&line),
                    res.code
                )),
                None => Err(format!(
                    "Инициализация {} не завершилась (код {})",
                    def.display, res.code
                )),
            };
        }
        sink.emit(console::event(
            ToolchainEventType::TaskProgress {
                line: format!("tc:ok {}: первый запуск SDK завершён", def.display),
            },
            index,
            total,
            task_id,
            tool_id,
            session_id,
        ));
    }

    // Проверка: пересканируем инструмент тем же discovery. Проба
    // known_paths умеет находить бинарь и без PATH (postgres).
    sink.emit(console::event(
        ToolchainEventType::TaskPhaseChanged {
            phase: TaskPhase::Verifying,
        },
        index,
        total,
        task_id,
        tool_id,
        session_id,
    ));
    match discovery::detect_tool(def).await {
        ToolStatus::Installed { version } => {
            // .NET MAUI: SDK сам по себе не даёт шаблон `dotnet new maui` —
            // нужен workload. Ставим сразу после подтверждённой установки.
            if def.id == "dotnet" {
                install_maui_workload(
                    index,
                    total,
                    task_id,
                    tool_id,
                    session_id,
                    sink,
                    Arc::clone(abort),
                )
                .await;
            }
            // PHP на Windows: без настроенного php.ini composer падает
            // («The zip extension and unzip/7z commands are both missing»).
            // Строгая конфигурация после подтверждённой установки.
            if def.id == "php" {
                configure_php_ini(def, index, total, task_id, tool_id, session_id, sink).await;
            }
            Ok((version, password))
        }
        // Инструмент установлен и работает, но версия ниже рекомендуемой
        // (например winget поставил .NET SDK 8, а recommended — 10).
        // Для установки это УСПЕХ: цель «инструмент есть» достигнута,
        // совет обновить версию даст скан окружения. Для ОБНОВЛЕНИЯ это
        // провал источника: цель «достичь рекомендуемой версии» не
        // достигнута — следующий источник (dotnet-install с каналом 10.0)
        // получает шанс довести версию до рекомендуемой.
        ToolStatus::UpdateAvailable { installed, .. } => {
            if is_update {
                Err(format!(
                    "Установлена версия {installed}, рекомендуемая {} не достигнута",
                    def.versions.recommended.as_deref().unwrap_or("—")
                ))
            } else {
                Ok((installed, password))
            }
        }
        // Установка найдена, но бинарь не отвечает: честная причина
        // вместо обобщённого «инструмент не найден».
        ToolStatus::PathBroken { reason } => {
            Err(format!("Установка не подтвердилась: {reason}"))
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
                ..Default::default()
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
                    bootstrap: None,
                    sha256: None,
                    url_template: None,
                    version_resolver: None,
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
            extended: Default::default(),
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
                ..Default::default()
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
            extended: Default::default(),
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
            session_id: "s-test".to_string(),
        }
    }

    #[test]
    fn password_comes_from_csprng() {
        // Новый генератор возвращает Result и алфавит без опасных символов.
        let pw = crypto::generate_db_password().unwrap();
        assert_eq!(pw.len(), 16);
        assert!(!pw.chars().any(|c| "\"'`:;$\\ \0".contains(c)));
        let second = crypto::generate_db_password().unwrap();
        assert_ne!(pw, second, "два вызова CSPRNG не должны совпадать");
    }

    #[test]
    fn redacting_sink_hides_registered_secret() {
        let inner = Arc::new(TestSink::default());
        let sink = Arc::new(RedactingSink::new(inner.clone() as Arc<dyn EventSink>));
        sink.register_secret("TopSecretPass1");

        sink.emit(console::event(
            ToolchainEventType::TaskProgress {
                line: "winget --override \"--superpassword TopSecretPass1\"".to_string(),
            },
            0,
            1,
            "t",
            "tool",
            "s-1",
        ));
        let events = inner.events.lock().unwrap();
        match &events[0].event_type {
            ToolchainEventType::TaskProgress { line } => {
                assert!(
                    !line.contains("TopSecretPass1"),
                    "секрет утёк в событие: {line}"
                );
                assert!(line.contains("***"), "ожидаем редакцию: {line}");
            }
            other => panic!("не тот тип события: {other:?}"),
        }
    }

    #[test]
    fn redaction_ignores_short_values() {
        let inner = Arc::new(TestSink::default());
        let sink = Arc::new(RedactingSink::new(inner.clone() as Arc<dyn EventSink>));
        sink.register_secret("abc");
        assert_eq!(
            sink.redact("abc def"),
            "abc def",
            "короткие значения не редактируются"
        );
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
        assert!(cmd
            .args
            .windows(2)
            .any(|w| w[0] == "install" && w[1] == "--id"));
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
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
        // Дополнительно: /D=path (последний аргумент!) принудительно
        // задаёт каталог установки, чтобы NSIS не использовал сохранённый
        // из реестра путь (пользователь previously chose нестандартный).
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
        };
        let cmd = build_install_command(&bare_def(), &source, None, None).unwrap();
        assert_eq!(cmd.args[0], "/S", "/S должен идти первым: {:?}", cmd.args);
        assert!(cmd.args.iter().any(|a| a == "/S"), "нет /S: {:?}", cmd.args);
        assert!(
            cmd.args.iter().any(|a| a.starts_with("/v")),
            "нет /v флага для внутреннего MSI: {:?}",
            cmd.args
        );
        // /D=path — последний аргумент, принудительно задаёт каталог
        let d_arg = cmd.args.iter().find(|a| a.starts_with("/D="));
        assert!(
            d_arg.is_some(),
            "нет /D= для каталога установки: {:?}",
            cmd.args
        );
        // /D=path должен быть ПОСЛЕДНИМ аргументом (требование NSIS)
        assert_eq!(
            cmd.args.last().unwrap(),
            d_arg.unwrap(),
            "/D= должен быть последним аргументом NSIS: {:?}",
            cmd.args
        );

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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
        };
        let cmd = build_install_command(&bare_def(), &source, None, None).unwrap();
        assert_eq!(cmd.args.iter().filter(|a| *a == "/S").count(), 1);
        assert_eq!(cmd.args.iter().filter(|a| a.starts_with("/v")).count(), 1);
        // /D= добавляется даже когда /S и /v уже есть
        assert!(cmd.args.iter().any(|a| a.starts_with("/D=")));
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
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
    fn php_ini_canonical_line_uncomments() {
        // Закомментированные директивы php.ini-development приводятся
        // к каноническому активному виду.
        assert_eq!(
            canonical_php_ini_line(";extension_dir = \"ext\"", "extension_dir"),
            Some("extension_dir = \"ext\"")
        );
        assert_eq!(
            canonical_php_ini_line(";extension=zip", "zip"),
            Some("extension=zip")
        );
        assert_eq!(
            canonical_php_ini_line(";extension=php_openssl.dll", "openssl"),
            Some("extension=openssl")
        );
        assert_eq!(
            canonical_php_ini_line("extension=mbstring", "mbstring"),
            Some("extension=mbstring")
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn php_ini_canonical_line_ignores_others() {
        // Чужие строки и чужие расширения не трогаются.
        assert_eq!(canonical_php_ini_line(";extension=gd", "zip"), None);
        assert_eq!(
            canonical_php_ini_line(";error_log = php_errors.log", "zip"),
            None
        );
        assert_eq!(canonical_php_ini_line("[PHP]", "extension_dir"), None);
        assert_eq!(canonical_php_ini_line("", "zip"), None);
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn configure_php_ini_creates_and_enables_extensions() {
        // php.ini создаётся из php.ini-development, extension_dir и
        // обязательные расширения включаются (раскомментирование),
        // посторонние строки сохраняются.
        let dir = std::env::temp_dir().join(format!("tc-php-ini-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("php.ini-development"),
            "; Start\r\n;extension_dir = \"ext\"\r\nextension=gd\r\n;extension=zip\r\n;extension=curl\r\n[PHP]\r\n; End\r\n",
        )
        .unwrap();

        let mut def = bare_def();
        def.id = "php".to_string();
        def.path_entries = vec![dir.to_string_lossy().into_owned()];

        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();
        let redactor = Arc::new(RedactingSink::new(trait_sink));
        configure_php_ini(&def, 0, 1, "t", "php", "test-session", &redactor).await;

        let ini = std::fs::read_to_string(dir.join("php.ini")).unwrap();
        assert!(
            ini.contains("extension_dir = \"ext\""),
            "extension_dir: {ini}"
        );
        assert!(ini.contains("extension=zip"), "zip: {ini}");
        assert!(ini.contains("extension=openssl"), "openssl: {ini}");
        assert!(ini.contains("extension=curl"), "curl: {ini}");
        assert!(ini.contains("extension=mbstring"), "mbstring: {ini}");
        assert!(ini.contains("extension=pdo_sqlite"), "pdo_sqlite: {ini}");
        assert!(ini.contains("extension=gd"), "gd сохранился: {ini}");
        assert!(
            !ini.contains(";extension="),
            "нет закомментированных: {ini}"
        );

        // Идемпотентность: повторный запуск не меняет содержимое.
        configure_php_ini(&def, 0, 1, "t", "php", "test-session", &redactor).await;
        let again = std::fs::read_to_string(dir.join("php.ini")).unwrap();
        assert_eq!(again, ini, "повторный прогон обязан быть идемпотентным");

        let _ = std::fs::remove_dir_all(&dir);
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
        };
        let script = std::env::temp_dir().join("tc-tool-dotnet-install.ps1");
        let cmd = build_install_command(&bare_def(), &source, Some(&script), None).unwrap();
        assert_eq!(cmd.program, "powershell");
        let file_pos = cmd.args.iter().position(|a| a == "-File").unwrap();
        assert_eq!(cmd.args[file_pos + 1], script.to_string_lossy());
        // Аргументы источника (канал/версия) НЕ должны теряться
        assert!(
            cmd.args.iter().any(|a| a == "-Channel"),
            "args: {:?}",
            cmd.args
        );
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
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

    /// Регрессия каталога: known_paths обязаны указывать на КАТАЛОГ
    /// БИНАРЯ (bin/), а не на корень SDK — иначе пробы known_paths
    /// дают только след вместо рабочей улики, и PATH-диагностика
    /// молчит. Плюс: git_clone_target при этом всё равно срезает bin
    /// и клонирует КОРЕНЬ SDK.
    #[cfg(target_os = "windows")]
    #[test]
    fn flutter_known_paths_point_at_bin_and_clone_target_stays_root() {
        let flutter = defs::load_definitions()
            .into_iter()
            .find(|d| d.id == "flutter")
            .unwrap();

        let expanded = path_service::expand_env_vars(&flutter.detection.known_paths[0]);
        assert!(
            Path::new(&expanded)
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case("bin")),
            "known_paths обязан указывать на bin/: {expanded}"
        );

        let source = &flutter.sources.windows[0];
        let target = git_clone_target(&flutter, source).unwrap();
        assert!(
            Path::new(&target)
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| !n.eq_ignore_ascii_case("bin")),
            "клонирование идёт в корень SDK, а не в bin: {target}"
        );
    }

    /// Регрессия каталога: у git-источника flutter объявлен bootstrap
    /// (первый запуск SDK качает Dart SDK минутами — без него verify
    /// упирается в таймаут пробы 10с и ложно сообщает «не подтвердилось»).
    #[test]
    fn flutter_git_source_declares_first_run_bootstrap() {
        let flutter = defs::load_definitions()
            .into_iter()
            .find(|d| d.id == "flutter")
            .unwrap();
        let source = &flutter.sources.windows[0];
        assert_eq!(source.execution, Some(ExecutionKind::GitClone));
        let bootstrap = source.bootstrap.as_deref().expect("bootstrap объявлен");
        assert_eq!(bootstrap, &["flutter".to_string(), "--version".to_string()]);
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
        };
        let target = git_clone_target(&def, &source).unwrap();
        let expected = path_service::expand_env_vars("%USERPROFILE%/flutter");
        assert_eq!(target, expected);
    }

    /// Регрессия Java: каталог установки Temurin версионированный
    /// (jdk-21.0.x-hotspot), поэтому known_paths/path_entries обязаны быть
    /// glob-шаблоном `*/bin`, а не жёсткой версией — иначе winget-установка
    /// «не подтверждается» (детект ищет бинарь в несуществующем каталоге),
    /// а PATH после MSI не обновляется. Официальный источник (Adoptium
    /// API-редирект без расширения в URL) обязан декларировать file_name,
    /// иначе файл качается как .bin и запускается напрямую вместо msiexec.
    #[test]
    fn java_temurin_defines_glob_paths_and_msi_file_name() {
        let java = defs::load_definitions()
            .into_iter()
            .find(|d| d.id == "java")
            .expect("java в tools.json");

        assert_eq!(java.detection.known_paths, vec!["C:/Program Files/Eclipse Adoptium/*/bin"]);
        assert_eq!(
            java.path_entries,
            vec!["C:\\Program Files\\Eclipse Adoptium\\*\\bin"]
        );

        let msi = java
            .sources
            .windows
            .iter()
            .find(|s| s.id == "temurin-msi")
            .expect("temurin-msi источник");
        assert_eq!(
            msi.file_name.as_deref(),
            Some("temurin-jdk.msi"),
            "файл без расширения в URL качается как .bin и не попадает в msiexec"
        );

        // Собранная команда обязана идти через msiexec /i, а не запускать
        // .bin напрямую.
        let cmd = build_install_command(
            &java,
            msi,
            Some(Path::new("C:\\temp\\tc-java-1.msi")),
            None,
        )
        .unwrap();
        assert_eq!(cmd.program.to_ascii_lowercase(), "msiexec");
        assert_eq!(cmd.args[0], "/i");
    }

    /// Установщик, скачанный как .bin (нет file_name и расширения в URL),
    /// распознаётся по магическим байтам: MSI — OLE-контейнер, exe — MZ.
    #[test]
    fn sniff_installer_extension_detects_msi_and_exe() {
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_tc_sniff_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let msi = dir.join("x.bin");
        std::fs::write(&msi, [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0x00]).unwrap();
        assert_eq!(sniff_installer_extension(&msi), Some("msi"));

        let exe = dir.join("y.bin");
        std::fs::write(&exe, [0x4D, 0x5A, 0x90, 0x00, 0x03]).unwrap();
        assert_eq!(sniff_installer_extension(&exe), Some("exe"));

        let garbage = dir.join("z.bin");
        std::fs::write(&garbage, b"PK\x03\x04 not an installer").unwrap();
        assert_eq!(sniff_installer_extension(&garbage), None);

        assert_eq!(sniff_installer_extension(&dir.join("missing.bin")), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Регрессия «winget-msix: Не удалось запустить ...tc-winget-*.bin:
    /// %1 не является приложением Win32 (os error 193)»: у URL с явным
    /// расширением (.msixbundle) расширение обязано браться из URL,
    /// когда file_name в tools.json не задан.
    #[test]
    fn url_extension_is_derived_from_url_path() {
        assert_eq!(
            url_file_extension(
                "https://github.com/microsoft/winget-cli/releases/download/v1.29.280/Microsoft.DesktopAppInstaller_8wekyb3d8bbwe.msixbundle"
            ),
            Some("msixbundle")
        );
        assert_eq!(
            url_file_extension("https://example.com/download/file.msi?token=abc&x=1"),
            Some("msi")
        );
        assert_eq!(url_file_extension("https://example.com/api/v3/binary/"), None);
        assert_eq!(url_file_extension("https://example.com/releases/latest"), None);
        assert_eq!(url_file_extension("https://example.com/archive.tar.gz"), Some("gz"));
        assert_eq!(
            url_file_extension("https://example.com/file.with.dots.bundle"),
            Some("bundle")
        );
        // Мусорные «расширения» из URL не пропускаются: файл уйдёт в .bin
        // и попадёт в sniff-слой (msi/exe по магическим байтам).
        assert_eq!(url_file_extension("https://example.com/file.asdf_12345678901"), None);
        assert_eq!(url_file_extension("https://example.com/file."), None);
        assert_eq!(url_file_extension("https://example.com/file"), None);
    }

    /// Скачанный бандл winget (URL без file_name в tools.json) обязан
    /// получить расширение .msixbundle и уйти в Add-AppxPackage, а не
    /// «запускаться» как .bin.
    #[test]
    fn winget_msixbundle_url_keeps_extension_and_installs_via_appx() {
        let def = defs::load_definitions()
            .into_iter()
            .find(|d| d.id == "winget")
            .expect("winget в tools.json");
        let msix = def
            .sources
            .windows
            .iter()
            .find(|s| s.id == "winget-msix")
            .expect("winget-msix источник");
        assert!(
            msix.file_name.is_none(),
            "расширение должно браться из URL, file_name не нужен"
        );

        let url = msix.url.as_deref().expect("url у winget-msix");
        let ext = url_file_extension(url).expect("у URL msixbundle-бандла есть расширение");
        assert_eq!(ext, "msixbundle");

        // Команда для скачанного файла с правильным расширением —
        // PowerShell с Add-AppxPackage, не прямой запуск.
        let bundle = std::env::temp_dir().join(format!("tc-winget-0-{}.msixbundle", std::process::id()));
        let cmd = build_install_command(&def, msix, Some(&bundle), None).unwrap();
        assert_eq!(cmd.program, "powershell");
        let script = cmd.args.last().unwrap();
        assert!(script.contains("Add-AppxPackage"));
        assert!(script.contains(&bundle.to_string_lossy().into_owned()));
    }

    /// Скачанный .bin с MSI-содержимым обязан переименовываться в .msi,
    /// чтобы build_install_command построил команду msiexec — регрессия
    /// «temurin-msi: Установщик завершился с кодом 1».
    #[test]
    fn downloaded_msi_bin_is_routed_through_msiexec() {
        let def = bare_def();
        let dir = std::env::temp_dir().join(format!(
            "stackpilot_tc_msiroute_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("tc-java-1.bin");
        std::fs::write(&bin, [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0x00]).unwrap();

        let msi_path = if let Some(ext) = sniff_installer_extension(&bin) {
            let mut typed = bin.clone();
            typed.set_extension(ext);
            assert_eq!(
                typed.extension().and_then(|e| e.to_str()),
                Some("msi"),
                "sniff должен вернуть msi-расширение"
            );
            typed
        } else {
            bin.clone()
        };

        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "temurin-msi".to_string(),
            url: Some("https://api.adoptium.net/v3/binary/...".to_string()),
            args: vec!["/quiet".to_string()],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: None,
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
        };
        let cmd = build_install_command(&def, &source, Some(&msi_path), None).unwrap();
        assert_eq!(cmd.program.to_ascii_lowercase(), "msiexec");
        assert!(cmd.args.iter().any(|a| a == "/i"), "{:?}", cmd.args);
        assert!(cmd.args.iter().any(|a| a.contains("tc-java-1.msi")));

        // Без sniffing (.bin как есть) команда шла бы «запустить .bin»
        // напрямую — именно это падало с кодом 1.
        let raw_cmd = build_install_command(&def, &source, Some(&bin), None).unwrap();
        assert_ne!(raw_cmd.program.to_ascii_lowercase(), "msiexec");

        let _ = std::fs::remove_dir_all(&dir);
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
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
    fn archive_source_rejects_command_building() {
        // Архивные источники распаковываются ТОЛЬКО безопасным слоем
        // archive.rs (prevalidation + extract_*_safe). Строитель команд
        // обязан честно отказаться — скрипты распаковки здесь не живут
        // (политика безопасности в одном месте).
        let source = InstallSource {
            kind: InstallSourceKind::Official,
            id: "gradle-zip".to_string(),
            url: Some("https://example.com/gradle.zip".to_string()),
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: Some("%LOCALAPPDATA%/Programs/gradle".to_string()),
            needs_admin: None,
            file_name: None,
            execution: None,
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
        };
        let zip = std::env::temp_dir().join("tc-tool-gradle.zip");
        let err = build_install_command(&bare_def(), &source, Some(&zip), None).unwrap_err();
        assert!(
            err.contains("archive.rs"),
            "архив обязан указывать на безопасный распаковщик: {err}"
        );
    }

    /// Зависимость (extended.dependencies: composer→php), которой нет на
    /// машине, блокирует задачу ПЕРЕД скачиванием: установка зависимого
    /// инструмента без неё невозможна (composer без php — «program not
    /// found»), и пользователь получает понятную причину, а не каскад
    /// ошибок установщика.
    #[tokio::test]
    async fn declared_dependency_missing_blocks_task() {
        let mut php = bare_def();
        php.id = "php".to_string();
        php.display = "PHP".to_string();
        let mut composer = bare_def();
        composer.id = "composer".to_string();
        composer.display = "Composer".to_string();
        composer.extended.dependencies = vec!["php".to_string()];

        let err = check_declared_dependencies(&composer, &[php, composer.clone()])
            .await
            .unwrap_err();
        assert!(
            err.contains("PHP") && err.contains("Composer"),
            "причина должна называть зависимого и зависимость: {err}"
        );
    }

    /// Установленная зависимость (Installed) проверку проходит.
    #[tokio::test]
    async fn declared_dependency_present_passes() {
        let mut php = echo_def("php");
        php.display = "PHP".to_string();
        let mut composer = bare_def();
        composer.id = "composer".to_string();
        composer.display = "Composer".to_string();
        composer.extended.dependencies = vec!["php".to_string()];

        check_declared_dependencies(&composer, &[php, composer.clone()])
            .await
            .unwrap();
    }

    /// Неработоспособная зависимость (PathBroken) блокирует задачу:
    /// php в PATH, но молчит — composer всё равно не запустится.
    #[tokio::test]
    async fn declared_dependency_broken_blocks_task() {
        let mut php = echo_def("php");
        php.display = "PHP".to_string();
        // reg.exe существует в PATH, но с --version молчит (код 1):
        // «бинарь есть, установка сломана» → PathBroken.
        php.detection.version_probes = vec![vec!["reg".to_string(), "--version".to_string()]];
        let mut composer = bare_def();
        composer.id = "composer".to_string();
        composer.display = "Composer".to_string();
        composer.extended.dependencies = vec!["php".to_string()];

        let err = check_declared_dependencies(&composer, &[php, composer.clone()])
            .await
            .unwrap_err();
        assert!(
            err.contains("PHP"),
            "сломанная зависимость обязана блокировать: {err}"
        );
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
        };
        let cmd = build_install_command(&bare_def(), &source, None, None).unwrap();
        assert_eq!(cmd.program, "msiexec");
        assert_eq!(cmd.args[0], "/i");
        assert_eq!(cmd.args[1], "node.msi");
        // диагностический MSI-лог должен быть добавлен автоматически;
        // имя уникально (temp-squatting защита): tc-node-msi-*.log
        assert!(
            cmd.args.iter().any(|a| a == "/l*v"),
            "нет /l*v: {:?}",
            cmd.args
        );
        assert!(
            cmd.args
                .iter()
                .any(|a| a.contains("-msi-") && a.ends_with(".log")),
            "нет пути к MSI-логу: {:?}",
            cmd.args
        );
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
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

        let secrets = execute_plan(&[def], &mut plan, trait_sink, no_abort(), "s-e2e", false).await;

        assert!(secrets.is_empty());
        match &plan.tasks[0].state {
            TaskState::Success { version } => assert_eq!(version, "1.2.3"),
            other => panic!("ожидали Success, получили {other:?}"),
        }

        let events: Vec<ToolchainEvent> = sink.events.lock().unwrap().clone();
        assert!(events
            .iter()
            .any(|e| matches!(e.event_type, ToolchainEventType::TaskStarted)));
        assert!(
            events
                .iter()
                .any(|e| matches!(e.event_type, ToolchainEventType::TaskProgress { .. })),
            "должны были стримиться строки out/err"
        );
        assert!(events.iter().any(|e| matches!(
            e.event_type,
            ToolchainEventType::TaskCompleted {
                state: TaskState::Success { .. }
            }
        )));
        assert!(events
            .iter()
            .any(|e| matches!(e.event_type, ToolchainEventType::AllCompleted { .. })));
        // Все события задания помечены его session_id.
        assert!(
            events.iter().all(|e| e.session_id == "s-e2e"),
            "события обязаны нести session_id задания"
        );
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn failing_installer_marks_failed() {
        let mut def = echo_def("fail-tool");
        def.sources.windows[0].args = vec!["/c".to_string(), "exit".to_string(), "1".to_string()];

        let mut plan = one_task_plan("fail-tool");
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();

        execute_plan(&[def], &mut plan, trait_sink, no_abort(), "s-fail", false).await;

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
                bootstrap: None,
                sha256: None,
                url_template: None,
                version_resolver: None,
            },
            def.sources.windows[0].clone(),
        ];

        let mut plan = one_task_plan("fallback-tool");
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();

        execute_plan(&[def], &mut plan, trait_sink, no_abort(), "s-fb", false).await;

        match &plan.tasks[0].state {
            TaskState::Success { version } => assert_eq!(version, "1.2.3"),
            other => panic!("ожидали Success после fallback, получили {other:?}"),
        }

        // Fallback тихий: промежуточный сбой НЕ должен попадать в UI
        // (tc:info о смене источника) — там только итоговый tc:ok.
        let events: Vec<ToolchainEvent> = sink.events.lock().unwrap().clone();
        assert!(
            !events.iter().any(|e| matches!(
                &e.event_type,
                ToolchainEventType::TaskProgress { line } if line.starts_with("tc:info Источник")
            )),
            "промежуточный сбой не должен светиться в UI: {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(
                &e.event_type,
                ToolchainEventType::TaskProgress { line } if line.starts_with("tc:error")
            )),
            "tc:error при успешном fallback быть не должно: {events:?}"
        );
        assert!(
            events.iter().any(|e| matches!(
                &e.event_type,
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
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
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
        };
        def.sources.windows = vec![bad, bad2];

        let mut plan = one_task_plan("total-fail-tool");
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();

        execute_plan(&[def], &mut plan, trait_sink, no_abort(), "s-allfail", false).await;

        assert!(matches!(plan.tasks[0].state, TaskState::Failed { .. }));

        let events: Vec<ToolchainEvent> = sink.events.lock().unwrap().clone();
        let errors: Vec<&String> = events
            .iter()
            .filter_map(|e| match &e.event_type {
                ToolchainEventType::TaskProgress { line } if line.starts_with("tc:error") => {
                    Some(line)
                }
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

        execute_plan(&[], &mut plan, trait_sink, abort, "s-abort", false).await;

        assert!(matches!(plan.tasks[0].state, TaskState::Skipped { .. }));
    }

    #[tokio::test]
    async fn unknown_tool_becomes_skipped() {
        let mut plan = one_task_plan("no-such-tool");
        let sink = Arc::new(TestSink::default());
        let trait_sink: Arc<dyn EventSink> = sink.clone();
        execute_plan(&[], &mut plan, trait_sink, no_abort(), "s-unknown", false).await;
        assert!(matches!(plan.tasks[0].state, TaskState::Skipped { .. }));
    }

    #[test]
    fn install_execution_supported_returns_true() {
        // On all three supported platforms (Windows, Linux, macOS)
        // installation is supported.
        assert!(
            install_execution_supported(),
            "install_execution_supported must be true on the current platform"
        );
    }

    #[test]
    fn which_exists_for_known_binary() {
        // "cargo" or "rustc" should be available in a Rust build env
        let known = which_exists("cargo") || which_exists("rustc");
        assert!(
            known,
            "at least one Rust toolchain binary should be findable"
        );
    }

    #[test]
    fn which_exists_for_garbage_name() {
        assert!(
            !which_exists("this-definitely-not-a-real-binary-xyzzy"),
            "non-existent binary must not be found"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn build_install_command_windows_pkg_manager() {
        let def = defs::load_definitions()
            .into_iter()
            .find(|d| d.id == "git")
            .unwrap();
        let source = def.sources.windows.first().unwrap();
        let cmd = build_install_command(&def, source, None, None).unwrap();
        assert_eq!(cmd.program, "winget");
        assert!(cmd
            .args
            .windows(2)
            .any(|w| w[0] == "install" && w[1] == "--id"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn build_install_command_linux_pkg_manager_finds_available() {
        // On Linux, PkgManager should resolve to one of apt-get/dnf/pacman/zypper
        let source = InstallSource {
            kind: InstallSourceKind::PkgManager,
            id: "fake-pkg-id".to_string(),
            url: None,
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: None,
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
        };
        let def = bare_def();
        let result = build_install_command(&def, &source, None, None);
        if result.is_ok() {
            let cmd = result.unwrap();
            let valid_programs = ["apt-get", "dnf", "pacman", "zypper"];
            assert!(
                valid_programs.contains(&cmd.program.as_str()),
                "unexpected Linux package manager: {}",
                cmd.program
            );
            assert!(
                cmd.args.contains(&"fake-pkg-id".to_string()),
                "package id should be in args"
            );
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn build_install_command_macos_pkg_manager_uses_brew() {
        // On macOS, PkgManager should resolve to brew
        let source = InstallSource {
            kind: InstallSourceKind::PkgManager,
            id: "fake-mac-pkg".to_string(),
            url: None,
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: None,
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
        };
        let def = bare_def();
        let os = platforms::current_platform().os_name();
        if os == "macos" {
            let cmd = build_install_command(&def, &source, None, None).unwrap();
            assert_eq!(cmd.program, "brew");
            assert!(cmd.args.contains(&"install".to_string()));
            assert!(cmd.args.contains(&"fake-mac-pkg".to_string()));
        }
    }

    #[test]
    fn linux_pkg_manager_error_when_no_manager() {
        // This test verifies the error message when no package manager is found.
        // On CI it might find one, so we just test the function signature works.
        let source = InstallSource {
            kind: InstallSourceKind::PkgManager,
            id: "test".to_string(),
            url: None,
            args: vec![],
            extra_args: vec![],
            dynamic_args: false,
            install_dir: None,
            needs_admin: None,
            file_name: None,
            execution: None,
            bootstrap: None,
            sha256: None,
            url_template: None,
            version_resolver: None,
        };
        let result = build_linux_pkg_command(&source, None);
        if result.is_err() {
            assert!(
                result.unwrap_err().contains("менеджера пакетов"),
                "error should mention package manager"
            );
        }
    }
}
