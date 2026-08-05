// ============================================================
// Unix-общее: пользовательский PATH через rc-файлы шелла
// ============================================================
// На Linux/macOS у «пользовательского PATH» нет единого места, как
// реестр на Windows. Toolchain Manager ведёт свой блок в rc-файле
// (~/.bashrc, ~/.zshrc или ~/.profile — что существует):
//
//   # StackPilot:begin
//   export PATH="/path/one:/path/two"
//   # StackPilot:end
//
// Блок управляется целиком: чтение возвращает записи из него,
// запись полностью заменяет. Чужой PATH не трогаем, при перезапуске
// шелла export'ы сработают штатно. Для текущего процесса PATH
// обновляет path_service::sync_process_path (set_var).

use std::path::PathBuf;

const RC_BEGIN: &str = "# StackPilot:begin";
const RC_END: &str = "# StackPilot:end";

/// Путь к rc-файлу: предпочтение по $SHELL, иначе любой существующий.
fn rc_path() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    let shell = std::env::var("SHELL").unwrap_or_default();
    let preferred = if shell.contains("zsh") {
        home.join(".zshrc")
    } else {
        home.join(".bashrc")
    };
    if preferred.exists() {
        return preferred;
    }
    home.join(".profile")
}

/// Записи PATH из блока StackPilot в rc-файле (пусто, если блока нет).
pub fn read_rc_user_path() -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(rc_path()) else {
        return Vec::new();
    };
    let Some(begin) = content.lines().position(|l| l == RC_BEGIN) else {
        return Vec::new();
    };
    content
        .lines()
        .skip(begin + 1)
        .find(|l| l.trim_start().starts_with("export PATH"))
        .map(|line| {
            line.split('"')
                .nth(1)
                .unwrap_or("")
                .split(':')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Заменяет (или создаёт) блок StackPilot в rc-файле.
pub fn write_rc_user_path(dirs: &[String]) -> Result<(), String> {
    let path = rc_path();
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    // «Подозрительные» символы убираем, чтобы запись не выполнила
    // код при подстановке в export (записи у нас свои, но защита лишней
    // не бывает).
    let safe: Vec<String> = dirs
        .iter()
        .map(|d| d.chars().filter(|c| *c != '"' && *c != '$' && *c != '`').collect())
        .collect();
    let block = format!("{RC_BEGIN}\nexport PATH=\"{}\"\n{RC_END}\n", safe.join(":"));

    let new_content = if let Some(start) = existing.find(RC_BEGIN) {
        let end = existing
            .find(RC_END)
            .map(|e| e + RC_END.len())
            .unwrap_or(existing.len());
        let mut s = String::with_capacity(existing.len());
        s.push_str(&existing[..start]);
        s.push_str(&block);
        s.push_str(&existing[end..]);
        s
    } else {
        let mut s = existing;
        if !s.ends_with('\n') {
            s.push('\n');
        }
        s.push_str(&block);
        s
    };

    std::fs::write(&path, new_content)
        .map_err(|e| format!("Не удалось записать {}: {e}", path.display()))
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

    /// Создаёт временный rc-файл и подменяет на него rc_path
    /// (rc_path читает HOME из окружения — переопределяем через env).
    fn with_rc(content: &str, f: impl FnOnce()) {
        let _guard = RC_TEST_LOCK.lock().unwrap();
        let home = std::env::temp_dir().join(format!("tc-rc-test-{}", std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&home).unwrap();
        let mut file = std::fs::File::create(home.join(".bashrc")).unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file.flush().unwrap();

        let old = std::env::var("HOME").ok();
        let old_shell = std::env::var("SHELL").ok();
        std::env::set_var("HOME", &home);
        std::env::set_var("SHELL", "/bin/bash");

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
        with_rc("export PATH=\"/usr/bin:/bin\"\n", || {
            assert!(read_rc_user_path().is_empty());
        });
    }

    #[test]
    fn read_parses_marker_block() {
        with_rc(
            "# other\n# StackPilot:begin\nexport PATH=\"/a:/b\"\n# StackPilot:end\n",
            || assert_eq!(read_rc_user_path(), vec!["/a".to_string(), "/b".to_string()]),
        );
    }

    #[test]
    fn write_replaces_marker_block() {
        with_rc(
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
    fn write_appends_block_when_absent() {
        with_rc("export FOO=bar\n", || {
            write_rc_user_path(&["/appended".to_string()]).unwrap();
            let content = std::fs::read_to_string(
                std::env::var("HOME").unwrap() + "/.bashrc",
            )
            .unwrap();
            assert!(content.contains("# StackPilot:begin"));
            assert!(content.contains("export PATH=\"/appended\""));
            assert!(content.starts_with("export FOO=bar\n"));
        });
    }

    #[test]
    fn write_strips_dangerous_chars() {
        // `"`, `$` и backtick вырезаются, чтобы запись не выполнила код
        with_rc("", || {
            write_rc_user_path(&["/a\"$(rm -rf /)".to_string()]).unwrap();
            assert_eq!(read_rc_user_path(), vec!["/a(rm -rf /)".to_string()]);
        });
    }
}
