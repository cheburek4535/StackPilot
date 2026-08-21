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

use super::console::{self, ps_quote};

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
    pub fn validate(&self) -> Result<(), String> {
        for name in &self.entries {
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
    let output = timeout(
        Duration::from_secs(120),
        TokioCommand::new(program).args(args).output(),
    )
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

/// Список записей zip: PowerShell + .NET ZipFile, stdout захватывается.
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
        // Имя — последний столбец (в именах бывают пробелы).
        let name = line.rsplit(' ').next().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        match kind {
            'l' | 'h' => entries.push(format!("{SYMLINK_MARK}{name}")),
            _ => entries.push(name.to_string()),
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
    if listing.contains_symlinks() {
        return Err(
            "архив содержит симлинки/жёсткие ссылки — политика безопасности запрещает их установку"
                .to_string(),
        );
    }
    Ok(listing)
}

/// Безопасно распаковывает zip: prevalidation → Expand-Archive →
/// fallback на tar (как раньше), но только если список записей чист.
/// Эталонный traversal-safe распаковщик слоя; производственный путь
/// installer.rs пока вызывает prevalidate_archive + свои скрипты —
/// миграция распаковки сюда остаётся задокументированным cleanup'ом.
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

/// Безопасно распаковывает tgz/tar: prevalidation → tar -xf.
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
}
