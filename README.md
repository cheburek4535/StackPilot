# StackPilot

<p align="center">
  <img src="static/images/logo-name.svg" alt="StackPilot" width="360">
</p>

> Локальное desktop-workspace для разработчика: соберите окружение, создайте или проанализируйте проект, запустите весь стек одной кнопкой и наблюдайте за процессами в одном месте.

[![License: AGPL-3.0-only](https://img.shields.io/badge/license-AGPL--3.0--only-blue.svg)](https://www.gnu.org/licenses/agpl-3.0.html)
[![Tauri 2](https://img.shields.io/badge/Tauri-2.x-24c8db.svg)](https://tauri.app/)
[![SvelteKit](https://img.shields.io/badge/SvelteKit-2.x-ff3e00.svg)](https://kit.svelte.dev/)

StackPilot объединяет терминал, менеджер пакетов, IDE и набор скриптов в одном локальном приложении. Он помогает анализировать существующие проекты, создавать новые каркасы, готовить toolchain, запускать весь стек одной кнопкой и видеть процессы, логи и ошибки.

> **Статус проекта:** активная open-source разработка.

---

## Содержание

- [Что такое StackPilot](#что-такое-stackpilot)
- [Кому он нужен](#кому-он-нужен)
- [Возможности](#возможности)
- [Как устроен рабочий цикл](#как-устроен-рабочий-цикл)
- [Быстрый старт для пользователя](#быстрый-старт-для-пользователя)
- [Модули приложения](#модули-приложения)
  - [DevLauncher](#devlauncher)
  - [Анализ проекта](#анализ-проекта)
  - [Project Creator](#project-creator)
    - [Система совместимости инструментов](#система-совместимости-инструментов)
  - [Toolchain](#toolchain)
  - [Workspace](#workspace)
  - [Настройки](#настройки)
- [Поддерживаемые проекты и инструменты](#поддерживаемые-проекты-и-инструменты)
- [Профили запуска V2](#профили-запуска-v2)
- [Окружения проекта и bindings](#окружения-проекта-и-bindings)
- [Платформенная модель](#платформенная-модель)
- [Установка готового приложения](#установка-готового-приложения)
- [Сборка из исходников](#сборка-из-исходников)
- [Разработка интерфейса](#разработка-интерфейса)
- [Тесты и проверки](#тесты-и-проверки)
- [Структура репозитория](#структура-репозитория)
- [Данные и приватность](#данные-и-приватность)
- [Безопасность](#безопасность)
- [Известные ограничения](#известные-ограничения)
- [Решение проблем](#решение-проблем)
- [FAQ](#faq)
- [Участие в разработке](#участие-в-разработке)
- [Лицензия](#лицензия)

---

## Что такое StackPilot

StackPilot — нативное приложение на [Tauri 2](https://tauri.app/), Rust и SvelteKit. Оно закрывает пять повседневных задач:

1. **Понять проект.** Просканировать папку, определить языки, фреймворки, lock-файлы, Docker, тесты, CI и конфигурацию.
2. **Подготовить проект.** Выбрать тип и стек в мастере, увидеть план файлов и сгенерировать каркас.
3. **Подготовить компьютер.** Проверить версии инструментов, health и PATH; построить план установки или обновления.
4. **Запустить стек.** Сохранить сценарий из команд, скриптов, IDE, URL и readiness-checks и запускать его как граф зависимостей.
5. **Наблюдать за результатом.** Смотреть процессы, stdout/stderr, логи, статусы, ошибки, рабочую папку и открывать проект в VS Code.

StackPilot **не является** облачной IDE, CI-сервером, контейнерным оркестратором или заменой Git. Он управляет локальными файлами и локальными процессами, а проект остаётся обычным проектом.

## Кому он нужен

- разработчику с несколькими backend/frontend проектами;
- команде, которой нужен общий сценарий запуска без ручного открытия терминалов;
- новичку, которому надо создать первый проект из готового стека;
- maintainer'у, который проверяет чужой репозиторий;
- автору open-source проекта, который хочет дать контрибьюторам понятный onboarding;
- пользователю Windows, macOS или Linux, которому нужен единый UI.

Если вам нужен удалённый devcontainer, CI/CD, менеджер секретов или полноценный редактор кода, StackPilot следует использовать вместе с такими инструментами.

---

## Возможности

### DevLauncher

- анализирует проект и создаёт **черновик профиля запуска**;
- сохраняет профиль в JSON и запускает его повторно;
- поддерживает старый формат `actions` и новый графовый формат `steps`;
- запускает независимые шаги параллельно, а зависимые — после предшественников;
- выполняет одноразовые команды и долгоживущие сервисы;
- открывает нативное окно терминала, URL, IDE или папку;
- ждёт порт, URL или готовность Docker daemon;
- показывает прогресс каждого шага и итог `succeeded`, `partial_success`, `failed` или `cancelled`;
- останавливает процессы всего запуска вместе с деревом дочерних процессов;
- хранит логи ограниченного размера;
- показывает качество PID: `exact`, `terminal_wrapper`, `approximate`, `detached`.

### Project Creator

- пошаговый wizard с типом проекта, языками, backend/frontend-фреймворками и инструментами;
- готовые пресеты для API, full-stack, desktop/mobile, CLI, Telegram-ботов, ETL, browser extension и других сценариев;
- проверка совместимости и предупреждения о спорных комбинациях;
- рекомендации недостающих языков и инструментов;
- анализ существующего проекта;
- preview рецепта и дерева файлов до записи на диск;
- генерация файлов, директорий, шаблонов и конфигурации;
- опции Git init, VS Code, тестов, CI и Docker;
- передача созданного проекта в DevLauncher.

### Toolchain

- каталог инструментов с minimum/recommended версиями, зависимостями, конфликтами и источниками;
- read-only scan без установки ПО;
- обнаружение по `PATH`, version probe, известным путям и (только на Windows) реестру;
- честные состояния: установлен, отсутствует, нездоров, PATH broken, доступно обновление, manual only, Docker и другие;
- оценка здоровья окружения от 0 до 100;
- план установки, обновления, исправления PATH и health-check;
- Windows: winget и официальные установщики; macOS: Homebrew; Linux: apt, dnf, pacman, zypper и официальные скрипты;
- Docker как альтернатива локальной установке инфраструктурных сервисов;
- отмена, повтор и восстановление прерванных заданий;
- ручное добавление уже установленного инструмента (`adopt`).

### Workspace

- текущий проект и его стек;
- обзор сессии и времени работы;
- вкладки Runtime, Session, Logs, Problems, Info и Files;
- список процессов с PID, командой, cwd, статусом и длительностью;
- просмотр stdout/stderr в терминале xterm;
- файловый браузер с чтением и записью файлов;
- открытие проекта в VS Code;
- assistant-панель (UI-контур; подключение модели пока не выполняет запросы).

### Интерфейс

- русский и английский языки;
- светлая, тёмная и системная тема;
- размер шрифта, accent color, reduced motion и подсказки;
- восстановление последнего маршрута;
- список последних проектов;
- onboarding, который можно открыть повторно через кнопку помощи.

---

## Как устроен рабочий цикл

```text
Открыть или создать проект
          │
          ▼
  Анализ / Project Creator
          │  стек + проектный контекст
          ▼
  Toolchain: scan → plan → install/repair
          │
          ▼
 DevLauncher: профиль (граф шагов)
          │
          ▼
  Run: процессы + readiness + логи
          │
          ▼
 Workspace: обзор, runtime, проблемы, файлы
```

Каждый запуск получает собственный `run_id`. Один профиль можно запускать повторно, а состояние и логи разных запусков не смешиваются. Профиль — это намерение («как поднять проект»), run — конкретная попытка («что произошло сейчас»).

---

## Быстрый старт для пользователя

### 1. Откройте приложение

После первого запуска StackPilot показывает onboarding. Нажмите **Начало работы**, если нужно открыть его снова.

### 2. Выберите сценарий

- **Уже есть проект:** откройте **Анализ** или **DevLauncher → Анализ**.
- **Начинаете с нуля:** откройте **Project Creator**.
- **Проверяете компьютер:** откройте **Toolchain** и запустите Scan.
- **Профиль уже сохранён:** откройте **DevLauncher → Профили**.

### 3. Проверьте окружение

В Toolchain сначала выполните read-only Scan. Сканирование ничего не устанавливает. В карточке инструмента доступны доказательства: version probe, найденный путь, health-check и источник установки. Если инструмент установлен, но не найден, перезапустите приложение или исправьте PATH.

### 4. Запустите проект

На странице профиля проверьте шаги, зависимости и рабочие каталоги. Нажмите **Run**. Для long-running сервисов используйте **Stop** на уровне процесса или всего запуска.

### 5. Разбирайте ошибки

Откройте **Workspace → Problems** для агрегированных ошибок или **DevLauncher → Processes** для конкретного stdout/stderr. Статус `partial_success` означает, что некритичный шаг завершился ошибкой, но остальные сервисы продолжили работу.

---

## Модули приложения

### DevLauncher

DevLauncher — центр запуска. Разделы:

| Раздел | Назначение |
| --- | --- |
| **Обзор** | текущий проект, недавние проекты, быстрые действия и сохранённые профили |
| **Анализ** | построение черновика профиля по файлам проекта |
| **Профили** | просмотр, редактирование, удаление и запуск сохранённых сценариев |
| **Процессы** | ручной запуск команды, live-вывод, refresh и остановка |

#### Что анализатор проверяет

`FsProjectAnalyzer` ищет признаки проекта на глубину до 8 директорий: manifest/lock-файлы, entry points, Docker/compose, конфигурацию IDE, тесты и CI. Каждое предположение получает confidence (`high`, `medium`, `low`) и diagnostic. Анализ создаёт **draft**, а не безусловно правильный план — просмотрите его перед сохранением.

#### Типы шагов профиля

- `run_command` — команда с захваченным выводом;
- `run_script` — shell-скрипт (`sh`, `bash`, `zsh`, `fish`, `cmd`, `powershell`, `pwsh`);
- `open_application` — IDE, Docker Desktop или другое GUI-приложение;
- `open_url` — открыть адрес в браузере;
- `wait_for_port` — ждать TCP-порт;
- `wait_for_url` — ждать HTTP URL;
- `wait_for_docker` — ждать ответа Docker daemon;
- `delay` — фиксированная пауза;
- `open_terminal` — открыть нативный терминал;
- `open_folder` — открыть папку системным приложением.

### Анализ проекта

Анализ читает структуру и конфигурацию, но не запускает install-команды и не изменяет файлы. Результат включает найденные технологии, версии (если их можно вывести), существующие и отсутствующие конфигурации, признаки Git/CI/tests/README/license, подсказки типа проекта и diagnostics.

Если уверенность `low`, воспринимайте её как подсказку. Например, порт `3000` может быть выведен из типичного стека, но не гарантирует, что ваш сервер слушает именно его.

### Project Creator

Мастер создаёт `WizardContext`, валидирует выбранный стек и строит `ExecutionPlan`.

1. Выберите тип: REST API, web app, desktop/mobile, CLI, bot, library, data pipeline, embedded, browser extension или custom.
2. Укажите backend/frontend языки и фреймворки.
3. Добавьте базы, кэш, брокеры, observability, тесты и инструменты.
4. Выберите Docker или локальную инфраструктуру.
5. Включите Git, VS Code, CI и testing при необходимости.
6. Посмотрите recipe preview и file preview.
7. Подтвердите выполнение. Команды и запись файлов отображаются событиями по шагам.

Генерация не стирает существующие файлы молча: шаги имеют `overwrite`, а preview показывает, что будет создано, изменено или пропущено. Для существующей папки сделайте backup и проверьте preview.

### Система совместимости инструментов

Project Creator включает интеллектуальную валидацию совместимости инструментов:

- **Контроль зависимостей:** ловит недостающие зависимости (например, Alembic без SQLAlchemy);
- **Обнаружение пересечения ответственностей:** предупреждает, когда несколько инструментов выполняют одну роль (например, Django ORM + SQLAlchemy);
- **Предупреждения фреймворк↔инструмент:** объясняет, когда инструменты дублируют функции фреймворка;
- **Постепенная обратная связь:** ошибки блокируют генерацию, предупреждения информируют.

Подробная документация — в [`src-tauri/src/modules/project_creator/docs/`](src-tauri/src/modules/project_creator/docs/):
- `TOOL_METADATA.md` — справочник по схеме метаданных;
- `COMPATIBILITY_RULES.md` — логика валидации и правила решений;
- `IMPLEMENTATION_CHECKLIST.md` — статус реализации и известные ограничения.

### Toolchain

Toolchain разделён на три режима.

#### Manage everything

Показывает все определения каталога и состояние на вашей машине. Фильтры работают по статусу, health, категории, происхождению, возможностям и способу исполнения.

#### Build environment

Выберите тип проекта, языки и фреймворки. StackPilot рассчитывает обязательные, рекомендуемые и optional инструменты, учитывает зависимости и конфликты и предлагает план. Инфраструктурные сервисы можно установить локально или оставить Docker Compose проекта.

#### Tool Marketplace

Каталог доступных для текущей ОС инструментов работает даже без scan. Runtime-фильтры (состояние, health, происхождение) появятся только после scan.

#### Установка и права

Перед стартом показываются источник, примерный размер, необходимость администратора и checksum, если он объявлен в каталоге. Инструменты с `manual_install` не устанавливаются автоматически: для них показываются инструкции.

### Workspace

Workspace — представление текущего проекта, а не отдельная копия файлов. Закрытие Workspace не удаляет проект и не останавливает внешние процессы автоматически.

- **Overview:** сводка проекта и активных процессов.
- **Runtime:** статусы `starting`, `running`, `ready`, `exited`, `crashed`, `killed`, `timed_out`, `cancelled` и др.
- **Session:** время сессии, количество процессов и ошибок.
- **Logs:** захваченные логи процессов.
- **Problems:** ошибки из реального состояния процессов.
- **Info:** профиль, путь, стек и метаданные.
- **Files:** чтение/запись файлов в пределах выбранного проекта.

### Настройки

Вкладки **Личные**, **Система**, **Отображение**, **Поведение**, **ИИ** и **О приложении** позволяют настроить пути к VS Code/браузеру/терминалу, тему, язык, размер шрифта, accent color, автосохранение, восстановление маршрута, локальные личные метаданные и пользовательские GUI-приложения.

Раздел ИИ пока является фундаментом интерфейса: поля сохраняются, но запросы к OpenAI/Anthropic/Ollama/custom endpoint не отправляются, ключи не проверяются и assistant не активируется от одного переключателя.

---

## Поддерживаемые проекты и инструменты

Каталог находится в [`src-tauri/src/modules/project_creator/knowledge/wizard_tree.json`](src-tauri/src/modules/project_creator/knowledge/wizard_tree.json) и [`src-tauri/src/modules/toolchain/tools.json`](src-tauri/src/modules/toolchain/tools.json). В нём есть, среди прочего:

- языки: Python, Rust, Go, TypeScript, JavaScript, Java, C#, C++, Dart, Kotlin, PHP, Swift, Zig, Elixir, Gleam и HTML;
- web/backend: FastAPI, Django, Flask, Axum, Gin, NestJS, Express, Spring Boot, Laravel, Symfony, Phoenix, Ktor;
- frontend/UI: React, Vue, Svelte, Next.js, Nuxt, SvelteKit, SolidStart, Flutter, React Native, Expo, Tauri, Electron, Qt, Jetpack Compose, SwiftUI;
- utility/CLI: npm, cargo, Gradle, Maven, CMake, Composer, curl, tar, Git, VS Code;
- инфраструктура: Docker, PostgreSQL, Redis/Memurai, MongoDB, MySQL, SQLite, Kafka, ClickHouse, Airflow, dbt, Grafana, Firebase, OpenTelemetry, Terraform;
- testing/code quality: Pytest и Ruff.

Реальная доступность зависит от ОС, архитектуры, источника установки и состояния каталога. Если технологии нет в wizard tree, используйте `custom` и добавьте шаги профиля вручную.

---

## Профили запуска V2

Профиль хранится как JSON в `<app_data>/profiles/<name>.json`.

Минимальный пример:

```json
{
  "schema_version": "2",
  "id": "api-stack",
  "name": "FastAPI + PostgreSQL",
  "description": "Локальный backend и база данных",
  "project_root": "C:/work/my-api",
  "preferred_ide": "Vscode",
  "steps": [
    {
      "id": "db",
      "label": "PostgreSQL",
      "enabled": true,
      "kind": { "type": "run_command", "command": "docker compose up db" },
      "visibility": "captured",
      "execution_mode": "long_running",
      "completion": { "type": "process_started" },
      "failure_policy": "stop_run"
    },
    {
      "id": "api",
      "label": "Uvicorn",
      "enabled": true,
      "depends_on": ["db"],
      "kind": { "type": "run_command", "command": "uvicorn app.main:app --reload --port 8000" },
      "visibility": "captured",
      "execution_mode": "long_running",
      "completion": { "type": "port_open", "host": "127.0.0.1", "port": 8000 },
      "timeout": 60,
      "failure_policy": "stop_run",
      "retry_policy": { "max_retries": 1, "delay_ms": 1000 }
    },
    {
      "id": "docs",
      "label": "Open API docs",
      "enabled": true,
      "depends_on": ["api"],
      "kind": { "type": "open_url", "url": "http://127.0.0.1:8000/docs" },
      "completion": { "type": "external_launch_accepted" },
      "failure_policy": "warn_and_continue"
    }
  ]
}
```

### Поля профиля

| Поле | Смысл |
| --- | --- |
| `schema_version` | сейчас `2`; старые файлы с `actions` мигрируются прозрачно |
| `id` | стабильный идентификатор профиля |
| `name`, `description` | имя и пояснение в UI |
| `project_root` | базовая рабочая директория для относительных путей |
| `environment_binding_id` | привязка к переопределениям окружения |
| `preferred_ide` | VS Code, PyCharm, GoLand, IDEA, WebStorm, Xcode, Visual Studio или custom |
| `steps` | узлы графа запуска |

У шага есть `id`, `label`, `enabled`, `kind`, `depends_on`, `working_directory`, `environment`, `visibility`, `execution_mode`, `completion`, `timeout`, `failure_policy`, `retry_policy` и `metadata`.

### Visibility и execution mode

- `captured` — stdout/stderr попадают в логи StackPilot;
- `visible_terminal` — команда выполняется в нативном окне терминала;
- `detached` — GUI/внешний процесс запускается без захвата вывода;
- `one_shot` — шаг завершается после успешного выхода процесса;
- `long_running` — сервис остаётся активным до остановки или падения.

### Политика завершения и ошибок

Completion policy может быть `exit_success`, `process_started`, `port_open`, `url_ready`, `delay_elapsed`, `external_launch_accepted` или `manual`. Ошибка шага обрабатывается явно:

- `stop_run` — остановить run и его управляемые процессы;
- `skip_dependents` — пропустить зависимые шаги;
- `warn_and_continue` — продолжить и получить `partial_success`.

Зависимости должны ссылаться на существующие ID и не образовывать цикл. Валидация проверяет пустые команды, некорректные порты/URL, несовместимый shell и циклы до запуска.

### Обратная совместимость

Старый формат с плоским массивом `actions` поддерживается legacy API. При обращении через V2 действия превращаются в последовательные шаги, а при сохранении новый формат остаётся V2. Не удаляйте `actions` вручную, если профиль используют старые интеграции.

---

## Окружения проекта и bindings

`EnvironmentBinding` позволяет не менять глобальную систему ради одного проекта. Binding может содержать путь конкретного `python`, `node`, `cargo` и другого executable, дополнительные PATH entries, переменные для установки/удаления, рекомендуемый IDE и привязку к абсолютному пути проекта.

Binding хранится в `<app_data>/project_environments/<binding_id>.json` и применяется как overlay при запуске. Относительные и отсутствующие executable дают warning. Это удобно для нескольких версий Node/Python, SDK в нестандартных каталогах и portable toolchain.

---

## Платформенная модель

Поддерживаются **Windows, Linux и macOS** (x64/arm64 в пределах возможностей зависимостей и конкретного установщика).

| Возможность | Windows | Linux | macOS |
| --- | --- | --- | --- |
| Команды по умолчанию | `cmd /D /C` | `sh -lc` | `sh -lc` |
| Скрипты | PowerShell/cmd | sh/bash/zsh/fish | sh/bash/zsh/fish |
| Группа процессов | `CREATE_NEW_PROCESS_GROUP` | `setsid()` | `setsid()` |
| Остановка дерева | `taskkill /F /T` | SIGTERM → SIGKILL | SIGTERM → SIGKILL |
| PATH persistence | registry/PowerShell | rc-файл с маркерами | rc-файл с маркерами |
| Package manager | winget | apt/dnf/pacman/zypper | brew |
| Нативный терминал | Windows Terminal или cmd | первый найденный emulator | Terminal.app или iTerm2 |

Если package manager, Docker Desktop, WSL2, Xcode или права администратора недоступны, UI покажет причину и предложит ручной путь либо Docker-альтернативу, если она объявлена каталогом.

---

## Установка готового приложения

Репозиторий содержит исходники и конфигурацию Tauri, но до отдельного релизного процесса может не содержать опубликованных installers в Releases. Когда бинарные сборки появятся, скачивайте их только со страницы Releases этого репозитория и проверяйте checksum.

Для запуска из исходников используйте раздел ниже.

---

## Сборка из исходников

### Требования

- Git;
- Node.js 18+ (рекомендуется 22 LTS) и npm;
- Rust stable и Cargo;
- системные зависимости Tauri 2 для вашей ОС;
- Windows: WebView2 и Visual Studio Build Tools/Windows SDK;
- Linux: GTK/WebKitGTK и пакеты, указанные в [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/);
- macOS: Xcode Command Line Tools;
- опционально Bun (есть `bun.lock`, но npm поддерживается официальными scripts).

### Клонирование и зависимости

```bash
git clone https://github.com/cheburek4535/StackPilot.git
cd StackPilot
npm ci
```

Или Bun:

```bash
bun install --frozen-lockfile
```

### Запуск desktop-приложения

```bash
npm run tauri dev
```

Tauri сам запустит `npm run dev`, frontend будет доступен через `http://127.0.0.1:1420`, а окно подключится к нему.

### Production bundle

```bash
npm run tauri build
```

Результаты появятся в `src-tauri/target/release/bundle/` (`.msi`/`.exe`, `.dmg`, `.deb`, `.AppImage` и т. п. в зависимости от ОС). Перед публикацией проверьте подпись, installer и чистую установку на целевой ОС.

### Только frontend

```bash
npm run dev
npm run build
npm run preview
```

В браузерном режиме включается `tauriMock`: команды и события эмулируются в памяти/localStorage. Это удобно для UI, но не проверяет реальные процессы, PATH, Docker или права ОС.

### Переменные окружения

```bash
# macOS / Linux / Git Bash
cp .env.example .env
```

```powershell
# Windows PowerShell
Copy-Item .env.example .env
```

В примере:

```dotenv
PORT=3000
NODE_ENV=development
```

Vite dev-сервер StackPilot использует порт `1420`; `PORT` из примера относится к данным проекта/демо и не меняет адрес Tauri frontend. Секреты не коммитьте в `.env`.

---

## Разработка интерфейса

```bash
npm run check
npm run check:watch
npm run lint
npm test
npm run test:watch
```

Frontend находится в `src/`, маршруты — в `src/routes/`, компоненты — в `src/lib/components/`, доменная логика — в `src/lib/modules/`. Вызовы Rust проходят через API-обёртки `src/lib/modules/*/api.ts`.

При изменении Rust-команд обновляйте типы frontend и события. Новые строки добавляйте в `src/lib/core/locales/en.ts` и `ru.ts`.

---

## Тесты и проверки

### Frontend

```bash
npm test
npm run check
```

Тесты охватывают типы DevLauncher, правила стека, состояние и события Toolchain, фильтры каталога, score и startup-координатор.

### Rust

```bash
cd src-tauri
cargo fmt --check
cargo check
cargo test
```

Тесты, зависящие от сети, package manager, Docker или конкретной IDE, должны быть изолированы или явно помечены. Изменения shell/process/PATH должны иметь регрессионные тесты для Windows и Unix-ветки.

---

## Структура репозитория

```text
.
├── src/                         # SvelteKit frontend
│   ├── routes/                  # страницы Home, DevLauncher, Toolchain, Workspace…
│   └── lib/                     # components, core, modules
├── src-tauri/
│   └── src/
│       ├── core/                # настройки и общие сервисы
│       ├── platform/            # OS, shell, command, terminal, IDE, Docker
│       └── modules/
│           ├── devlauncher/     # профили, analyzer, orchestrator, runs
│           ├── project_creator/ # wizard, recipes, generators, analysis
│           ├── project_environment/ # bindings и overlays
│           ├── toolchain/       # catalog, scan, planner, installer, jobs
│           └── workspace/       # processes, logs, session, files
├── static/                      # логотипы, иконки и изображения каталога
├── docs/                        # контракты и progress-документы
├── package.json
└── README.md
```

Rust-модули регистрируют Tauri commands в [`src-tauri/src/lib.rs`](src-tauri/src/lib.rs). Контракт DevLauncher V2 описан в [`docs/devlauncher-contract.md`](docs/devlauncher-contract.md), кросс-платформенная матрица — в [`docs/CROSS_PLATFORM_PROGRESS.md`](docs/CROSS_PLATFORM_PROGRESS.md).

### Для интеграторов и авторов плагинов

Frontend общается с Rust через Tauri IPC. Основные группы команд:

| Группа | Примеры команд |
| --- | --- |
| DevLauncher | `analyze_project_v2`, `save_profile_v2`, `run_profile_v2`, `cancel_run`, `get_run_logs` |
| Workspace | `spawn_process`, `list_processes`, `kill_process`, `read_file`, `write_file`, `open_in_vscode` |
| Project Creator | `get_wizard_tree`, `start_wizard`, `preview_project_files`, `start_project_execution` |
| Toolchain | `tcx_start_scan`, `tcx_build_plan`, `tcx_start_job`, `tcx_cancel_job`, `tcx_run_health_checks` |
| Environment | `pe_create_binding`, `pe_resolve_overlay`, `pe_validate_binding` |
| Settings | `get_settings`, `update_settings`, `reset_settings`, `get_app_data_dir` |

Ключевые события Tauri: `process-output`, `process-status`, `devlauncher:step-status-changed`, `devlauncher:run-status-changed`, `toolchainx:scan_progress`, `toolchainx:scan_done` и `toolchainx:job_event`. Схемы payload находятся в соответствующих `models.rs` и frontend `types.ts`; при интеграции используйте их как источник истины.

В Cargo feature `plugins` включён по умолчанию. Он регистрирует заготовленные команды mini-IDE (completion, diagnostics, hover, go-to-definition и format); это отдельный экспериментальный слой и не превращает Workspace в полноценный редактор.

---

## Данные и приватность

Приложение локальное:

- настройки: `<app_data>/settings.json`;
- профили: `<app_data>/profiles/`;
- bindings: `<app_data>/project_environments/`;
- Toolchain state и журналы: `<app_data>/toolchain/`;
- недавние проекты, onboarding, тема и последний маршрут: `localStorage` frontend-контекста;
- временные файлы записываются атомарно через `.tmp` и rename.

Точный путь виден в **Настройки → Система → Папка данных приложения** или возвращается командой `get_app_data_dir`. На Windows часть секретов использует DPAPI; если DPAPI недоступен, backend предупреждает и может сохранить значение без шифрования. Не храните production-токены в профилях и не коммитьте каталог данных.

Toolchain может обращаться к официальным URL установщиков и package manager вашей ОС. DevLauncher выполняет команды, которые вы сами сохранили или подтвердили через wizard. Фоновая telemetry не используется.

---

## Безопасность

Профили — исполняемые сценарии:

1. Просматривайте шаги и команды перед первым запуском.
2. Не запускайте непроверенные JSON-профили из чужих репозиториев.
3. Проверяйте `working_directory`, `environment`, shell и URL.
4. Не добавляйте API-ключи в `steps`, `metadata`, `.env.example` или issue.
5. Используйте минимально необходимые права ОС.
6. Помните, что `run_script` выполняет произвольный shell-код с правами пользователя.
7. Уязвимости отправляйте приватно через Security policy репозитория, если она опубликована.

Генератор проверяет пути и конфликты, но не является sandbox'ом. Docker-альтернатива — рекомендация, а не изоляция любой команды.

---


## Решение проблем

### `command not found`, `not recognized` или PATH broken

1. Запустите Toolchain Scan.
2. Проверьте найденные пути и evidence в карточке.
3. Перезапустите StackPilot и уже открытые терминалы после установки.
4. Добавьте абсолютный путь через Environment Binding.
5. На Unix проверьте блоки `# StackPilot:begin` / `# StackPilot:end` в rc-файле.

### Docker не готов

Убедитесь, что `docker version` работает и daemon запущен. На Windows может потребоваться WSL2. Добавьте `wait_for_docker` перед `docker compose`.

### Сервис сразу завершается

Проверьте cwd, переменные binding и completion policy. Запустите команду вручную и откройте stderr в **Processes → Logs**.

### Порт занят

Измените порт, остановите старый процесс в **DevLauncher → Processes** и обновите статус. `wait_for_port` только проверяет готовность и не освобождает порт автоматически.

### Не открывается IDE или браузер

Укажите полный путь в **Настройки → Система** или добавьте приложение в **Предпочтительные приложения**. Пустой путь означает системное значение по умолчанию.

### `cargo test` на Windows завершается `STATUS_ENTRYPOINT_NOT_FOUND`

Тестовые бинарники требуют Windows SDK resource compiler (`rc.exe`) для common-controls manifest. Установите Windows SDK/Visual Studio Build Tools и повторите.

### После обновления пропали профили

Проверьте путь **Настройки → Система → Папка данных приложения**. Не удаляйте `profiles/`: V2 loader читает старые `actions` и сохраняет неизвестные поля для forward compatibility.

---

## FAQ

**StackPilot отправляет мой код в интернет?**

Нет, обычные функции работают локально. Установщики и package manager сами скачивают выбранное ПО. AI UI пока не отправляет запросы.

**Можно пользоваться только DevLauncher?**

Да. Откройте существующую папку, проанализируйте её или создайте профиль вручную.

**Можно ли использовать свои команды?**

Да, через `run_command`, `run_script` и ручной менеджер процессов. Команда выполняется с правами текущего пользователя.

**Обязателен ли Docker?**

Нет. Docker — опция проекта; многие инструменты устанавливаются локально.

**Можно ли запускать Windows-профиль на Linux/macOS?**

Профиль с платформенным shell/путями нужно адаптировать. Валидатор предупредит о несовместимом `cmd`/PowerShell и путях.

**Где хранятся профили?**

В `<app_data>/profiles/*.json`; точный путь виден в Settings.

**Удалит ли StackPilot мои файлы?**

Только если подтверждённый шаг имеет `overwrite=true`. Проверяйте file preview и делайте backup.

**Нужен ли Bun?**

Нет. Официальный путь — Node.js + npm; Bun поддержан как альтернатива.

**Что означает `partial_success`?**

Некритичный шаг не удался, но политика профиля разрешила продолжить. Откройте diagnostics и логи проблемного шага.

---

## Участие в разработке

Pull requests и issues приветствуются. Перед большой задачей откройте issue с контекстом и ожидаемым поведением.

```bash
git checkout -b codex/my-change
npm ci
npm run check
npm test
cd src-tauri
cargo fmt --check
cargo check
cargo test
```

Для изменений каталога инструментов указывайте detection, sources, platform availability, checksum и manual notes честно, добавляйте тесты и проверяйте bootstrap-зависимость package manager.

Для изменений DevLauncher сохраняйте совместимость `LaunchProfile`, `LaunchAction` и legacy commands, валидируйте duplicate IDs/missing dependency/cycle и обновляйте [`docs/devlauncher-contract.md`](docs/devlauncher-contract.md) при изменении контракта.

В PR описывайте пользовательский эффект, затронутые ОС, тесты и ограничения. Не прикладывайте логи с токенами, домашними путями и персональными данными.

---

## Лицензия

StackPilot распространяется под [GNU Affero General Public License v3.0 only](https://www.gnu.org/licenses/agpl-3.0.html) (`AGPL-3.0-only`, как указано в `package.json`). Производные работы и изменения должны предоставляться на условиях AGPL-3.0, включая требования лицензии для сетевого взаимодействия с модифицированной версией. Перед коммерческим распространением прочитайте полный текст лицензии.

Автор и текущий maintainer: **cheburek4535**. Благодарности, предложения и контрибуции — через GitHub issues и pull requests.

---

## Полезные ссылки

- [Tauri 2 documentation](https://v2.tauri.app/)
- [SvelteKit documentation](https://kit.svelte.dev/docs)
- [Rust documentation](https://doc.rust-lang.org/)
- [Vite documentation](https://vite.dev/guide/)
- [Документация DevLauncher V2](docs/devlauncher-contract.md)
- [Кросс-платформенный прогресс](docs/CROSS_PLATFORM_PROGRESS.md)
- [Прогресс Toolchain](docs/progress_toolchain.md)
