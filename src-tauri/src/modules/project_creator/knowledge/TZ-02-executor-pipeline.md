# TZ-02: Execution Pipeline

## Контекст
Recipe Engine составил `ExecutionPlan` с конкретными шагами. Теперь их нужно **выполнить**: запустить shell-команды, записать файлы, отрендерить шаблоны, создать директории — и всё это с live-стримингом логов на фронтенд через `mpsc::Sender<ExecutionEvent>`.

Сейчас все методы `StepExecutor` — заглушки, возвращающие `StepStatus::Skipped`. Нужно написать реальную имплементацию.

## Где писать код

Файл: `src-tauri/src/modules/project_creator/engine/executor.rs`

Четыре метода:
- `run_command()`
- `write_file()`
- `render_template()`
- `create_directory()`

Плюс вспомогательные утилиты.

## 1. Метод `run_command()`

```rust
pub async fn run_command(
    &self,
    step: &Step,
    plan: &ExecutionPlan,
    tx: &mpsc::Sender<ExecutionEvent>,
    index: usize,
) -> StepResult
```

**Назначение:** Выполнить shell-команду с потоковым выводом.

**Алгоритм:**

1. Извлеки параметры из `Step::Command`:
   - `command` — исполняемый файл
   - `args` — аргументы
   - `working_dir` — рабочий каталог (относительно `plan.project_path`)
   - `env` — дополнительные переменные окружения
   - `timeout_secs` — таймаут (опционально)

2. Создай `tokio::process::Command`:
   ```rust
   let mut cmd = tokio::process::Command::new(&command);
   cmd.args(args)
      .current_dir(full_working_dir)
      .envs(extra_env);
   ```

3. Настрой stdout/stderr как piped:
   ```rust
   cmd.stdout(std::process::Stdio::piped());
   cmd.stderr(std::process::Stdio::piped());
   ```

4. Спавн процесса:
   ```rust
   let mut child = cmd.spawn()?;
   ```

5. Читай stdout и stderr построчно в параллельных тасках:
   ```rust
   let stdout = child.stdout.take().unwrap();
   let stderr = child.stderr.take().unwrap();
   
   let reader = BufReader::new(stdout);
   let mut lines = reader.lines();
   while let Some(line) = lines.next_line().await? {
       tx.send(ExecutionEvent {
           event_type: ExecutionEventType::StepProgress {
               stdout: line,
               stderr: String::new(),
           },
           ...
       }).await.ok();
   }
   ```

   **Важно:** Используй `tokio::io::BufReader` и `tokio::io::AsyncBufReadExt` для асинхронного чтения строк. Не используй `std::io::BufReader`.

6. Дождись завершения:
   ```rust
   let status = child.wait().await?;
   ```

7. Если `status.success()`, верни `StepStatus::Success { message }`. Иначе `StepStatus::Failed { error: stderr_output }`.

8. Если установлен `timeout_secs`, оберни всё в `tokio::time::timeout()`:
   ```rust
   tokio::time::timeout(Duration::from_secs(timeout), async {
       // spawn + read + wait
   }).await.map_err(|_| "Timeout exceeded")?
   ```

**Что нужно импортировать:**
```rust
use tokio::process::Command as TokioCommand;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::time::{timeout, Duration};
```

**Платформенные особенности:**
- На Windows команды вроде `cargo init` работают напрямую
- Для shell-специфичных конструкций (`&&`, `|`) нужно запускать через `cmd /c` на Windows или `sh -c` на Unix
- Можно определить: `let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" }` и `let shell_arg = if cfg!(target_os = "windows") { "/C" } else { "-c" }`

**Обработка ошибок:**
- Если spawn вернул ошибку (команда не найдена) → `Failed { error: "command not found: {name}" }`
- Если процесс завершился с ненулевым кодом → `Failed { error: stderr + exit_code }`
- Если таймаут → `Failed { error: "timeout after {n}s" }`
- Если tx.send() вернул ошибку (канал закрыт) → игнорируй (пользователь ушёл со страницы)

**Отправка событий:**
- `StepProgress` — после каждой прочитанной строки stdout/stderr
- `StepCompleted` — после завершения (успех/ошибка)

## 2. Метод `write_file()`

```rust
pub async fn write_file(
    &self,
    step: &Step,
    plan: &ExecutionPlan,
    tx: &mpsc::Sender<ExecutionEvent>,
    index: usize,
) -> StepResult
```

**Назначение:** Записать строковое содержимое в файл.

**Алгоритм:**

1. Извлеки `path`, `content`, `overwrite` из `Step::WriteFile`
2. Полный путь: `plan.project_path.join(path)`
3. Создай родительскую директорию: `fs::create_dir_all(parent)`
4. Проверь `overwrite`:
   - Если файл существует и `overwrite == false` → `Skipped { reason: "File exists, overwrite disabled" }`
   - Если файл существует и `overwrite == true` → предупреди в логе и перезапиши
   - Если файла нет → создай
5. Запиши: `fs::write(full_path, &content)?`
6. Отправь `StepProgress` с `stdout: "Wrote {path}"`
7. Верни `StepStatus::Success { message: "Wrote {path}" }`

**Важно:**
- Используй `std::fs` (синхронный) — запись файлов быстрая, не нужно async
- Все пути должны быть абсолютными — join с `plan.project_path`
- При ошибке создания директории верни `Failed` с описанием

## 3. Метод `render_template()`

```rust
pub async fn render_template(
    &self,
    step: &Step,
    plan: &ExecutionPlan,
    tx: &mpsc::Sender<ExecutionEvent>,
    index: usize,
    template_engine: &TemplateEngine,
) -> StepResult
```

**Назначение:** Отрендерить шаблон с подстановкой переменных и записать результат.

**Алгоритм:**

1. Извлеки `path`, `template`, `context`, `overwrite` из `Step::RenderTemplate`
2. Вызови `template_engine.render(&template, &context)` — получи готовый контент
3. Запиши результат как в `write_file` (можно вызвать `self.write_file()`)
4. Отправь `StepProgress`: `stdout: "Rendered {path}"`

**Обработка ошибок:**
- Если template_engine.render() вернул пустую строку (и template не пустой) → warning в лог
- Если запись файла не удалась → такой же `Failed` как в write_file

## 4. Метод `create_directory()`

**Назначение:** Создать директорию (рекурсивно).

Уже реализован как заглушка — нужно только проверить что работает корректно и отправляет `StepProgress`:

```rust
let _ = tx.send(ExecutionEvent {
    event_type: ExecutionEventType::StepProgress {
        stdout: format!("Created {}", full_path.display()),
        stderr: String::new(),
    },
    ...
}).await;
```

## 5. Вспомогательные функции

Добавь в конец файла утилиты:

```rust
fn project_path(plan: &ExecutionPlan, relative: &str) -> PathBuf {
    plan.project_path.join(relative)
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir {}: {}", parent.display(), e))
    } else { Ok(()) }
}

fn current_timestamp() -> String {
    // Используй chrono::Local::now().to_rfc3339() или примитивную заглушку
    "now".to_string()
}
```

## Критерии приёмки

1. `run_command` с командой `echo "hello"` → step завершается Success, в логах есть строка "hello"
2. `run_command` с несуществующей командой → step завершается Failed с сообщением
3. `run_command` с timeout=1s на команду `sleep 10` → step завершается Failed с "timeout"
4. `write_file` создаёт новый файл с контентом → файл существует, контент совпадает
5. `write_file` с existing file + overwrite=false → step Skipped
6. `write_file` с существующим файлом + overwrite=true → файл перезаписан
7. `render_template` с `"Hello {{ name }}"` и `{"name": "World"}` → в файле `"Hello World"`
8. `create_directory` создаёт вложенную структуру `a/b/c` → директория существует
9. Все события отправляются в правильном порядке: Started → Progress (0+ раз) → Completed
10. При ошибке в run_command отправляется Completed с Failed до того, как executor переходит к следующему шагу
