# Профили запуска V2 — контракт

Формат профиля, типы шагов и политики запуска. Документ для разработчиков и интеграторов.

Профиль хранится как JSON в `<app_data>/profiles/<name>.json`. Путь к папке данных виден в **Настройки → Система → Папка данных приложения**.

## Минимальный пример

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

## Поля профиля

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

## Типы шагов

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

## Visibility и execution mode

- `captured` — stdout/stderr попадают в логи StackPilot;
- `visible_terminal` — команда выполняется в нативном окне терминала;
- `detached` — GUI/внешний процесс запускается без захвата вывода;
- `one_shot` — шаг завершается после успешного выхода процесса;
- `long_running` — сервис остаётся активным до остановки или падения.

## Политика завершения и ошибок

Completion policy может быть `exit_success`, `process_started`, `port_open`, `url_ready`, `delay_elapsed`, `external_launch_accepted` или `manual`. Ошибка шага обрабатывается явно:

- `stop_run` — остановить run и его управляемые процессы;
- `skip_dependents` — пропустить зависимые шаги;
- `warn_and_continue` — продолжить и получить `partial_success`.

Зависимости должны ссылаться на существующие ID и не образовывать цикл. Валидация проверяет пустые команды, некорректные порты/URL, несовместимый shell и циклы до запуска.

## Обратная совместимость

Старый формат с плоским массивом `actions` поддерживается legacy API. При обращении через V2 действия превращаются в последовательные шаги, а при сохранении новый формат остаётся V2. Не удаляйте `actions` вручную, если профиль используют старые интеграции.

## Окружения проекта и bindings

`EnvironmentBinding` позволяет не менять глобальную систему ради одного проекта. Binding может содержать путь конкретного `python`, `node`, `cargo` и другого executable, дополнительные PATH entries, переменные для установки/удаления, рекомендуемый IDE и привязку к абсолютному пути проекта.

Binding хранится в `<app_data>/project_environments/<binding_id>.json` и применяется как overlay при запуске. Относительные и отсутствующие executable дают warning. Это удобно для нескольких версий Node/Python, SDK в нестандартных каталогах и portable toolchain.

Реализация: [`src-tauri/src/modules/project_environment/`](../src-tauri/src/modules/project_environment/).
