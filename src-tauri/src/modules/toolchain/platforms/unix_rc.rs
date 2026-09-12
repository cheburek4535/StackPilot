// ============================================================
// Unix-общее: пользовательский PATH через rc-файлы шелла
// ============================================================
// На Linux/macOS у «пользовательского PATH» нет единого места, как
// реестр на Windows. Toolchain Manager ведёт свой блок в rc-файле:
//
//   # StackPilot:begin
//   export PATH="/managed/a:/managed/b:$PATH"
//   # StackPilot:end
//
// Блок управляется целиком:
//   - read_rc_user_path: возвращает записи из блока (без $PATH);
//   - write_rc_user_path: полностью заменяет блок, сохраняя
//     остальное содержимое rc-файла.
//
// Дизайн:
//   - Никогда не заменяем весь rc-файл (мёртвый пользователь).
//   - $PATH в export-линии сохраняет наследуемый PATH шелла.
//   - Символы " $ ` фильтруются для предотвращения shell injection.
//   - Атомарная запись: temp-файл + rename (на той же FS).
//   - For current process PATH: use path_service::sync_process_path.

use std::path::PathBuf;

const RC_BEGIN: &str = "# StackPilot:begin";
const RC_END: &str = "# StackPilot:end";

/// Выбирает rc-файл для записи StackPilot-блока.
/// Приоритет:
///   1. Файл, уже содержащий StackPilot:begin (какой бы он ни был)
///   2. Файл по умолчанию для текущей оболочки ($SHELL):
///      - zsh → ~/.zshrc
///      - bash → ~/.bashrc (если существует), иначе ~/.bash_profile
///   3. ~/.profile (fallback)
///
/// Никогда не создаёт новый rc-файл — если ничего не существует,
/// используем ~/.profile (создастся при записи).
fn rc_path() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));

    // 1. Уже содержащий StackPilot:begin?
    for name in &[".zshrc", ".bashrc", ".bash_profile", ".profile"] {
        let path = home.join(name);
        if let Ok(content) = std::fs::read_to_string(&path) {
            if content.contains(RC_BEGIN) {
                return path;
            }
        }
    }

    // 2. По умолчанию для оболочки.
    let shell = std::env::var("SHELL").unwrap_or_default();
    if shell.contains("zsh") {
        return home.join(".zshrc");
    }
    if shell.contains("bash") {
        let bashrc = home.join(".bashrc");
        if bashrc.exists() {
            return bashrc;
        }
        return home.join(".bash_profile");
    }

    // 3. Fallback.
    home.join(".profile")
}

/// Sanitize a path entry to prevent shell injection.
/// Strips characters that could execute code in an export context.
fn sanitize_path_entry(entry: &str) -> String {
    entry
        .chars()
        .filter(|c| *c != '"' && *c != '$' && *c != '`' && *c != '\'' && *c != '\\')
        .collect()
}

/// Записи PATH из блока StackPilot в rc-файле.
/// Возвращает ТОЛЬКО Managed-записи (без $PATH).
pub fn read_rc_user_path() -> Vec<String> {
    let path = rc_path();
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    extract_block_entries(&content)
}

/// Extract managed entries from a StackPilot block.
/// Parses `export PATH="entry1:entry2:$PATH"` and returns only
/// the concrete managed entries (not $PATH itself).
fn extract_block_entries(content: &str) -> Vec<String> {
    let Some(begin_pos) = content.lines().position(|l| l.trim() == RC_BEGIN) else {
        return Vec::new();
    };

    // Find the first export PATH line between begin and end.
    for line in content.lines().skip(begin_pos + 1) {
        if line.trim() == RC_END {
            break;
        }
        if let Some(entries) = parse_export_path_line(line) {
            return entries;
        }
    }
    Vec::new()
}

/// Parse an `export PATH="..."` line and return managed entries.
/// The line format is: export PATH="/a:/b:$PATH"
/// We extract everything before $PATH (or ${PATH}).
fn parse_export_path_line(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    // Must start with "export PATH" (after optional spaces)
    if !trimmed.starts_with("export PATH=") && !trimmed.starts_with("export PATH =") {
        return None;
    }

    // Extract the value between quotes
    let after_eq = trimmed.splitn(2, '=').nth(1)?;
    let value = after_eq.trim();
    let value = value.strip_prefix('"').unwrap_or(value);
    let value = value.strip_suffix('"').unwrap_or(value);

    // Split by ':', remove $PATH references
    let entries: Vec<String> = value
        .split(':')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "$PATH" && s != "${PATH}")
        .collect();

    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

/// Replace (or create) the StackPilot block in the rc-file.
/// The block content is: export PATH="/managed/a:/managed/b:$PATH"
/// Managed entries are prepended, $PATH preserves inherited PATH.
pub fn write_rc_user_path(dirs: &[String]) -> Result<(), String> {
    let path = rc_path();
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    // Sanitize entries to prevent shell injection
    let safe_entries: Vec<String> = dirs.iter().map(|d| sanitize_path_entry(d)).collect();

    // Build the export line: prepend managed dirs, preserve $PATH
    let managed_part = safe_entries.join(":");
    let export_line = if managed_part.is_empty() {
        // No managed dirs: keep $PATH as-is (block is a no-op placeholder)
        format!("export PATH=\"$PATH\"")
    } else {
        format!("export PATH=\"{managed_part}:$PATH\"")
    };

    let block = format!("{RC_BEGIN}\n{export_line}\n{RC_END}\n");

    let new_content = if let Some(start) = existing.find(RC_BEGIN) {
        // Replace existing block
        let end = existing
            .find(RC_END)
            .map(|e| e + RC_END.len())
            .unwrap_or(existing.len());
        // Include any trailing newline after RC_END
        let end = if end < existing.len() && existing.as_bytes().get(end) == Some(&b'\n') {
            end + 1
        } else {
            end
        };
        let mut s = String::with_capacity(existing.len());
        s.push_str(&existing[..start]);
        s.push_str(&block);
        s.push_str(&existing[end..]);
        s
    } else {
        // Append block
        let mut s = existing;
        if !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(&block);
        s
    };

    // Atomic write: temp file + rename
    let temp_path = path.with_extension("rc.tmp");
    std::fs::write(&temp_path, &new_content)
        .map_err(|e| format!("Не удалось записать {}: {e}", temp_path.display()))?;

    std::fs::rename(&temp_path, &path).map_err(|e| {
        format!(
            "Не удалось переименовать {} → {}: {e}",
            temp_path.display(),
            path.display()
        )
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::Mutex;

    /// Большинство тестов дёргают глобальный HOME/SHELL — в тестах
    /// они обязаны выполняться строго по одному, иначе параллельные
    /// тесты читают чужие rc-файлы.
    static RC_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Создаёт временный rc-файл и подменяет HOME/SHELL.
    fn with_rc(shell: &str, content: &str, f: impl FnOnce()) {
        let _guard = RC_TEST_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join(format!(
            "tc-rc-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&home).unwrap();
        let rc_name = if shell.contains("zsh") {
            ".zshrc"
        } else {
            ".bashrc"
        };
        let mut file = std::fs::File::create(home.join(rc_name)).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();

        let old = std::env::var("HOME").ok();
        let old_shell = std::env::var("SHELL").ok();
        std::env::set_var("HOME", &home);
        std::env::set_var("SHELL", shell);

        f();

        // Восстанавливаем окружение и убираем временный каталог.
        match old {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match old_shell {
            Some(v) => std::env::set_var("SHELL", v),
            None => std::env::remove_var("SHELL"),
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn read_empty_when_no_marker() {
        with_rc("/bin/bash", "export PATH=\"/usr/bin:/bin\"\n", || {
            assert!(read_rc_user_path().is_empty());
        });
    }

    #[test]
    fn read_parses_marker_block() {
        with_rc(
            "/bin/bash",
            "# other\n# StackPilot:begin\nexport PATH=\"/a:/b:$PATH\"\n# StackPilot:end\n",
            || {
                assert_eq!(
                    read_rc_user_path(),
                    vec!["/a".to_string(), "/b".to_string()]
                );
            },
        );
    }

    #[test]
    fn read_excludes_path_variable() {
        with_rc(
            "/bin/bash",
            "# StackPilot:begin\nexport PATH=\"/managed:$PATH\"\n# StackPilot:end\n",
            || {
                assert_eq!(read_rc_user_path(), vec!["/managed".to_string()]);
            },
        );
    }

    #[test]
    fn write_creates_block_with_path_preservation() {
        with_rc("/bin/bash", "export FOO=bar\n", || {
            write_rc_user_path(&["/a".to_string(), "/b".to_string()]).unwrap();
            let content =
                std::fs::read_to_string(std::env::var("HOME").unwrap() + "/.bashrc").unwrap();
            assert!(content.contains("# StackPilot:begin"));
            assert!(content.contains("export PATH=\"/a:/b:$PATH\""));
            assert!(content.contains("# StackPilot:end"));
            assert!(content.starts_with("export FOO=bar\n"));
        });
    }

    #[test]
    fn write_replaces_existing_block() {
        with_rc(
            "/bin/bash",
            "# StackPilot:begin\nexport PATH=\"/old\"\n# StackPilot:end\n",
            || {
                write_rc_user_path(&["/new1".to_string(), "/new2".to_string()]).unwrap();
                assert_eq!(
                    read_rc_user_path(),
                    vec!["/new1".to_string(), "/new2".to_string()]
                );
            },
        );
    }

    #[test]
    fn write_preserves_surrounding_content() {
        with_rc(
            "/bin/bash",
            "# User stuff\nexport PS1='> '\n# StackPilot:begin\nexport PATH=\"/old\"\n# StackPilot:end\n# More user stuff\n",
            || {
                write_rc_user_path(&["/new".to_string()]).unwrap();
                let content =
                    std::fs::read_to_string(std::env::var("HOME").unwrap() + "/.bashrc").unwrap();
                assert!(content.contains("# User stuff"));
                assert!(content.contains("export PS1="));
                assert!(content.contains("# More user stuff"));
                assert!(content.contains("export PATH=\"/new:$PATH\""));
            },
        );
    }

    #[test]
    fn write_strips_dangerous_chars() {
        with_rc("/bin/bash", "", || {
            write_rc_user_path(&["/a\"$(rm -rf /)".to_string()]).unwrap();
            let entries = read_rc_user_path();
            assert_eq!(entries, vec!["/a(rm -rf /)".to_string()]);
            // Кавычки в блоке — только синтаксис export PATH="...":
            // сама управляемая запись обязана быть очищена от опасных
            // символов ("$, `, ', \) — проверяем строку целиком.
            let content =
                std::fs::read_to_string(std::env::var("HOME").unwrap() + "/.bashrc").unwrap();
            let line = content
                .lines()
                .find(|l| l.contains("export PATH="))
                .expect("export PATH line in block");
            assert_eq!(
                line.trim(),
                r#"export PATH="/a(rm -rf /):$PATH""#,
                "dangerous chars must be stripped from the managed entry"
            );
        });
    }

    #[test]
    fn write_empty_dirs_preserves_path_variable() {
        with_rc("/bin/bash", "", || {
            write_rc_user_path(&[]).unwrap();
            let content =
                std::fs::read_to_string(std::env::var("HOME").unwrap() + "/.bashrc").unwrap();
            assert!(content.contains("export PATH=\"$PATH\""));
        });
    }

    #[test]
    fn parse_export_path_line_works() {
        assert_eq!(
            parse_export_path_line("export PATH=\"/a:/b:$PATH\""),
            Some(vec!["/a".to_string(), "/b".to_string()])
        );
        assert_eq!(
            parse_export_path_line("export PATH=\"/only\""),
            Some(vec!["/only".to_string()])
        );
        assert_eq!(parse_export_path_line("export FOO=bar"), None);
        assert_eq!(parse_export_path_line("not export"), None);
    }

    #[test]
    fn extract_block_entries_works() {
        let content = "# StackPilot:begin\nexport PATH=\"/x:/y:$PATH\"\n# StackPilot:end\n";
        assert_eq!(
            extract_block_entries(content),
            vec!["/x".to_string(), "/y".to_string()]
        );
    }

    #[test]
    fn rc_path_selects_zshrc_for_zsh() {
        with_rc("/bin/zsh", "", || {
            let path = rc_path();
            assert!(
                path.to_string_lossy().ends_with(".zshrc"),
                "zsh should select .zshrc: {path:?}"
            );
        });
    }

    #[test]
    fn sanitize_removes_injection_chars() {
        assert_eq!(sanitize_path_entry("/a\"b"), "/ab");
        assert_eq!(sanitize_path_entry("/a$b"), "/ab");
        assert_eq!(sanitize_path_entry("/a`b`"), "/ab");
        assert_eq!(sanitize_path_entry("/a'b"), "/ab");
        assert_eq!(sanitize_path_entry("/normal/path"), "/normal/path");
    }

    #[test]
    fn write_roundtrip_preserves_entries() {
        with_rc("/bin/bash", "", || {
            let dirs = vec![
                "/managed/a".to_string(),
                "/managed/b".to_string(),
                "/managed/c".to_string(),
            ];
            write_rc_user_path(&dirs).unwrap();
            let readback = read_rc_user_path();
            assert_eq!(readback, dirs);
        });
    }
}
