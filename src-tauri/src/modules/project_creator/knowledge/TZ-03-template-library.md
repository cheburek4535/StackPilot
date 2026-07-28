# TZ-03: Template Engine + Content Library

## Контекст
У нас есть 3 связанных компонента:

1. **TemplateEngine** — движок подстановки `{{ variable }}` в строках (`engine/template.rs`, заглушка)
2. **Content generators** — функции, возвращающие строки для файлов (Dockerfile, .gitignore, README, CI, docker-compose)
3. **RenderTemplate + WriteFile шаги** — executor их выполняет, но контент для них нужно где-то брать

Суть задачи: написать TemplateEngine и библиотеку шаблонов/контента для всех поддерживаемых технологий.

## Где писать код

1. `src-tauri/src/modules/project_creator/engine/template.rs` — сам движок
2. `src-tauri/src/modules/project_creator/engine/content.rs` — **НОВЫЙ ФАЙЛ** с библиотекой контента

## Часть 1: TemplateEngine

Файл: `engine/template.rs`

### 1.1. `render()`

```rust
pub fn render(&self, template: &str, context: &HashMap<String, String>) -> String
```

**Алгоритм:**

1. Используй `regex::Regex::new(r"\{\{\s*(\w+)\s*\}\}")` для поиска всех `{{ key }}`
   - Добавь `regex = "1"` в `Cargo.toml`
2. Для каждого найденного совпадения:
   - Извлеки имя переменной (capture group 1)
   - Найди значение в `context`
   - Если ключ есть → замени всё вхождение `{{ key }}` на значение
   - Если ключа нет → **оставь как есть** (не заменяй на пустую строку — это позволит обнаружить missing variables)
3. Используй `regex::Regex::replace_all()` для однопроходной замены

**Пример:**
```rust
let engine = TemplateEngine::new();
let mut ctx = HashMap::new();
ctx.insert("project_name".into(), "myapp".into());
ctx.insert("language".into(), "Rust".into());

let result = engine.render("Project {{ project_name }} is written in {{ language }}", &ctx);
assert_eq!(result, "Project myapp is written in Rust");
```

### 1.2. Кэширование Regex

Regex-ы дорого компилировать. Сделай `lazy_static` или `once_cell` для кэширования:

```rust
use std::sync::LazyLock;
use regex::Regex;

static TEMPLATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{\{\s*(\w+)\s*\}\}").unwrap()
});
```

Или храни `Regex` как поле `TemplateEngine`:
```rust
pub struct TemplateEngine {
    re: Regex,
}
```

### 1.3. `load_template()`

Заглушка на будущее для загрузки из файлов. Пока просто возвращает `content.to_string()`.

### 1.4. Тесты

Добавь unit-тесты в конец файла:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_simple_substitution() { ... }
    #[test]
    fn test_missing_variable_kept() { ... }
    #[test]
    fn test_multiple_variables() { ... }
    #[test]
    fn test_no_variables() { ... }
}
```

## Часть 2: Content Library

**НОВЫЙ ФАЙЛ:** `src-tauri/src/modules/project_creator/engine/content.rs`

Модуль с функциями, возвращающими строковое содержимое для генерируемых файлов. Каждая функция принимает контекст проекта и возвращает готовый к записи текст.

### 2.1. Структура модуля

```rust
// engine/content.rs

use std::collections::HashMap;

/// Вернуть содержимое Dockerfile для языка/фреймворка
pub fn dockerfile(language: &str, framework: Option<&str>, tools: &[String]) -> String { ... }

/// Вернуть содержимое docker-compose.yml для выбранных tool-ов
pub fn docker_compose(tools: &[String], language: &str) -> String { ... }

/// Вернуть содержимое .gitignore
pub fn gitignore(languages: &[String]) -> String { ... }

/// Вернуть содержимое README.md
pub fn readme(project_name: &str, context: &WizardContext) -> String { ... }

/// Вернуть содержимое .dockerignore
pub fn dockerignore(language: &str) -> String { ... }

/// Вернуть содержимое .editorconfig
pub fn editorconfig() -> &'static str { ... }

/// Вернуть настройки VS Code (settings.json)
pub fn vscode_settings(language: &str, framework: Option<&str>) -> String { ... }

/// Вернуть рекомендации расширений VS Code (extensions.json)
pub fn vscode_extensions(language: &str) -> String { ... }

/// Вернуть CI конфиг (GitHub Actions)
pub fn github_ci(language: &str, framework: Option<&str>) -> String { ... }
```

### 2.2. Dockerfile шаблоны

**Rust (multi-stage):**
```dockerfile
FROM rust:1-slim-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
WORKDIR /app
COPY --from=builder /app/target/release/{{project_name}} /app/{{project_name}}
EXPOSE 8080
CMD ["/app/{{project_name}}"]
```

**Node.js:**
```dockerfile
FROM node:20-alpine AS builder
WORKDIR /app
COPY package*.json ./
RUN npm ci
COPY . .
RUN npm run build

FROM node:20-alpine
WORKDIR /app
COPY --from=builder /app/dist ./dist
COPY --from=builder /app/node_modules ./node_modules
COPY package*.json ./
EXPOSE 3000
CMD ["node", "dist/main.js"]
```

**Python:**
```dockerfile
FROM python:3.12-slim
WORKDIR /app
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt
COPY . .
EXPOSE 8000
CMD ["uvicorn", "main:app", "--host", "0.0.0.0", "--port", "8000"]
```

**Go:**
```dockerfile
FROM golang:1.22-alpine AS builder
WORKDIR /app
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -o /app/server

FROM alpine:3.19
WORKDIR /app
COPY --from=builder /app/server .
EXPOSE 8080
CMD ["/app/server"]
```

### 2.3. docker-compose шаблон

Генерируется динамически: для каждого выбранного tool-а добавляется сервис.

```yaml
version: "3.8"
services:
  app:
    build: .
    ports:
      - "${PORT:-8080}:8080"
    environment:
      - DATABASE_URL=postgres://user:pass@postgres:5432/db
    depends_on:
      - postgres
      - redis

  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_USER: user
      POSTGRES_PASSWORD: pass
      POSTGRES_DB: db
    ports:
      - "5432:5432"
    volumes:
      - pgdata:/var/lib/postgresql/data

  redis:
    image: redis:7-alpine
    ports:
      - "6379:6379"

volumes:
  pgdata:
```

**Какие сервисы добавлять для каждого tool:**
- `postgresql` → сервис postgres
- `redis` → сервис redis
- `kafka` → сервисы zookeeper + kafka
- `clickhouse` → сервис clickhouse
- `mongodb` → сервис mongo
- `mysql` → сервис mysql
- `rabbitmq` → сервис rabbitmq
- `minio` → сервис minio
- `mailpit` → сервис mailpit
- `grafana` → сервис grafana
- `opentelemetry` → сервис otel-collector

### 2.4. .gitignore

Верни многострочную строку с правилами для переданных языков:

```
# Rust
/target
Cargo.lock

# Node
node_modules/
.env
.env.local

# Python
__pycache__/
*.pyc
.venv/
*.egg-info/

# Go
*.exe
*.exe~

# IDE
.idea/
.vscode/
*.swp

# OS
.DS_Store
Thumbs.db
```

### 2.5. README

Markdown-шаблон с секциями:
```markdown
# {{project_name}}

{{description}}

## Tech Stack
- **Language:** {{language}}
- **Framework:** {{framework}}
- **Tools:** {{tools}}

## Getting Started

### Prerequisites
- {{language}} (see version in .tool-versions)
- Docker (optional)

### Installation
```bash
git clone https://github.com/your/repo.git
cd {{project_name}}
# install deps
```

### Development
```bash
# run with hot-reload
```

### Docker
```bash
docker compose up -d
```

## Project Structure
```

### 2.6. CI (GitHub Actions)

```yaml
name: CI
on: [push, pull_request]
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-{{language}}@v1
      - run: {{build_command}}
      - run: {{test_command}}
```

Для каждого языка свои `setup-*` action и команды.

## Требования

1. **TemplateEngine не должен падать** — если regex не нашёл совпадений, вернуть строку как есть
2. **Content functions должны быть чистыми** — никакого I/O, только строки на входе и выходе
3. **Все Dockerfile шаблоны должны быть production-ready**: multi-stage, slim images, non-root user где возможно
4. **docker-compose должен быть валидным YAML** — проверь через `serde_yaml` или руками
5. **README должен быть полезным** — не просто заглушка, а реальная документация с инструкциями
6. **Все функции должны быть покрыты тестами** — особенно TemplateEngine

## Критерии приёмки

1. `TemplateEngine::render()` корректно заменяет `{{ key }}` на значения из context
2. `render()` оставляет `{{ missing_key }}` без изменений если ключа нет
3. `dockerfile("Rust", None, &[])` возвращает корректный multi-stage Dockerfile
4. `docker_compose(&["postgresql", "redis"], "rust")` возвращает YAML с postgres и redis сервисами
5. `gitignore(&["rust"])` содержит `/target` и `Cargo.lock`
6. `readme("myapp", &context)` возвращает markdown с именем проекта в заголовке
7. `github_ci("rust", Some("actix-web"))` возвращает YAML с `cargo build` и `cargo test`
8. Все содержимое валидно: Dockerfile парсится, YAML парсится, JSON парсится
