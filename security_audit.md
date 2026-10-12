# StackPilot Security Audit & Remediation Plan

**ВНИМАНИЕ ДЛЯ АГЕНТА:** Этот документ описывает критические уязвимости безопасности (Security Vulnerabilities) на границе между WebView (Frontend) и Core (Rust/Tauri) в проекте StackPilot. **Исправление этих уязвимостей имеет наивысший приоритет**, так как приложение работает с файловой системой пользователя.

---

## 1. КРИТИЧЕСКАЯ УЯЗВИМОСТЬ: Arbitrary File Read/Write (Path Traversal)
**Где:** 
- `src-tauri/src/modules/workspace/file_explorer.rs`
- `src-tauri/src/modules/workspace/commands.rs`

**Описание проблемы:**
Кастомные команды Tauri (`read_file`, `write_file`, `list_directory`) принимают строковый параметр `path` напрямую из WebView и передают его в стандартную библиотеку Rust (`std::fs::read_to_string`, `std::fs::write`) без **абсолютно никакой проверки границ (Sandboxing)**.

```rust
// Уязвимый код в file_explorer.rs:
fn read_file(&self, path: &str) -> Result<FileContent, String> {
    let content = fs::read_to_string(path).map_err(...)?; // Читает любой путь в системе
    // ...
}
```

**Сценарий атаки:**
Если во фронтенд внедрен вредоносный JavaScript (например, через XSS при рендере `README.md` чужого проекта, либо через скомпрометированный плагин), этот скрипт может выполнить:
```javascript
import { invoke } from "@tauri-apps/api/core";
// Чтение приватного SSH ключа:
const sshKey = await invoke("read_file", { path: "C:/Users/Alex/.ssh/id_rsa" });
// Запись в системные файлы / автозагрузку:
await invoke("write_file", { path: "C:/Users/.../AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup/malware.bat", content: "..." });
```
Так как это кастомная команда, встроенные ограничения Tauri (Scope Restrictions) для плагина `@tauri-apps/plugin-fs` **здесь не работают**.

**План лечения (Remediation):**
1. В модуле `workspace` внедрить строгий механизм **Path Sandboxing**. 
2. У `WorkspaceState` есть понятие текущего проекта (`project.get_current()`).
3. Создать утилиту валидации пути (в `file_explorer.rs` или `core/utils.rs`):
   ```rust
   fn resolve_and_verify_path(project_root: &Path, user_path: &str) -> Result<PathBuf, String> {
       let target = Path::new(user_path);
       // Если путь абсолютный, проверяем, что он начинается с project_root
       // Если относительный, комбинируем и проверяем
       let combined = project_root.join(target);
       let canonical_target = combined.canonicalize().map_err(|_| "Invalid path")?;
       let canonical_root = project_root.canonicalize().map_err(|_| "Invalid root")?;
       
       if !canonical_target.starts_with(&canonical_root) {
           return Err("Access Denied: Path Traversal detected".into());
       }
       Ok(canonical_target)
   }
   ```
4. Внедрить эту валидацию во **все** функции `FileExplorerService`. Если открытого проекта нет, доступ к `read_file`/`write_file` должен быть полностью запрещен.

## 2. Изоляция рабочих директорий профилей (DevLauncher)
**Где:** `src-tauri/src/modules/devlauncher/commands.rs` -> `resolve_working_dir`
**Замечание по безопасности:**
В данный момент логика `resolve_working_dir` разрешает абсолютные пути (`if path.is_relative() { ... }`). Поскольку сами профили (`LaunchProfile`) хранятся в доверенной зоне (`%APPDATA%`) и управляются пользователем, это архитектурно допустимо. 
Однако, если в будущем StackPilot начнет загружать профили напрямую из папки `.stackpilot/profiles.json` из репозиториев (чужих Workspace проектов), это станет вектором для RCE (Remote Code Execution) вне песочницы.
**Рекомендация:** Строго документировать, что `devlauncher` профили загружаются только из доверенных системных путей, а не из несанитизированных директорий пользователя.

---
*Действуй осторожно и обязательно покрой измененный слой File Explorer юнит-тестами, имитирующими Path Traversal атаки (`../../`, `C:/Windows`, и т.д.).*
