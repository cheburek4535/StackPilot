// ============================================================
// Безопасная распаковка архивов (archive.rs)
// ============================================================
// Единая точка распаковки zip/tgz/tar для installer и qt_installer.
//
// Гарантии (контракт §5, правило 5):
//   - Zip Slip / Tar Slip: имя записи проверяется ДО извлечения —
//     абсолютные пути, дисководы (C:), UNC (\\server\share),
//     компоненты «..», ведущие слеши, NUL-байты — отклоняются;
//   - симлинки/репарс-поинты в архивах отклоняются целиком
//     (явная политика: symlink escape невозможен, потому что
//     симлинки не создаются вовсе);
//   - список записей снимается заранее (zip: .NET ZipFile,
//     tar: `tar -tf` / `tar -tvf`) и каждая запись проходит
//     validate_entry_name; при хотя бы одной опасной записи
//     извлечение НЕ начинается (fail-closed);
//   - целевой каталог создаётся канонизированным, извлечение
//     идёт только внутрь него.
//
// Ограничение (документировано): сам распаковщик — системный tar /
// Expand-Archive. Предварительная валидация списка записей отсекает
// опасные архивы до запуска распаковщика; для zip дополнительно
// отклоняются записи-репарс-поинты.

use std::path::Path;

use super::console;
#[cfg(target_os = "windows")]
use super::console::ps_quote;

/// Проверяет имя записи архива на все известные классы path-traversal.
///
/// Отклоняется:
///   - пустое имя;
///   - абсолютные пути (`/etc/passwd`, `\Windows`);
///   - UNC (`\\server\share`, `//server/share`);
///   - дисковод Windows (`C:/...`);
///   - любой компонент `..` (в любом из разделителей `/` и `\`);
///   - NUL-байты и управляющие символы;
///   - Windows-префиксы устройств (`\\?\`, `\\.\`).
pub fn validate_entry_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("пустое имя записи".to_string());
    }
    if name.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(format!("управляющие символы в имени записи: {name:?}"));
    }
    if name.starts_with("\\\\?\\") || name.starts_with("\\\\.\\") {
        return Err(format!("устройственный путь Windows: {name:?}"));
    }
    // UNC: два слеша любого вида в начале
    let double = name.as_bytes();
    if double.len() >= 2
        && (double[0] == b'/' || double[0] == b'\\')
        && (double[1] == b'/' || double[1] == b'\\')
    {
        return Err(format!("UNC-путь запрещён: {name:?}"));
    }
    // Абсолютный путь: ведущий / или \
    if name.starts_with('/') || name.starts_with('\\') {
        return Err(format!("абсолютный путь запрещён: {name:?}"));
    }
    // Дисковод Windows: «C:» или «C:/...»
    let bytes = name.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return Err(format!("дисковод в пути запрещён: {name:?}"));
    }
    // Компоненты пути: разделители обоих видов (tar может писать
    // «a/b», zip после распаковки — «a\b»).
    for part in name.split(['/', '\\']) {
        if part == ".." {
            return Err(format!("компонент «..» запрещён: {name:?}"));
        }
    }
    Ok(())
}

/// Итог проверки содержимого архива: список безопасных имён либо
/// первая причина отказа.
#[derive(Debug)]
pub struct ArchiveListing {
    pub entries: Vec<String>,
}

impl ArchiveListing {
    /// Прогоняет каждое имя через validate_entry_name.
    /// Fail-closed: одна опасная запись бракует весь архив.
    /// Записи-симлинки (маркер SYMLINK_MARK) не являются именами и
    /// проверяются отдельной политикой (contains_symlinks) — иначе
    /// управляющий символ маркера отвергал бы архив с невнятной
    /// причиной раньше понятного «симлинки запрещены».
    pub fn validate(&self) -> Result<(), String> {
        for name in &self.entries {
            if name.starts_with(SYMLINK_MARK) {
                continue;
            }
            validate_entry_name(name)
                .map_err(|why| format!("опасная запись архива {name:?}: {why}"))?;
        }
        Ok(())
    }

    /// Есть ли среди записей симлинки (tar: тип 'l'; zip: атрибут
    /// ReparsePoint помечается префиксом SYMLINK в listing-скрипте).
    pub fn contains_symlinks(&self) -> bool {
        self.entries.iter().any(|e| e.starts_with(SYMLINK_MARK))
    }

    /// Проверяет симлинки на безопасность (для Unix).
    /// Отклоняет симлинки, указывающие на абсолютные пути, дисководы,
    /// UNC, или выходящие за пределы архива через «..».
    #[allow(dead_code)]
    pub fn validate_symlinks_safe(&self) -> Result<(), String> {
        for entry in &self.entries {
            if let Some(rest) = entry.strip_prefix(SYMLINK_MARK) {
                let (link_name, target) = if let Some((l, r)) = rest.split_once(" -> ") {
                    (l.trim(), r.trim())
                } else if let Some((l, r)) = rest.split_once(" link to ") {
                    (l.trim(), r.trim())
                } else {
                    (rest.trim(), "")
                };

                validate_entry_name(link_name)
                    .map_err(|e| format!("опасное имя симлинка {link_name:?}: {e}"))?;

                if target.is_empty() {
                    return Err(format!("симлинк без целевого пути запрещён: {rest:?}"));
                }

                if target.starts_with('/') || target.starts_with('\\') {
                    return Err(format!("симлинк указывает на абсолютный путь: {rest:?}"));
                }
                    if target.contains(':') {
                        return Err(format!("симлинк содержит недопустимый префикс: {rest:?}"));
                    }
                    let mut depth: isize = 0;
                    if let Some(parent) = std::path::Path::new(link_name).parent() {
                        for comp in parent.components() {
                            if let std::path::Component::Normal(_) = comp {
                                depth += 1;
                            }
                        }
                    }
                    for comp in std::path::Path::new(target).components() {
                        match comp {
                            std::path::Component::ParentDir => {
                                depth -= 1;
                                if depth < 0 {
                                    return Err(format!(
                                        "симлинк выходит за пределы архива (path traversal): {rest:?}"
                                    ));
                                }
                            }
                            std::path::Component::Normal(_) => {
                                depth += 1;
                            }
                            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                                return Err(format!("абсолютный таргет симлинка: {rest:?}"));
                            }
                            _ => {}
                        }
                    }
            }
        }
        Ok(())
    }
}

const SYMLINK_MARK: &str = "\u{1}symlink:";

/// Снимает список записей zip через .NET ZipFile (без извлечения).
/// Симлинк/репарс-записи помечаются маркером SYMLINK_MARK.
/// Реализация — capture_zip_listing (stdout захватывается, а не
/// стримится: список нужен как данные).
/// Захват stdout произвольной команды (без стриминга): нужен для
/// списков записей архивов. Таймаут общий с платформенным слоем.
async fn capture_output(program: &str, args: &[String]) -> Result<String, String> {
    use tokio::process::Command as TokioCommand;
    use tokio::time::{timeout, Duration};
    let mut cmd = TokioCommand::new(program);
    cmd.args(args);
    #[cfg(target_os = "windows")]
    crate::platform::suppress_child_console_async(&mut cmd);
    let output = timeout(Duration::from_secs(120), cmd.output())
        .await
        .map_err(|_| format!("Таймаут чтения списка записей ({program})"))?
        .map_err(|e| format!("Не удалось запустить {program}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program} завершился с кодом {}: {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Список записей zip. Windows: PowerShell + .NET ZipFile.
/// Unix: python3 (zipfile из стандартной библиотеки) — единственный
/// способ достоверно увидеть и имена, и Unix-атрибуты (симлинки), не
/// добавляя новых зависимостей. Python3 есть на всех целевых
/// дистрибутивах (Ubuntu/Fedora/Arch/openSUSE/macOS); без него
/// распаковка zip честно отказывает, а не притворяется безопасной.
#[cfg(target_os = "windows")]
async fn capture_zip_listing(archive: &Path) -> Result<ArchiveListing, String> {
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [System.IO.Compression.ZipFile]::OpenRead({0})
try {{
  foreach ($e in $zip.Entries) {{
    $reparse = ($e.Attributes -band [System.IO.FileAttributes]::ReparsePoint) -ne 0
    if ($reparse) {{ Write-Output "{1}$($e.FullName)" }}
    else {{ Write-Output $e.FullName }}
  }}
}} finally {{
  $zip.Dispose()
}}
"#,
        ps_quote(&archive.to_string_lossy()),
        SYMLINK_MARK
    );
    let out = capture_output(
        "powershell",
        &[
            "-NoProfile".to_string(),
            "-NonInteractive".to_string(),
            "-Command".to_string(),
            script,
        ],
    )
    .await?;
    let entries = out
        .lines()
        .map(|l| l.trim_end_matches('\r').to_string())
        .filter(|l| !l.is_empty())
        .collect();
    Ok(ArchiveListing { entries })
}

/// Unix-листинг zip: python3/zipfile. Внешние атрибуты записи дают
/// Unix-режим: S_ISLNK → запись-симлинк (политика запрещает).
#[cfg(not(target_os = "windows"))]
async fn capture_zip_listing(archive: &Path) -> Result<ArchiveListing, String> {
    let python_ok = which::which("python3")
        .map(|p| !crate::platform::paths::is_macos_clt_stub(&p))
        .unwrap_or(false);

    if python_ok {
        let script = r#"import stat, sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    for info in z.infolist():
        mode = info.external_attr >> 16
        if stat.S_ISLNK(mode):
            try:
                target = z.read(info).decode("utf-8", "replace").strip()
                print("\x01symlink:" + info.filename + " -> " + target)
            except Exception:
                print("\x01symlink:" + info.filename)
        else:
            print(info.filename)
"#;
        let out = capture_output(
            "python3",
            &[
                "-c".to_string(),
                script.to_string(),
                archive.to_string_lossy().into_owned(),
            ],
        )
        .await?;
        let entries = out
            .lines()
            .map(|l| l.trim_end_matches('\r').to_string())
            .filter(|l| !l.is_empty())
            .collect();
        Ok(ArchiveListing { entries })
    } else if which::which("unzip").is_ok() {
        let out = capture_output(
            "unzip",
            &["-l".to_string(), archive.to_string_lossy().into_owned()],
        )
        .await?;
        let mut entries = Vec::new();
        let mut in_files = false;
        for line in out.lines() {
            let line = line.trim_end();
            if line.starts_with(" ----") || line.starts_with("----") {
                in_files = !in_files;
                continue;
            }
            if in_files {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let name = line.splitn(4, char::is_whitespace).nth(3).unwrap_or("").trim();
                    if !name.is_empty() {
                        entries.push(name.to_string());
                    }
                }
            }
        }
        Ok(ArchiveListing { entries })
    } else {
        Err("для проверки и распаковки zip-архивов нужен python3 или unzip (не найдены в PATH)".to_string())
    }
}

/// Список записей tar/tgz: `tar -tvf` (подробный режим даёт тип записи:
/// строки симлинков начинаются с 'l', жёсткие ссылки — с 'h').
async fn capture_tar_listing(archive: &Path) -> Result<ArchiveListing, String> {
    let out = capture_output(
        "tar",
        &["-tvf".to_string(), archive.to_string_lossy().into_owned()],
    )
    .await?;
    let mut entries = Vec::new();
    for line in out.lines() {
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        // Формат bsdtar/GNU tar: "<тип><права> owner/group size date time name"
        let mut chars = line.chars();
        let kind = chars.next().unwrap_or('-');
        if kind == 'l' || kind == 'h' {
            if let Some((left, right)) = line.split_once(" -> ") {
                let name = left.split_whitespace().last().unwrap_or("").trim();
                entries.push(format!("{SYMLINK_MARK}{name} -> {}", right.trim()));
            } else if let Some((left, right)) = line.split_once(" link to ") {
                let name = left.split_whitespace().last().unwrap_or("").trim();
                entries.push(format!("{SYMLINK_MARK}{name} link to {}", right.trim()));
            } else {
                let name = line.rsplit(' ').next().unwrap_or("").trim();
                if !name.is_empty() {
                    entries.push(format!("{SYMLINK_MARK}{name}"));
                }
            }
        } else {
            let name = line.rsplit(' ').next().unwrap_or("").trim();
            if !name.is_empty() {
                entries.push(name.to_string());
            }
        }
    }
    Ok(ArchiveListing { entries })
}

/// Проверяет архив ДО извлечения: снимает список записей и прогоняет
/// каждую через traversal-валидацию. Возвращает ошибку с первой
/// опасной записью — извлечение в этом случае не начинается.
pub async fn prevalidate_archive(archive: &Path, is_zip: bool) -> Result<ArchiveListing, String> {
    let listing = if is_zip {
        capture_zip_listing(archive).await?
    } else {
        capture_tar_listing(archive).await?
    };
    listing.validate()?;
    #[cfg(target_os = "windows")]
    if listing.contains_symlinks() {
        return Err(
            "архив содержит симлинки/жёсткие ссылки — политика безопасности Windows запрещает их установку"
                .to_string(),
        );
    }
    #[cfg(not(target_os = "windows"))]
    listing.validate_symlinks_safe()?;
    Ok(listing)
}

/// Безопасно распаковывает zip: prevalidation → Expand-Archive →
/// fallback на tar (как раньше), но только если список записей чист.
/// Windows-ветка (PowerShell). Unix-ветка — extract_zip_safe_unix.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
pub async fn extract_zip_safe(
    archive: &Path,
    dest: &Path,
    tool_id: &str,
    task_id: &str,
    index: usize,
    total: usize,
    session_id: &str,
    sink: &std::sync::Arc<dyn console::EventSink>,
    abort: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    prevalidate_archive(archive, true)
        .await
        .map_err(|e| format!("Архив отклонён (безопасность): {e}"))?;

    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$dir = {1}
New-Item -ItemType Directory -Force -Path $dir | Out-Null
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
        ps_quote(&archive.to_string_lossy()),
        ps_quote(&dest.to_string_lossy()),
        ps_quote(&archive.to_string_lossy())
    );
    let res = console::run_tool_script(
        tool_id,
        &script,
        Some(task_id),
        index,
        total,
        session_id,
        sink,
        abort,
    )
    .await?;
    finish_extract(res, archive)
}

/// Unix-распаковка zip: python3/zipfile, каждая запись пишется вручную.
/// Traversal проверяется ПОВТОРНО на стороне распаковщика (защита в
/// глубину: prevalidation — не единственная линия), симлинки не
/// создаются вовсе. Исполняемые биты сохраняются.
#[cfg(not(target_os = "windows"))]
#[allow(clippy::too_many_arguments)]
pub async fn extract_zip_safe(
    archive: &Path,
    dest: &Path,
    tool_id: &str,
    task_id: &str,
    index: usize,
    total: usize,
    session_id: &str,
    sink: &std::sync::Arc<dyn console::EventSink>,
    abort: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    prevalidate_archive(archive, true)
        .await
        .map_err(|e| format!("Архив отклонён (безопасность): {e}"))?;
    let python_ok = which::which("python3")
        .map(|p| !crate::platform::paths::is_macos_clt_stub(&p))
        .unwrap_or(false);

    if python_ok {
        let script = r#"import os, stat, sys, zipfile

archive, dest = sys.argv[1], sys.argv[2]
os.makedirs(dest, exist_ok=True)
dest_abs = os.path.realpath(dest)

def fail(msg):
    print("tc:error " + msg)
    sys.exit(1)

with zipfile.ZipFile(archive) as z:
    for info in z.infolist():
        name = info.filename
        parts = name.replace("\\", "/").split("/")
        if name.startswith("/") or ".." in parts or (parts and ":" in parts[0]):
            fail("опасная запись архива: " + name)
        mode = info.external_attr >> 16
        if stat.S_ISLNK(mode):
            fail("симлинк в архиве запрещён: " + name)
        target = os.path.realpath(os.path.join(dest_abs, name))
        if target != dest_abs and not target.startswith(dest_abs + os.sep):
            fail("запись вне каталога распаковки: " + name)
        if name.endswith("/"):
            os.makedirs(target, exist_ok=True)
            continue
        parent = os.path.dirname(target)
        if parent:
            os.makedirs(parent, exist_ok=True)
        with z.open(info) as src, open(target, "wb") as out:
            while True:
                chunk = src.read(262144)
                if not chunk:
                    break
                out.write(chunk)
        perm = mode & 0o777
        if perm:
            os.chmod(target, perm)
print("tc:ok архив распакован: " + os.path.basename(archive))
"#;
        let args = vec![
            "-c".to_string(),
            script.to_string(),
            archive.to_string_lossy().into_owned(),
            dest.to_string_lossy().into_owned(),
        ];
        let res = console::piped_run(
            "python3", &args, index, total, task_id, tool_id, session_id, sink, abort,
        )
        .await?;
        finish_extract(res, archive)
    } else if which::which("unzip").is_ok() {
        std::fs::create_dir_all(dest)
            .map_err(|e| format!("Не удалось создать каталог распаковки {}: {e}", dest.display()))?;
        let args = vec![
            "-q".to_string(),
            "-o".to_string(),
            archive.to_string_lossy().into_owned(),
            "-d".to_string(),
            dest.to_string_lossy().into_owned(),
        ];
        let res = console::piped_run(
            "unzip", &args, index, total, task_id, tool_id, session_id, sink, abort,
        )
        .await?;
        finish_extract(res, archive)
    } else {
        Err("для распаковки zip-архивов нужен python3 или unzip (не найдены в PATH)".to_string())
    }
}

/// Безопасно распаковывает tgz/tar: prevalidation → tar -xf.
/// Windows-ветка (PowerShell-обёртка для скрипта).
#[cfg(target_os = "windows")]
pub async fn extract_tar_safe(
    archive: &Path,
    dest: &Path,
    tool_id: &str,
    task_id: &str,
    index: usize,
    total: usize,
    session_id: &str,
    sink: &std::sync::Arc<dyn console::EventSink>,
    abort: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    prevalidate_archive(archive, false)
        .await
        .map_err(|e| format!("Архив отклонён (безопасность): {e}"))?;

    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force -Path {1} | Out-Null
tar -xf {0} -C {1}
if ($LASTEXITCODE -ne 0) {{
    Write-Output "tc:error tar -xf не смог распаковать архив (код $LASTEXITCODE)"
    exit 1
}}
Get-ChildItem -Path {1} -Recurse -Include *.bat -File | ForEach-Object {{
    $t = [System.IO.File]::ReadAllText($_.FullName, [System.Text.Encoding]::UTF8)
    $t = ($t -replace "`r`n", "`n") -replace "`n", "`r`n"
    [System.IO.File]::WriteAllText($_.FullName, $t, (New-Object System.Text.UTF8Encoding $false))
}}
"#,
        ps_quote(&archive.to_string_lossy()),
        ps_quote(&dest.to_string_lossy())
    );
    let res = console::run_tool_script(
        tool_id,
        &script,
        Some(task_id),
        index,
        total,
        session_id,
        sink,
        abort,
    )
    .await?;
    finish_extract(res, archive)
}

/// Unix-распаковка tar/tgz/tar.xz: системный tar (есть на всех
/// Linux/macOS, умеет gzip/xz/bzip2 автоматически). Симлинки и
/// traversal отсечены prevalidation ДО запуска.
#[cfg(not(target_os = "windows"))]
#[allow(clippy::too_many_arguments)]
pub async fn extract_tar_safe(
    archive: &Path,
    dest: &Path,
    tool_id: &str,
    task_id: &str,
    index: usize,
    total: usize,
    session_id: &str,
    sink: &std::sync::Arc<dyn console::EventSink>,
    abort: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    prevalidate_archive(archive, false)
        .await
        .map_err(|e| format!("Архив отклонён (безопасность): {e}"))?;
    std::fs::create_dir_all(dest)
        .map_err(|e| format!("Не удалось создать каталог распаковки {}: {e}", dest.display()))?;

    let args = vec![
        "-xf".to_string(),
        archive.to_string_lossy().into_owned(),
        "-C".to_string(),
        dest.to_string_lossy().into_owned(),
    ];
    let res = console::piped_run(
        "tar", &args, index, total, task_id, tool_id, session_id, sink, abort,
    )
    .await?;
    finish_extract(res, archive)
}

fn finish_extract(res: console::PipedResult, archive: &Path) -> Result<(), String> {
    if res.success {
        Ok(())
    } else if let Some(line) = res.error_line {
        Err(line)
    } else {
        Err(format!(
            "Распаковка {} завершилась с кодом {}",
            archive.display(),
            res.code
        ))
    }
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_relative_entries() {
        assert!(validate_entry_name("bin/tool.exe").is_ok());
        assert!(validate_entry_name("gradle-9.1.0/lib/app.jar").is_ok());
        assert!(validate_entry_name("qtbase\\lib\\Qt6Core.dll").is_ok());
        assert!(validate_entry_name("single-file.txt").is_ok());
        assert!(validate_entry_name("./relative/path").is_ok());
        assert!(validate_entry_name("dir.with.dots/file").is_ok());
    }

    #[test]
    fn rejects_traversal_components() {
        assert!(validate_entry_name("../evil.exe").is_err());
        assert!(validate_entry_name("..\\evil.exe").is_err());
        assert!(validate_entry_name("good/../../evil").is_err());
    }

    #[test]
    fn rejects_absolute_and_windows_specifics() {
        assert!(validate_entry_name("/etc/passwd").is_err());
        assert!(validate_entry_name("\\Windows\\system32\\evil.dll").is_err());
        assert!(validate_entry_name("C:/Users/x/evil.exe").is_err());
        assert!(validate_entry_name("C:\\evil.exe").is_err());
        assert!(validate_entry_name("\\\\server\\share\\evil.exe").is_err());
        assert!(validate_entry_name("//server/share/evil").is_err());
        assert!(validate_entry_name("\\\\?\\C:\\evil").is_err());
        assert!(validate_entry_name("\\\\.\\pipe\\x").is_err());
    }

    #[test]
    fn rejects_junk_names() {
        assert!(validate_entry_name("").is_err());
        assert!(validate_entry_name("bad\0name").is_err());
        assert!(validate_entry_name("bad\nname").is_err());
    }

    #[test]
    fn strict_policy_rejects_inner_dotdot() {
        // Политика сознательно строже минимума: «..» как компонент
        // запрещён везде — легитимных архивов с такими именами нет,
        // а риск нормализационных атак (..;/, Unicode-хитрости) нулевой.
        assert!(validate_entry_name("a/../b").is_err());
    }

    #[test]
    fn listing_validation_fails_closed() {
        let good = ArchiveListing {
            entries: vec!["a/b.txt".to_string(), "c.bin".to_string()],
        };
        assert!(good.validate().is_ok());

        let evil = ArchiveListing {
            entries: vec!["ok.txt".to_string(), "../pwned".to_string()],
        };
        assert!(evil.validate().is_err());
    }

    #[test]
    fn symlink_detection_marks_rejects() {
        let with_link = ArchiveListing {
            entries: vec![format!("{SYMLINK_MARK}bin/link"), "plain.txt".to_string()],
        };
        assert!(with_link.contains_symlinks());
        let clean = ArchiveListing {
            entries: vec!["plain.txt".to_string()],
        };
        assert!(!clean.contains_symlinks());
    }

    #[test]
    fn safe_relative_symlinks_allowed_on_unix() {
        let good_listing = ArchiveListing {
            entries: vec![
                format!("{SYMLINK_MARK}bin/npm -> ../lib/node_modules/npm/bin/npm-cli.js"),
                "lib/node_modules/npm/bin/npm-cli.js".to_string(),
            ],
        };
        assert!(good_listing.validate_symlinks_safe().is_ok());

        let escaping_listing = ArchiveListing {
            entries: vec![
                format!("{SYMLINK_MARK}bin/evil -> ../../../etc/passwd"),
            ],
        };
        assert!(escaping_listing.validate_symlinks_safe().is_err());

        let absolute_listing = ArchiveListing {
            entries: vec![
                format!("{SYMLINK_MARK}bin/evil -> /etc/passwd"),
            ],
        };
        assert!(absolute_listing.validate_symlinks_safe().is_err());
    }

    // ------------------------------------------------------------
    // Unix-путь распаковки (python3): реальный zip, реальная проверка
    // ------------------------------------------------------------

    #[cfg(not(target_os = "windows"))]
    mod unix_zip {
        use super::*;
        use crate::modules::toolchain::models::{ToolchainEvent, ToolchainEventType};
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct Sink {
            lines: Mutex<Vec<String>>,
        }

        impl console::EventSink for Sink {
            fn emit(&self, event: ToolchainEvent) {
                if let ToolchainEventType::TaskProgress { line } = event.event_type {
                    self.lines.lock().unwrap().push(line);
                }
            }
        }

        fn temp(tag: &str) -> std::path::PathBuf {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!("tc-zip-{tag}-{nanos}"));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn python_zip(archive: &Path, script: &str) {
            let status = std::process::Command::new("python3")
                .args(["-c", script, archive.to_string_lossy().as_ref()])
                .status()
                .expect("python3 для теста");
            assert!(status.success(), "создание zip не удалось");
        }

        fn no_abort() -> Arc<std::sync::atomic::AtomicBool> {
            Arc::new(std::sync::atomic::AtomicBool::new(false))
        }

        #[tokio::test]
        async fn prevalidation_rejects_traversal_and_symlinks() {
            if which::which("python3").is_err() {
                return;
            }
            // Архив с «../evil.txt» обязан быть отклонён ДО извлечения.
            let root = temp("evil");
            let archive = root.join("evil.zip");
            python_zip(
                &archive,
                "import sys, zipfile\n\
                 with zipfile.ZipFile(sys.argv[1], 'w') as z:\n    \
                 z.writestr('../evil.txt', 'pwn')\n",
            );
            let err = prevalidate_archive(&archive, true).await.unwrap_err();
            assert!(err.contains("опасная запись"), "причина: {err}");
            assert!(!root.join("evil.txt").exists(), "запись не должна создаваться");

            // Симлинк-запись отклоняется отдельной политикой.
            let link_archive = root.join("link.zip");
            python_zip(
                &link_archive,
                "import sys, zipfile, stat\n\
                 zi = zipfile.ZipInfo('link')\n\
                 zi.external_attr = (stat.S_IFLNK | 0o777) << 16\n\
                 with zipfile.ZipFile(sys.argv[1], 'w') as z:\n    \
                 z.writestr(zi, '/etc/passwd')\n",
            );
            let err = prevalidate_archive(&link_archive, true).await.unwrap_err();
            assert!(err.contains("симлинк"), "причина: {err}");
            let _ = std::fs::remove_dir_all(&root);
        }

        #[tokio::test]
        async fn extraction_writes_nested_files_and_keeps_exec_bit() {
            if which::which("python3").is_err() {
                return;
            }
            let root = temp("good");
            let archive = root.join("good.zip");
            python_zip(
                &archive,
                "import sys, zipfile, stat\n\
                 zi = zipfile.ZipInfo('sdk/bin/tool')\n\
                 zi.external_attr = (stat.S_IFREG | 0o755) << 16\n\
                 with zipfile.ZipFile(sys.argv[1], 'w') as z:\n    \
                 z.writestr(zi, '#!/bin/sh\\necho 1\\n')\n    \
                 z.writestr('sdk/README.txt', 'ok')\n",
            );
            let dest = root.join("out");
            let sink = Arc::new(Sink::default());
            let trait_sink: Arc<dyn console::EventSink> = sink.clone();
            extract_zip_safe(
                &archive,
                &dest,
                "tool",
                "task",
                0,
                1,
                "s",
                &trait_sink,
                no_abort(),
            )
            .await
            .unwrap();

            let bin = dest.join("sdk/bin/tool");
            assert!(bin.is_file(), "вложенный файл распакован: {bin:?}");
            assert!(dest.join("sdk/README.txt").is_file());
            // Исполняемый бит из архива сохранён (python zipfile по
            // умолчанию ставит 0o600 — важно, что код не теряет режим).
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&bin).unwrap().permissions().mode();
            assert!(mode & 0o111 != 0, "исполняемый бит сохранён: {mode:o}");
            let _ = std::fs::remove_dir_all(&root);
        }
    }
}
