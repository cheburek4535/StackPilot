# TZ-01: Recipe Composition Engine

## Контекст
Wizard собрал выбор пользователя — `WizardContext` с языком, фреймворком, инструментами, фичами. Теперь нужно **сконвертировать этот набор в конкретный план действий**: какие папки создать, какие файлы записать, какие команды выполнить, в каком порядке.

Это **самый важный файл во всём ProjectCreator**. Он определяет что именно создаётся для каждого сочетания технологий.

## Где писать код

Файл: `src-tauri/src/modules/project_creator/engine/mod.rs`
Функция: `fn compose_recipe(context: &WizardContext) -> Result<Recipe, String>`

Сейчас там заглушка — несколько шагов для примера. Нужно написать полноценную логику.

## Что нужно сделать

### 1. Структурировать композицию по фазам

Каждый рецепт состоит из последовательных фаз. Порядок важен:

```
Phase 1: Project init
  - Создать корневую директорию (уже есть)
  - Инициализировать язык: cargo init / npm init / go mod init / dotnet new
  - Это всегда командный шаг (Step::Command)

Phase 2: Framework setup  
  - Для React/Vue/Svelte/Angular: create-vite / create-react-app / ng new
  - Для Django: django-admin startproject
  - Для Tauri: cargo tauri init
  - Для Actix/Axum/Rocket: добавить зависимости в Cargo.toml
  - Это команды + запись файлов

Phase 3: Dependencies
  - npm install / cargo build / pip install
  - Только если нужны реальные пакеты

Phase 4: Docker
  - Dockerfile (RenderTemplate с контекстом)
  - docker-compose.yml (если выбраны БД/кеш/инфра)
  - .dockerignore

Phase 5: Config files
  - .gitignore (WriteFile с контентом под язык)
  - .editorconfig
  - VS Code settings (.vscode/extensions.json, .vscode/settings.json)
  - CI config (.github/workflows/ci.yml или .gitlab-ci.yml)

Phase 6: Git
  - git init + git add + git commit (опционально)

Phase 7: README
  - README.md с описанием проекта
```

### 2. Определить вспомогательные функции-генераторы

Для каждой технологии нужен свой набор контента. Создай отдельные функции, возвращающие `Vec<Step>`:

```
fn steps_for_language(lang: &str, project_name: &str) -> Vec<Step>
fn steps_for_framework(fw: &str, lang: &str) -> Vec<Step>
fn steps_for_tools(tools: &[String], features: &WizardContext) -> Vec<Step>
fn steps_for_docker(context: &WizardContext) -> Vec<Step>
fn steps_for_git(context: &WizardContext) -> Vec<Step>
fn steps_for_ci(context: &WizardContext) -> Vec<Step>
fn steps_for_vscode() -> Vec<Step>
fn readme_content(context: &WizardContext) -> String
fn gitignore_content(langs: &[String]) -> String
fn dockerfile_template(lang: &str, framework: Option<&str>) -> String
```

Это даст чистую модульную структуру, где каждая функция отвечает за свою часть.

### 3. Наполнить steps_for_language

Для каждого языка добавить шаг инициализации:

| Язык | Команда | Примечание |
|------|---------|------------|
| Rust | `cargo init --name {name}` | рабочий каталог — project_path |
| TypeScript/JavaScript | `mkdir src && echo '{}' > package.json` | потом npm install |
| Python | `mkdir src && echo '' > pyproject.toml` | или requirements.txt |
| Go | `go mod init {name}` | |
| Java | `mvn archetype:generate ...` | сложно, можно заглушку |
| C# | `dotnet new console/webapi` | |
| C/C++ | `mkdir src include` | ручная структура |
| Zig | `zig init-exe` | |

### 4. Наполнить steps_for_framework

Добавить зависимости и конфиги для каждого фреймворка:

**Rust:**
- Tauri: `cargo tauri init`, добавить tauri в зависимости
- Actix/Axum/Rocket: добавить Cargo.toml dependencies
- Yew/Dioxus/Leptos: добавить wasm-pack, настройки

**JavaScript/TypeScript:**
- React: `npm create vite@latest -- --template react-ts`
- Vue: `npm create vue@latest`
- Svelte: `npm create svelte@latest`
- Next.js: `npx create-next-app@latest`
- NestJS: `npx @nestjs/cli new`
- Express: ручная структура + npm install express

**Python:**
- FastAPI: `pip install fastapi uvicorn` + main.py
- Django: `django-admin startproject`
- Flask: ручная структура + app.py

### 5. Наполнить steps_for_tools

Для каждого инструмента добавить Docker сервис в docker-compose.yml:

| Tool | Docker service |
|------|---------------|
| PostgreSQL | image: postgres:16, port 5432 |
| Redis | image: redis:7, port 6379 |
| Kafka | image: confluentinc/cp-kafka, zookeeper |
| ClickHouse | image: clickhouse/clickhouse-server |
| MongoDB | image: mongo:7 |
| MySQL | image: mysql:8 |
| RabbitMQ | image: rabbitmq:3-management |
| MinIO | image: minio/minio |
| Mailpit | image: axllent/mailpit |

### 6. Наполнить steps_for_docker

Если `context.docker == true`:
- Создать `Dockerfile` через `RenderTemplate` с языковым шаблоном
- Если выбраны tool-ы с БД/кешем — создать `docker-compose.yml` с этими сервисами
- Создать `.dockerignore`

### 7. Наполнить gitignore_content

Вернуть многострочную строку с правилами для выбранных языков:
- Rust: `/target`, `Cargo.lock`
- Node: `node_modules/`, `.env`
- Python: `__pycache__/`, `.venv`, `*.pyc`
- Go: `*.exe`, `*.exe~`
- Общие: `.DS_Store`, `.idea`, `*.log`

### 8. Wrapper: подставить project_name

Извлеки имя проекта из `context.project_path` (последний компонент пути). Используй `Path::new(&path).file_name().unwrap_or("project")`.

## Требования к реализации

1. **Каждый шаг должен иметь уникальный `id`** — можно использовать формат `"{phase}_{index}"` или короткие хэши
2. **Все `on_error` должны быть осмысленными**: `Abort` для критических шагов (init, зависимости), `Skip` для опциональных (readme, vscode)
3. **Используй `StepCondition`** для опциональных шагов — например, `FeatureEnabled { feature: "docker" }` для Docker-шагов
4. **RenderTemplate шаги** должны включать `context` с переменными: `{"project_name": "...", "language": "...", "framework": "..."}` — эти переменные потом подставит TemplateEngine
5. **Команды должны быть кроссплатформенными** — для Windows используй `cmd /c` или проверяй `cfg!(target_os = "windows")`

## Архитектурные заметки

- `compose_recipe()` вызывается из `RecipeEngine::plan()`, результат кэшируется в `ExecutionPlan`
- Шаги потом фильтруются через `flatten_and_filter()` которая раскрывает Parallel и проверяет condition
- Не бойся создавать много шагов — executor их выполнит последовательно и покажет прогресс
- Каждый шаг должен иметь человекочитаемый `label` — он показывается пользователю

## Критерии приёмки

1. Для Rust + Actix + PostgreSQL + Redis + Docker → создаётся Cargo проект с Actix в dependencies, docker-compose с postgres и redis, Dockerfile multi-stage, .gitignore, README
2. Для Python + FastAPI + no Docker → pyproject.toml, main.py с FastAPI, requirements.txt, .gitignore, README
3. Для TypeScript + React + Docker → Vite проект, Dockerfile для node, .gitignore, README
4. Если Docker=true → всегда есть Dockerfile и .dockerignore
5. Если docker-compose не нужен (нет tool-ов с сервисами) → создаётся только Dockerfile
6. Все шаги имеют осмысленные id, label, description
