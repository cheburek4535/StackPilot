# 🤖 Задание для ИИ-агента: Подготовка StackPilot к релизу

## 📋 Контекст проекта

Проект: **StackPilot v1.2.1**  
Тип: Desktop-приложение на Tauri 2 + SvelteKit  
Репозиторий: `C:\Users\Cheburek\VSProjects\StackPilot`  
Язык backend: Rust  
Язык frontend: TypeScript + Svelte 5  

Проект представляет собой кроссплатформенное приложение для автоматизации создания проектов, управления окружением разработки и быстрого запуска проектов.

## 🎯 Общая цель

Исправить **5 критических блокеров** и **5 высокоприоритетных проблем**, препятствующих релизу проекта. После выполнения всех задач проект должен быть готов к публичному релизу с точки зрения безопасности, стабильности и соответствия заявленным стандартам.

---

## 🚨 ЗАДАЧА 2: Исправить хардкоженные пароли в генераторе проектов

### Приоритет: КРИТИЧЕСКИЙ (security issue)
### Время выполнения: 3-4 часа

### Описание проблемы:
Файл `src-tauri/src/modules/project_creator/engine/content.rs` содержит хардкоженные пароли, которые попадают в сгенерированные проекты:
- `POSTGRES_PASSWORD=12345`
- `MYSQL_ROOT_PASSWORD=root_pwd`
- `MYSQL_PASSWORD=root_pwd`
- `AIRFLOW__WEBSERVER__SECRET_KEY` без значения

Это серьезная проблема безопасности: все проекты, созданные через StackPilot, будут иметь одинаковые небезопасные пароли.

### Что нужно сделать:

#### Шаг 1: Добавить генератор безопасных паролей

Создать файл `src-tauri/src/modules/project_creator/utils/password_gen.rs`:

```rust
use rand::{thread_rng, Rng};
use rand::distributions::Alphanumeric;

/// Генерирует криптографически безопасный случайный пароль
pub fn generate_secure_password(length: usize) -> String {
    thread_rng()
        .sample_iter(&Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

/// Генерирует пароль с обязательным включением спецсимволов
pub fn generate_complex_password(length: usize) -> String {
    const SPECIAL_CHARS: &[u8] = b"!@#$%^&*-_=+";
    const DIGITS: &[u8] = b"0123456789";
    const LOWERCASE: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    const UPPERCASE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    
    let mut rng = thread_rng();
    let mut password = Vec::new();
    
    // Гарантируем хотя бы по одному символу каждого типа
    password.push(SPECIAL_CHARS[rng.gen_range(0..SPECIAL_CHARS.len())]);
    password.push(DIGITS[rng.gen_range(0..DIGITS.len())]);
    password.push(LOWERCASE[rng.gen_range(0..LOWERCASE.len())]);
    password.push(UPPERCASE[rng.gen_range(0..UPPERCASE.len())]);
    
    // Заполняем оставшуюся длину случайными символами
    let all_chars = [SPECIAL_CHARS, DIGITS, LOWERCASE, UPPERCASE].concat();
    for _ in 0..(length - 4) {
        password.push(all_chars[rng.gen_range(0..all_chars.len())]);
    }
    
    // Перемешиваем
    for i in (1..password.len()).rev() {
        let j = rng.gen_range(0..=i);
        password.swap(i, j);
    }
    
    String::from_utf8(password).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_password_length() {
        let pwd = generate_secure_password(16);
        assert_eq!(pwd.len(), 16);
    }
    
    #[test]
    fn test_passwords_are_different() {
        let pwd1 = generate_secure_password(20);
        let pwd2 = generate_secure_password(20);
        assert_ne!(pwd1, pwd2);
    }
    
    #[test]
    fn test_complex_password_has_required_chars() {
        let pwd = generate_complex_password(16);
        assert!(pwd.chars().any(|c| c.is_lowercase()));
        assert!(pwd.chars().any(|c| c.is_uppercase()));
        assert!(pwd.chars().any(|c| c.is_numeric()));
        assert!(pwd.chars().any(|c| "!@#$%^&*-_=+".contains(c)));
    }
}
```

#### Шаг 2: Добавить зависимость rand в Cargo.toml

Отредактировать `src-tauri/Cargo.toml`, добавить в секцию `[dependencies]`:
```toml
rand = "0.8"
```

#### Шаг 3: Зарегистрировать модуль

В `src-tauri/src/modules/project_creator/utils/mod.rs` добавить:
```rust
pub mod password_gen;
```

Если файла `mod.rs` нет, создать его:
```rust
pub mod password_gen;
```

И в `src-tauri/src/modules/project_creator/mod.rs` добавить:
```rust
pub mod utils;
```

#### Шаг 4: Исправить content.rs

Отредактировать `src-tauri/src/modules/project_creator/engine/content.rs`:

1. Добавить импорт в начало файла:
```rust
use crate::modules::project_creator::utils::password_gen::generate_complex_password;
```

2. Найти все строки с хардкоженными паролями и заменить:

**Было:**
```rust
"postgres" => r#"POSTGRES_USER=postgres
POSTGRES_PASSWORD=12345
POSTGRES_DB=mydb"#,
```

**Стало:**
```rust
"postgres" => {
    let db_password = generate_complex_password(20);
    format!(
        r#"# ВАЖНО: Сгенерированный пароль уникален для этого проекта
# В production обязательно измените пароли через secrets management
POSTGRES_USER=postgres
POSTGRES_PASSWORD={}
POSTGRES_DB=mydb"#,
        db_password
    )
},
```

**Было:**
```rust
"mysql" => r#"MYSQL_ROOT_PASSWORD=root_pwd
MYSQL_DATABASE=mydb
MYSQL_USER=user
MYSQL_PASSWORD=root_pwd"#,
```

**Стало:**
```rust
"mysql" => {
    let root_password = generate_complex_password(20);
    let user_password = generate_complex_password(20);
    format!(
        r#"# ВАЖНО: Сгенерированные пароли уникальны для этого проекта
# В production обязательно измените пароли через secrets management
MYSQL_ROOT_PASSWORD={}
MYSQL_DATABASE=mydb
MYSQL_USER=user
MYSQL_PASSWORD={}"#,
        root_password,
        user_password
    )
},
```

**Для Airflow:**
```rust
"airflow" => {
    let secret_key = generate_complex_password(32);
    format!(
        r#"AIRFLOW__CORE__FERNET_KEY=generated_on_first_run
AIRFLOW__WEBSERVER__SECRET_KEY={}
AIRFLOW__CORE__LOAD_EXAMPLES=False"#,
        secret_key
    )
},
```

3. Найти и исправить аналогично ВСЕ места с хардкоженными паролями в этом файле (их около 10).

#### Шаг 5: Добавить предупреждение в README сгенерированных проектов

В функции генерации README (вероятно в `src-tauri/src/modules/project_creator/engine/readme.rs`) добавить секцию:

```markdown
## ⚠️ Безопасность

**ВАЖНО:** StackPilot автоматически сгенерировал уникальные пароли для баз данных в файле `.env`. 

Перед deployment в production:
1. Измените все пароли на более надежные через систему secrets management
2. НЕ коммитьте файл `.env` в git
3. Используйте переменные окружения или vault (HashiCorp Vault, AWS Secrets Manager)
4. Регулярно ротируйте пароли

Сгенерированные пароли предназначены только для локальной разработки!
```

#### Шаг 6: Тестирование

Запустить тесты:
```bash
cd /sessions/great-vibrant-sagan/mnt/StackPilot/src-tauri
cargo test password_gen
cargo test audit_no_hardcoded_secrets
```

Создать тестовый проект и проверить:
```bash
# Запустить приложение и создать проект с PostgreSQL
# Проверить что в .env пароли разные и не "12345"
cat <путь_к_созданному_проекту>/.env
```

### Критерии выполнения:
- ✅ Модуль `password_gen.rs` создан и работает
- ✅ Зависимость `rand` добавлена в Cargo.toml
- ✅ Все хардкоженные пароли заменены на генерируемые
- ✅ В .env файлы добавлены предупреждения о безопасности
- ✅ README сгенерированных проектов содержит security-секцию
- ✅ Тесты проходят
- ✅ Вручную создан тестовый проект и проверены пароли
- ✅ Изменения закоммичены: `git commit -m "Security: Replace hardcoded passwords with generated ones"`

---

## 🚨 ЗАДАЧА 3: Включить Content Security Policy (CSP)

### Приоритет: КРИТИЧЕСКИЙ (security issue)
### Время выполнения: 30-60 минут

### Описание проблемы:
В `src-tauri/tauri.conf.json` строка 21 содержит `"csp": null`, что полностью отключает Content Security Policy. Это делает приложение уязвимым к XSS-атакам и injection-векторам.

### Что нужно сделать:

#### Шаг 1: Определить требуемые CSP-директивы

Проанализировать какие ресурсы загружает фронтенд:
```bash
cd /sessions/great-vibrant-sagan/mnt/StackPilot
# Поиск внешних ресурсов
grep -r "https://" src/routes --include="*.svelte" --include="*.ts"
grep -r "http://" src/routes --include="*.svelte" --include="*.ts"
grep -r "cdn\." src --include="*.svelte" --include="*.ts" --include="*.html"
```

#### Шаг 2: Настроить CSP в tauri.conf.json

Отредактировать `src-tauri/tauri.conf.json`, заменить строку 21:

**Было:**
```json
"security": {
  "csp": null
}
```

**Стало:**
```json
"security": {
  "csp": {
    "default-src": "'self'",
    "script-src": [
      "'self'",
      "'unsafe-inline'"
    ],
    "style-src": [
      "'self'",
      "'unsafe-inline'"
    ],
    "img-src": [
      "'self'",
      "data:",
      "https:"
    ],
    "font-src": [
      "'self'",
      "data:"
    ],
    "connect-src": [
      "'self'",
      "http://localhost:*",
      "ws://localhost:*",
      "http://127.0.0.1:*",
      "ws://127.0.0.1:*"
    ],
    "frame-src": "'none'",
    "object-src": "'none'"
  }
}
```

**Обоснование директив:**
- `'unsafe-inline'` в script-src и style-src нужны для Svelte (генерирует inline стили)
- `data:` в img-src для base64-изображений
- `https:` в img-src для загрузки иконок инструментов из интернета
- `localhost` и `127.0.0.1` в connect-src для WebSocket DevServer
- `frame-src: none` и `object-src: none` для дополнительной защиты

#### Шаг 3: Протестировать CSP

1. Собрать приложение:
```bash
cd /sessions/great-vibrant-sagan/mnt/StackPilot
npm run tauri build
```

2. Запустить и проверить консоль браузера (DevTools):
   - Открыть приложение
   - Нажать Ctrl+Shift+I (или Cmd+Option+I на Mac)
   - Проверить что нет CSP-ошибок в консоли
   - Пройтись по всем разделам: Project Creator, DevLauncher, Toolchain, Workspace

3. Если есть CSP-ошибки типа "Refused to load...", добавить нужные источники в CSP

#### Шаг 4: Документировать CSP

Создать файл `SECURITY.md` в корне проекта:

```markdown
# Security Policy

## Content Security Policy

StackPilot использует строгую Content Security Policy для защиты от XSS-атак:

- Разрешены только локальные скрипты (`'self'`)
- Inline стили разрешены для Svelte компонентов
- Изображения могут загружаться из HTTPS источников (для иконок инструментов)
- WebSocket соединения ограничены localhost (для dev-сервера)
- Запрещены фреймы и плагины

### Если нужно расширить CSP

Отредактируйте `src-tauri/tauri.conf.json`, секцию `app.security.csp`.

## Reporting Security Issues

Если вы обнаружили уязвимость в StackPilot:

1. **НЕ создавайте публичный GitHub Issue**
2. Отправьте письмо на: [создать email для security]
3. Опишите уязвимость и шаги для воспроизведения
4. Мы ответим в течение 48 часов

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 1.2.x   | ✅ Активная поддержка |
| < 1.2   | ❌ Не поддерживается |

## Security Best Practices

При использовании StackPilot:

1. Не запускайте непроверенные профили запуска из неизвестных источников
2. Проверяйте сгенерированные проекты перед использованием в production
3. Измените дефолтные пароли, сгенерированные для баз данных
4. Используйте последнюю версию приложения
5. Регулярно обновляйте установленные инструменты через Toolchain

## Known Security Considerations

- StackPilot выполняет команды от имени текущего пользователя
- Профили запуска содержат исполняемые команды — проверяйте их перед импортом
- Генератор проектов создает файлы с правами текущего пользователя

## Аудиты безопасности

- 2026-09-09: Внутренний аудит перед релизом v1.2.1
```

### Критерии выполнения:
- ✅ CSP настроен в tauri.conf.json
- ✅ Приложение собирается без ошибок
- ✅ Все разделы UI работают (нет CSP-блокировок)
- ✅ Файл SECURITY.md создан
- ✅ Изменения закоммичены: `git commit -m "Security: Enable Content Security Policy"`

---

## 🚨 ЗАДАЧА 4: Убрать критические .unwrap() из production-кода

### Приоритет: КРИТИЧЕСКИЙ (stability issue)
### Время выполнения: 6-8 часов

### Описание проблемы:
В кодовой базе найдено 1122 использования `.unwrap()`, `panic!`, `todo!`, `unimplemented!` в 62 файлах. Это приводит к аварийному завершению приложения при ошибках вместо graceful degradation.

### Стратегия:
Мы **НЕ** будем исправлять все 1122 случая (это нереалистично за один день). Вместо этого исправим критические пути, которые затрагивают пользовательский опыт.

### Что нужно сделать:

#### Шаг 1: Идентифицировать критические модули

Критические пути (пользователь видит краши):
1. `src-tauri/src/modules/devlauncher/orchestrator.rs` (167 unwrap) - запуск проектов
2. `src-tauri/src/modules/toolchain/core/installer.rs` (56 unwrap) - установка инструментов
3. `src-tauri/src/modules/project_creator/engine/mod.rs` - генерация проектов
4. `src-tauri/src/modules/devlauncher/profile_manager.rs` (48 unwrap) - работа с профилями
5. `src-tauri/src/modules/workspace/process_manager.rs` - управление процессами

#### Шаг 2: Создать утилиты для обработки ошибок

Создать файл `src-tauri/src/core/error_handling.rs`:

```rust
use serde::{Deserialize, Serialize};

/// Унифицированный тип ошибки для Tauri commands
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppError {
    pub message: String,
    pub code: String,
    pub details: Option<String>,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.details = Some(details.into());
        self
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)?;
        if let Some(details) = &self.details {
            write!(f, ": {}", details)?;
        }
        Ok(())
    }
}

impl std::error::Error for AppError {}

/// Конверсия из std::io::Error
impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::new("IO_ERROR", err.to_string())
    }
}

/// Конверсия из serde_json::Error
impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::new("JSON_ERROR", err.to_string())
    }
}

/// Тип результата для Tauri commands
pub type AppResult<T> = Result<T, AppError>;

/// Макрос для создания ошибок
#[macro_export]
macro_rules! app_error {
    ($code:expr, $msg:expr) => {
        $crate::core::error_handling::AppError::new($code, $msg)
    };
    ($code:expr, $msg:expr, $details:expr) => {
        $crate::core::error_handling::AppError::new($code, $msg).with_details($details)
    };
}
```

В `src-tauri/src/core/mod.rs` добавить:
```rust
pub mod error_handling;
```

#### Шаг 3: Исправить orchestrator.rs

Отредактировать `src-tauri/src/modules/devlauncher/orchestrator.rs`:

Найти функции которые возвращают `()` или `Result<(), String>` и изменить на `AppResult<()>`.

**Пример исправления:**

**Было:**
```rust
fn start_process(&self, step: &Step) {
    let child = Command::new(&step.command)
        .spawn()
        .unwrap(); // ❌ ПАНИКА при ошибке
    // ...
}
```

**Стало:**
```rust
fn start_process(&self, step: &Step) -> AppResult<()> {
    let child = Command::new(&step.command)
        .spawn()
        .map_err(|e| app_error!(
            "PROCESS_START_FAILED",
            format!("Failed to start process: {}", step.command),
            e.to_string()
        ))?;
    // ...
    Ok(())
}
```

**Для операций с файлами:**

**Было:**
```rust
let content = std::fs::read_to_string(&path).unwrap();
```

**Стало:**
```rust
let content = std::fs::read_to_string(&path)
    .map_err(|e| app_error!(
        "FILE_READ_ERROR",
        format!("Cannot read file: {}", path.display()),
        e.to_string()
    ))?;
```

**Для JSON парсинга:**

**Было:**
```rust
let config: Config = serde_json::from_str(&json).unwrap();
```

**Стало:**
```rust
let config: Config = serde_json::from_str(&json)
    .map_err(|e| app_error!(
        "CONFIG_PARSE_ERROR",
        "Failed to parse configuration",
        e.to_string()
    ))?;
```

#### Шаг 4: Исправить installer.rs

Аналогично orchestrator.rs, но фокус на:
- Проверка прав доступа перед установкой
- Обработка ошибок скачивания
- Валидация checksum

**Пример:**

**Было:**
```rust
fn download_installer(url: &str) -> PathBuf {
    let response = reqwest::blocking::get(url).unwrap();
    // ...
}
```

**Стало:**
```rust
fn download_installer(url: &str) -> AppResult<PathBuf> {
    let response = reqwest::blocking::get(url)
        .map_err(|e| app_error!(
            "DOWNLOAD_FAILED",
            format!("Cannot download from: {}", url),
            e.to_string()
        ))?;
    
    if !response.status().is_success() {
        return Err(app_error!(
            "HTTP_ERROR",
            format!("Server returned error: {}", response.status())
        ));
    }
    // ...
    Ok(path)
}
```

#### Шаг 5: Обновить Tauri commands

Все публичные Tauri commands должны возвращать `Result<T, AppError>`:

**Было:**
```rust
#[tauri::command]
fn create_project(ctx: ProjectContext) -> String {
    // ... код с unwrap()
    "success".to_string()
}
```

**Стало:**
```rust
#[tauri::command]
fn create_project(ctx: ProjectContext) -> AppResult<String> {
    // ... код с правильной обработкой ошибок
    Ok("success".to_string())
}
```

#### Шаг 6: Обновить frontend для обработки ошибок

В `src/lib/core/tauriMock.ts` и других местах вызова Tauri commands добавить обработку:

```typescript
import { invoke } from '@tauri-apps/api/core';

interface AppError {
  message: string;
  code: string;
  details?: string;
}

async function createProject(context: any): Promise<string> {
  try {
    return await invoke<string>('create_project', { ctx: context });
  } catch (error) {
    const appError = error as AppError;
    console.error(`[${appError.code}] ${appError.message}`);
    if (appError.details) {
      console.error('Details:', appError.details);
    }
    // Показать пользователю дружелюбное сообщение
    throw new Error(appError.message);
  }
}
```

#### Шаг 7: Добавить логирование вместо eprintln!

Найти все `eprintln!` и заменить на proper logging:

Добавить в `Cargo.toml`:
```toml
log = "0.4"
env_logger = "0.11"
```

В `src-tauri/src/main.rs`:
```rust
fn main() {
    env_logger::init();
    stackpilot_lib::run()
}
```

Заменить:
```rust
eprintln!("Error: {}", err); // ❌
```

На:
```rust
log::error!("Error: {}", err); // ✅
log::warn!("Warning: unexpected state");
log::info!("Process started");
log::debug!("Config loaded: {:?}", config);
```

### Тестирование:

```bash
cd /sessions/great-vibrant-sagan/mnt/StackPilot/src-tauri

# Проверить что компилируется
cargo check

# Запустить тесты
cargo test

# Собрать в release
cargo build --release
```

Вручную протестировать:
1. Создание проекта с невалидными данными
2. Установка инструмента без прав администратора
3. Запуск проекта с несуществующим профилем
4. Проверить что приложение не крашится, а показывает ошибку

### Критерии выполнения:
- ✅ Модуль error_handling.rs создан
- ✅ Критические unwrap() заменены в 5 модулях
- ✅ Все Tauri commands возвращают AppResult
- ✅ eprintln! заменены на log::error
- ✅ Frontend обрабатывает ошибки от backend
- ✅ Приложение собирается без ошибок
- ✅ Тесты проходят
- ✅ Вручную протестированы error-сценарии
- ✅ Изменения закоммичены: `git commit -m "Improve error handling in critical paths"`

---

## 🚨 ЗАДАЧА 5: Зафиксировать версии зависимостей

### Приоритет: КРИТИЧЕСКИЙ (reproducible builds)
### Время выполнения: 1 час

### Описание проблемы:
В `src-tauri/Cargo.toml` используются неточные версии зависимостей (например, `serde = "1"`), что может привести к непредсказуемым изменениям поведения при обновлениях.

### Что нужно сделать:

#### Шаг 1: Проверить текущие версии

```bash
cd /sessions/great-vibrant-sagan/mnt/StackPilot/src-tauri
cargo tree --depth 1
```

#### Шаг 2: Обновить Cargo.toml

Отредактировать `src-tauri/Cargo.toml`, заменить секцию dependencies:

**Было:**
```toml
[dependencies]
tauri = { version = "2", features = [] }
tauri-plugin-opener = "2"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tauri-plugin-dialog = "2"
async-trait = "0.1"
tokio = { version = "1", features = ["process", "io-util", "sync", "time", "rt-multi-thread", "macros"] }
webbrowser = "1.0"
toml = "1.1"
serde_yaml = "0.9"
regex = "1"
walkdir = "2"
which = "8"
libc = "0.2"
chrono = "0.4"
notify = "8.2"
uuid = { version = "1", features = ["v4"] }
url = "2"
```

**Стало:**
```toml
[dependencies]
tauri = { version = "2.1", features = [] }
tauri-plugin-opener = "2.0"
serde = { version = "1.0.215", features = ["derive"] }
serde_json = "1.0.133"
tauri-plugin-dialog = "2.0"
async-trait = "0.1.83"
tokio = { version = "1.41", features = ["process", "io-util", "sync", "time", "rt-multi-thread", "macros"] }
webbrowser = "1.0.2"
toml = "0.8.19"
serde_yaml = "0.9.34"
regex = "1.11"
walkdir = "2.5"
which = "8.0"
libc = "0.2.164"
chrono = { version = "0.4.38", default-features = false, features = ["clock", "std"] }
notify = "8.2.1"
uuid = { version = "1.11", features = ["v4", "serde"] }
url = "2.5"
rand = "0.8.5"
log = "0.4.22"
env_logger = "0.11.5"
```

**Обоснование:**
- Зафиксированы минорные версии для предсказуемости
- Добавлены `rand`, `log`, `env_logger` из предыдущих задач
- Для `chrono` отключены ненужные features для уменьшения размера

#### Шаг 3: Обновить Cargo.lock

```bash
cd /sessions/great-vibrant-sagan/mnt/StackPilot/src-tauri
cargo update
cargo build --release
```

#### Шаг 4: Зафиксировать frontend-зависимости

В `package.json` проверить что версии точные (не `^` и не `~`):

```bash
cd /sessions/great-vibrant-sagan/mnt/StackPilot
# Если есть ^ или ~, удалить их:
sed -i 's/"^/"/' package.json
sed -i 's/"~/"/' package.json
```

Обновить lockfile:
```bash
npm install
```

#### Шаг 5: Документировать

Создать файл `.cargo/config.toml`:
```toml
[build]
# Всегда использовать locked версии
locked = true
```

В README.md добавить секцию:

```markdown
## Dependency Management

Проект использует зафиксированные версии зависимостей для воспроизводимых сборок.

### Обновление зависимостей

```bash
# Backend (Rust)
cd src-tauri
cargo update
cargo test

# Frontend (Node.js)
npm update
npm test
```

Перед коммитом обновлений проверьте что все тесты проходят!
```

### Критерии выполнения:
- ✅ Версии в Cargo.toml зафиксированы
- ✅ Cargo.lock обновлен
- ✅ Версии в package.json не содержат ^ или ~
- ✅ package-lock.json обновлен
- ✅ Проект собирается без warning'ов о версиях
- ✅ Изменения закоммичены: `git commit -m "Pin dependency versions for reproducible builds"`

---

## ⚠️ ЗАДАЧА 6: Удалить debug-вывод из production-кода

### Приоритет: ВЫСОКИЙ
### Время выполнения: 2 часа

### Описание проблемы:
В коде присутствует 16+ файлов с `eprintln!`, `dbg!` (Rust) и `console.log` (TypeScript), которые не должны быть в production.

### Что нужно сделать:

#### Rust код:

Найти все dbg! и удалить или заменить:

```bash
cd /sessions/great-vibrant-sagan/mnt/StackPilot/src-tauri
grep -rn "dbg!" src/ --include="*.rs"
```

Для каждого найденного:

**Было:**
```rust
dbg!(config);
dbg!(&step.command);
```

**Стало:**
```rust
log::debug!("Config: {:?}", config);
log::debug!("Executing command: {}", step.command);
```

#### TypeScript код:

Найти все console.log:

```bash
cd /sessions/great-vibrant-sagan/mnt/StackPilot
grep -rn "console\." src/ --include="*.ts" --include="*.svelte"
```

Создать `src/lib/utils/logger.ts`:

```typescript
const isDev = import.meta.env.DEV;

export const logger = {
  debug: (...args: any[]) => {
    if (isDev) {
      console.debug('[DEBUG]', ...args);
    }
  },
  info: (...args: any[]) => {
    if (isDev) {
      console.info('[INFO]', ...args);
    }
  },
  warn: (...args: any[]) => {
    console.warn('[WARN]', ...args);
  },
  error: (...args: any[]) => {
    console.error('[ERROR]', ...args);
  }
};
```

Заменить:
```typescript
console.log('Creating project...'); // ❌
```

На:
```typescript
import { logger } from '$lib/utils/logger';
logger.debug('Creating project...'); // ✅
```

### Критерии выполнения:
- ✅ Все `dbg!()` удалены или заменены на `log::debug!`
- ✅ `eprintln!` заменены на `log::error!` или `log::warn!`
- ✅ Создан модуль logger.ts для frontend
- ✅ `console.log` заменены на logger.debug (кроме tauriMock.ts - там можно оставить)
- ✅ В production-сборке нет вывода в консоль
- ✅ Изменения закоммичены: `git commit -m "Remove debug output from production code"`

---

## ⚠️ ЗАДАЧА 7: Документировать unsafe блоки

### Приоритет: ВЫСОКИЙ
### Время выполнения: 2-3 часа

### Описание проблемы:
6 модулей используют `unsafe`, но без документации почему это безопасно.

Файлы:
- `src-tauri/src/modules/devlauncher/profile_manager.rs`
- `src-tauri/src/modules/workspace/process_manager.rs`
- `src-tauri/src/modules/workspace/process_supervisor.rs`
- `src-tauri/src/modules/toolchain/core/crypto.rs`
- `src-tauri/src/modules/toolchain/core/secrets.rs`
- `src-tauri/src/modules/project_creator/engine/mod.rs`

### Что нужно сделать:

Для каждого файла:

1. Найти все unsafe блоки:
```bash
grep -A 5 "unsafe" <файл>
```

2. Добавить комментарий ПЕРЕД каждым unsafe:

```rust
// SAFETY: <объяснение почему это безопасно>
unsafe {
    // код
}
```

**Примеры:**

```rust
// SAFETY: PID валиден и процесс существует, проверено выше через sysinfo
unsafe {
    libc::kill(pid as i32, libc::SIGTERM);
}

// SAFETY: Указатель получен из валидной Rust-строки и не null
unsafe {
    let c_str = CString::new(path).expect("path contains null byte");
    // ...
}

// SAFETY: Память выделена через alloc и размер корректен
unsafe {
    let buffer = std::slice::from_raw_parts_mut(ptr, len);
    // ...
}
```

3. Если не можете обосновать безопасность - попробовать переписать без unsafe

4. Создать файл `docs/unsafe-audit.md`:

```markdown
# Аудит unsafe кода в StackPilot

Дата: 2026-09-09
Версия: 1.2.1

## Обзор

Проект использует unsafe в 6 модулях для низкоуровневых операций с ОС.
Все блоки проверены и задокументированы.

## Использование unsafe по модулям

### process_manager.rs
- **Локация:** src/modules/workspace/process_manager.rs:142
- **Цель:** Отправка SIGTERM процессу
- **Обоснование:** PID проверен через sysinfo, процесс существует
- **Альтернатива:** Нет safe API для Unix signals

### crypto.rs
- **Локация:** src/modules/toolchain/core/crypto.rs:67
- **Цель:** SHA256 checksum через OpenSSL
- **Обоснование:** Используем проверенную библиотеку sha2
- **Риски:** Минимальные
- **TODO:** Мигрировать на pure-Rust sha2 crate (убрать unsafe)

[документировать остальные модули]

## Проверки безопасности

- [ ] Нет null pointer dereference
- [ ] Нет buffer overflow
- [ ] Нет use-after-free
- [ ] Нет data races (все unsafe в single-threaded context или под мьютексом)

## Рекомендации

1. Минимизировать unsafe в будущих PR
2. Каждый новый unsafe требует code review
3. Предпочитать safe abstractions
```

### Критерии выполнения:
- ✅ Все unsafe блоки имеют SAFETY комментарии
- ✅ Создан docs/unsafe-audit.md
- ✅ Нет неоправданных unsafe (где есть safe альтернатива)
- ✅ Изменения закоммичены: `git commit -m "Document all unsafe blocks"`

---

## ⚠️ ЗАДАЧА 8: Создать CHANGELOG.md

### Приоритет: ВЫСОКИЙ
### Время выполнения: 1 час

### Что нужно сделать:

Создать файл `CHANGELOG.md` в корне проекта:

```markdown
# Changelog

Все значимые изменения в проекте документируются в этом файле.

Формат основан на [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
проект следует [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.2.1] - 2026-09-09

### Added
- Project Creator: визуальный конструктор для создания проектов
  - Поддержка 10+ типов проектов (REST API, Full-stack, Desktop, CLI, etc.)
  - 70+ технологий и инструментов
  - Система проверки совместимости технологий
  - Preview структуры проекта перед генерацией
- DevLauncher: быстрый запуск проектов
  - Автоматический анализ существующих проектов
  - Графовая система зависимостей между шагами
  - Поддержка long-running сервисов и one-shot команд
  - Мониторинг состояния процессов
- Toolchain Manager: управление программным окружением
  - Автоматическое обнаружение установленных инструментов
  - Установка через winget (Windows), Homebrew (macOS), apt/dnf/pacman (Linux)
  - Health-check для каждого инструмента
  - Docker как альтернатива локальной установке
- Workspace: мониторинг запущенных проектов
  - Просмотр логов в реальном времени
  - Управление процессами (start/stop/restart)
  - Файловый браузер
  - Отслеживание времени работы
- Поддержка русского и английского языков
- Светлая и тёмная темы
- Onboarding для новых пользователей

### Changed
- Мигрировано на Svelte 5 с runes
- Обновлено до Tauri 2.1
- Улучшена производительность анализа проектов

### Security
- Добавлен Content Security Policy (CSP)
- Генерация уникальных паролей для баз данных вместо хардкоженных
- Улучшена обработка ошибок в критических путях
- Зафиксированы версии зависимостей для воспроизводимых сборок

### Fixed
- Исправлены memory leaks в file watcher
- Улучшена стабильность process supervisor
- Исправлено определение Python-проектов с pyproject.toml

## [1.2.0] - 2026-08-15

### Added
- Первая alpha-версия
- Базовый функционал создания проектов

## [1.1.0] - 2026-07-01

### Added
- Proof of concept
- Прототип UI

## [1.0.0] - 2026-06-01

### Added
- Начало разработки
- Базовая архитектура Tauri + Svelte

[1.2.1]: https://github.com/cheburek4535/StackPilot/releases/tag/v1.2.1
[1.2.0]: https://github.com/cheburek4535/StackPilot/releases/tag/v1.2.0
[1.1.0]: https://github.com/cheburek4535/StackPilot/releases/tag/v1.1.0
[1.0.0]: https://github.com/cheburek4535/StackPilot/releases/tag/v1.0.0
```

### Критерии выполнения:
- ✅ Файл CHANGELOG.md создан
- ✅ Описаны все major features версии 1.2.1
- ✅ Добавлена Security секция
- ✅ Изменения закоммичены: `git commit -m "Add CHANGELOG.md"`

---

## ⚠️ ЗАДАЧА 9: Настроить GitHub Actions CI/CD

### Приоритет: ВЫСОКИЙ
### Время выполнения: 2-3 часа

### Что нужно сделать:

#### Шаг 1: Создать структуру

```bash
mkdir -p .github/workflows
```

#### Шаг 2: Создать .github/workflows/ci.yml

```yaml
name: CI

on:
  push:
    branches: [ main, develop ]
  pull_request:
    branches: [ main ]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  test-rust:
    name: Test Rust (Backend)
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest, macos-latest]
    
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      
      - name: Cache cargo registry
        uses: actions/cache@v4
        with:
          path: ~/.cargo/registry
          key: ${{ runner.os }}-cargo-registry-${{ hashFiles('**/Cargo.lock') }}
      
      - name: Cache cargo build
        uses: actions/cache@v4
        with:
          path: src-tauri/target
          key: ${{ runner.os }}-cargo-build-${{ hashFiles('**/Cargo.lock') }}
      
      - name: Check formatting
        run: cd src-tauri && cargo fmt --all -- --check
      
      - name: Run clippy
        run: cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings
      
      - name: Run tests
        run: cd src-tauri && cargo test --verbose

  test-frontend:
    name: Test Frontend
    runs-on: ubuntu-latest
    
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '22'
          cache: 'npm'
      
      - name: Install dependencies
        run: npm ci
      
      - name: Run type check
        run: npm run check
      
      - name: Run tests
        run: npm test
      
      - name: Build
        run: npm run build

  security-audit:
    name: Security Audit
    runs-on: ubuntu-latest
    
    steps:
      - uses: actions/checkout@v4
      
      - name: Rust security audit
        uses: rustsec/audit-check@v1
        with:
          token: ${{ secrets.GITHUB_TOKEN }}
      
      - name: npm audit
        run: npm audit --audit-level=high
```

#### Шаг 3: Создать .github/workflows/build.yml

```yaml
name: Build

on:
  push:
    tags:
      - 'v*'
  workflow_dispatch:

jobs:
  build:
    name: Build - ${{ matrix.platform }}
    runs-on: ${{ matrix.os }}
    
    strategy:
      fail-fast: false
      matrix:
        include:
          - platform: windows-x86_64
            os: windows-latest
            target: x86_64-pc-windows-msvc
          
          - platform: macos-x86_64
            os: macos-latest
            target: x86_64-apple-darwin
          
          - platform: macos-aarch64
            os: macos-latest
            target: aarch64-apple-darwin
          
          - platform: linux-x86_64
            os: ubuntu-22.04
            target: x86_64-unknown-linux-gnu
    
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      
      - name: Install Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '22'
          cache: 'npm'
      
      - name: Install Linux dependencies
        if: matrix.platform == 'linux-x86_64'
        run: |
          sudo apt-get update
          sudo apt-get install -y libgtk-3-dev libwebkit2gtk-4.1-dev librsvg2-dev
      
      - name: Install frontend dependencies
        run: npm ci
      
      - name: Build frontend
        run: npm run build
      
      - name: Build Tauri app
        uses: tauri-apps/tauri-action@v0
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          tagName: ${{ github.ref_name }}
          releaseName: 'StackPilot ${{ github.ref_name }}'
          releaseBody: 'See CHANGELOG.md for details'
          releaseDraft: true
          prerelease: false
          args: --target ${{ matrix.target }}
      
      - name: Upload artifacts
        uses: actions/upload-artifact@v4
        with:
          name: stackpilot-${{ matrix.platform }}
          path: src-tauri/target/${{ matrix.target }}/release/bundle/*
```

#### Шаг 4: Создать .github/workflows/release.yml

```yaml
name: Release

on:
  push:
    tags:
      - 'v*'

permissions:
  contents: write

jobs:
  create-release:
    name: Create Release
    runs-on: ubuntu-latest
    
    steps:
      - uses: actions/checkout@v4
      
      - name: Extract version from tag
        id: version
        run: echo "VERSION=${GITHUB_REF#refs/tags/v}" >> $GITHUB_OUTPUT
      
      - name: Extract changelog
        id: changelog
        run: |
          VERSION=${{ steps.version.outputs.VERSION }}
          CHANGELOG=$(sed -n "/## \[$VERSION\]/,/## \[/p" CHANGELOG.md | sed '$d')
          echo "CHANGELOG<<EOF" >> $GITHUB_OUTPUT
          echo "$CHANGELOG" >> $GITHUB_OUTPUT
          echo "EOF" >> $GITHUB_OUTPUT
      
      - name: Create GitHub Release
        uses: softprops/action-gh-release@v1
        with:
          name: StackPilot v${{ steps.version.outputs.VERSION }}
          body: ${{ steps.changelog.outputs.CHANGELOG }}
          draft: false
          prerelease: false
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

#### Шаг 5: Добавить badge в README.md

В начало `README.md` после заголовка:

```markdown
[![CI Status](https://github.com/cheburek4535/StackPilot/workflows/CI/badge.svg)](https://github.com/cheburek4535/StackPilot/actions)
[![Build Status](https://github.com/cheburek4535/StackPilot/workflows/Build/badge.svg)](https://github.com/cheburek4535/StackPilot/actions)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0--only-blue.svg)](LICENSE)
```

### Критерии выполнения:
- ✅ Созданы 3 workflow файла (ci.yml, build.yml, release.yml)
- ✅ CI запускается на PR и коммитах
- ✅ Build workflow корректно настроен для всех платформ
- ✅ Badge добавлены в README
- ✅ Изменения закоммичены: `git commit -m "Add GitHub Actions CI/CD"`

---

## ⚠️ ЗАДАЧА 10: Финальная очистка и подготовка к релизу

### Приоритет: ВЫСОКИЙ
### Время выполнения: 2 часа

### Что нужно сделать:

#### Шаг 1: Обновить package.json

Добавить description:
```json
{
  "name": "stackpilot",
  "version": "1.2.1",
  "description": "Cross-platform desktop app for automated project setup and development environment management",
  "author": "cheburek4535",
  "license": "AGPL-3.0-only",
  "repository": {
    "type": "git",
    "url": "https://github.com/cheburek4535/StackPilot.git"
  },
  "bugs": {
    "url": "https://github.com/cheburek4535/StackPilot/issues"
  },
  "homepage": "https://github.com/cheburek4535/StackPilot#readme"
}
```

#### Шаг 2: Закоммитить все изменения

```bash
cd /sessions/great-vibrant-sagan/mnt/StackPilot

# Проверить статус
git status

# Добавить все новые файлы
git add LICENSE CHANGELOG.md SECURITY.md .github/ docs/

# Закоммитить
git commit -m "chore: prepare for v1.2.1 release

- Add LICENSE file (AGPL-3.0)
- Replace hardcoded passwords with secure generation
- Enable Content Security Policy
- Improve error handling in critical paths
- Pin dependency versions
- Remove debug output
- Document unsafe blocks
- Add CHANGELOG.md and SECURITY.md
- Setup GitHub Actions CI/CD
- Update package.json metadata"
```

#### Шаг 3: Создать Git тег

```bash
git tag -a v1.2.1 -m "Release version 1.2.1

Major improvements:
- Security hardening (CSP, password generation)
- Improved error handling
- Reproducible builds
- CI/CD automation"

git push origin main
git push origin v1.2.1
```

#### Шаг 4: Создать файл .gitattributes

```bash
cat > .gitattributes << 'EOF'
# Auto detect text files and perform LF normalization
* text=auto

# Rust
*.rs text diff=rust

# TypeScript/JavaScript
*.ts text
*.js text
*.svelte text
*.json text

# Documentation
*.md text
*.txt text

# Binary files
*.png binary
*.jpg binary
*.ico binary
*.icns binary
*.woff2 binary
EOF

git add .gitattributes
git commit -m "Add .gitattributes for consistent line endings"
```

#### Шаг 5: Проверить что можно собрать релиз

```bash
# Backend
cd src-tauri
cargo clean
cargo build --release
cargo test --release

# Frontend
cd ..
npm ci
npm run build
npm test

# Полная сборка Tauri
npm run tauri build
```

#### Шаг 6: Создать release checklist

Создать файл `docs/RELEASE_CHECKLIST.md`:

```markdown
# Release Checklist

Используйте этот чеклист перед каждым релизом.

## Pre-release

- [ ] Все тесты проходят (`cargo test` и `npm test`)
- [ ] Нет compiler warnings
- [ ] Версия обновлена в:
  - [ ] package.json
  - [ ] src-tauri/Cargo.toml
  - [ ] src-tauri/tauri.conf.json
- [ ] CHANGELOG.md обновлен
- [ ] SECURITY.md актуален
- [ ] README.md отражает текущую функциональность
- [ ] Нет TODO/FIXME в критическом коде
- [ ] Все PR смержены
- [ ] Git статус чистый

## Security

- [ ] Нет хардкоженных секретов
- [ ] CSP настроен
- [ ] Зависимости обновлены (cargo audit, npm audit)
- [ ] Unsafe блоки задокументированы
- [ ] Входные данные валидируются

## Build

- [ ] Windows сборка работает
- [ ] macOS сборка работает (x64 и ARM)
- [ ] Linux сборка работает
- [ ] Installers создаются корректно
- [ ] Иконки и ресурсы на месте
- [ ] Размер бинарника приемлемый (< 100MB)

## Testing

- [ ] Manual smoke test на каждой платформе
- [ ] Project Creator создает рабочие проекты
- [ ] DevLauncher запускает проекты
- [ ] Toolchain обнаруживает инструменты
- [ ] Workspace отображает процессы
- [ ] Локализация работает
- [ ] Темы переключаются

## Release

- [ ] Git тег создан (vX.Y.Z)
- [ ] GitHub Release опубликован
- [ ] Changelog скопирован в release notes
- [ ] Бинарники прикреплены к релизу
- [ ] SHA256 checksums добавлены
- [ ] Release анонсирован (если применимо)

## Post-release

- [ ] Мониторинг GitHub Issues
- [ ] Обновить documentation
- [ ] Начать следующий спринт
```

### Критерии выполнения:
- ✅ package.json заполнен
- ✅ Все изменения закоммичены
- ✅ Git тег создан
- ✅ .gitattributes добавлен
- ✅ Релиз собирается без ошибок
- ✅ docs/RELEASE_CHECKLIST.md создан

---

## 📊 ФИНАЛЬНАЯ ПРОВЕРКА

После выполнения всех 10 задач выполни следующие проверки:

### Автоматизированные проверки:

```bash
cd /sessions/great-vibrant-sagan/mnt/StackPilot

# 1. Проверить что LICENSE существует
test -f LICENSE && echo "✅ LICENSE exists" || echo "❌ LICENSE missing"

# 2. Проверить что нет хардкоженных паролей
! grep -r "POSTGRES_PASSWORD=12345" src-tauri/src && echo "✅ No hardcoded passwords" || echo "❌ Found hardcoded passwords"

# 3. Проверить что CSP настроен
grep -q '"csp":' src-tauri/tauri.conf.json && echo "✅ CSP configured" || echo "❌ CSP not configured"

# 4. Проверить что есть CHANGELOG
test -f CHANGELOG.md && echo "✅ CHANGELOG exists" || echo "❌ CHANGELOG missing"

# 5. Проверить что есть SECURITY.md
test -f SECURITY.md && echo "✅ SECURITY.md exists" || echo "❌ SECURITY.md missing"

# 6. Проверить что есть CI/CD
test -f .github/workflows/ci.yml && echo "✅ CI configured" || echo "❌ CI missing"

# 7. Проверить что зависимости зафиксированы
grep -q 'serde = { version = "1.0' src-tauri/Cargo.toml && echo "✅ Dependencies pinned" || echo "❌ Dependencies not pinned"

# 8. Проверить что нет console.log (кроме logger.ts и tauriMock.ts)
! grep -r "console\.log" src --include="*.ts" --include="*.svelte" | grep -v "logger.ts\|tauriMock.ts" && echo "✅ No console.log" || echo "❌ Found console.log"

# 9. Проверить что Git чистый
[[ -z $(git status -s) ]] && echo "✅ Git clean" || echo "❌ Uncommitted changes"

# 10. Проверить что тег создан
git tag | grep -q "v1.2.1" && echo "✅ Git tag created" || echo "❌ Git tag missing"

# 11. Сборка проходит
npm run build && echo "✅ Build successful" || echo "❌ Build failed"

# 12. Тесты проходят
npm test && cd src-tauri && cargo test && cd .. && echo "✅ All tests pass" || echo "❌ Tests failed"
```

### Ручные проверки:

1. **Запустить приложение:**
```bash
npm run tauri dev
```

2. **Проверить функциональность:**
- [ ] Создать новый проект (Project Creator)
- [ ] Проверить что пароли в .env уникальные и сложные
- [ ] Запустить проект (DevLauncher)
- [ ] Проверить что при ошибке показывается понятное сообщение (а не краш)
- [ ] Открыть Toolchain и просканировать систему
- [ ] Открыть Workspace и проверить мониторинг
- [ ] Переключить язык
- [ ] Переключить тему

3. **Проверить безопасность:**
- [ ] Открыть DevTools (Ctrl+Shift+I), проверить что нет CSP-ошибок
- [ ] Проверить что нет debug-вывода в консоли
- [ ] Попробовать создать проект с некорректными данными - должна показаться ошибка
- [ ] Проверить что секреты не логируются

4. **Проверить документацию:**
- [ ] README.md актуален
- [ ] CHANGELOG.md содержит v1.2.1
- [ ] SECURITY.md читабелен
- [ ] LICENSE корректен

---

## 📝 ОТЧЕТ О ВЫПОЛНЕНИИ

После завершения всех задач создай файл `RELEASE_AUDIT_REPORT.md`:

```markdown
# Release Audit Report - v1.2.1

**Дата:** 2026-09-09  
**Исполнитель:** [ИМЯ AI АГЕНТА]  
**Статус:** [ГОТОВ/НЕ ГОТОВ] к релизу

## Выполненные задачи


### ✅ ЗАДАЧА 2: Безопасные пароли
- [x] Модуль password_gen.rs создан
- [x] Все хардкоженные пароли заменены
- [x] Тесты проходят
- [x] Закоммичен

[продолжить для всех 10 задач]

## Метрики

- **Исправлено unwrap():** X из 1122
- **Удалено eprintln!:** X из 16
- **Удалено console.log:** X из Y
- **Задокументировано unsafe:** 6 из 6
- **Время выполнения:** X часов

## Проблемы

### Критические (блокируют релиз):
- [ ] Нет

### Некритические:
- [ ] [Описание проблемы]

## Рекомендации

1. [Что еще можно улучшить]
2. [Что делать после релиза]

## Финальная оценка

**Проект готов к релизу:** ДА / НЕТ  
**Комментарий:** [краткая оценка]

---

**Подпись:** [ИМЯ]  
**Дата:** [ДАТА]
```

---

## 🚀 ПОСЛЕ ВЫПОЛНЕНИЯ ВСЕХ ЗАДАЧ

**Уведоми maintainer'а:**
   ```
   @cheburek4535 Проект готов к релизу v1.2.1.
   
   Выполнено:
   - ✅ Все критические блокеры исправлены
   - ✅ Security hardening завершен
   - ✅ CI/CD настроен
   - ✅ Документация обновлена
   - ✅ Сборка проходит на всех платформах
   
   Смотри RELEASE_AUDIT_REPORT.md для деталей.
   ```

---

## ⚙️ КОНФИГУРАЦИЯ ДЛЯ AI АГЕНТА


**Приоритет задач:**
1. ЗАДАЧИ 1-5 (критические) - выполнить обязательно
2. ЗАДАЧИ 6-10 (высокие) - желательно выполнить
3. Остальное - по возможности

**В случае блокировки:**
- Если задача не может быть выполнена - документируй причину
- Если нужно решение maintainer'а - создай GitHub Issue
- Если есть альтернативный подход - опиши его в комментарии

---

## 📚 ДОПОЛНИТЕЛЬНЫЕ РЕСУРСЫ

- [Tauri Security Best Practices](https://tauri.app/v1/references/architecture/security/)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [SvelteKit Best Practices](https://kit.svelte.dev/docs/best-practices)
- [OWASP Secure Coding Practices](https://owasp.org/www-project-secure-coding-practices-quick-reference-guide/)

---

