// Content generation functions — pure data/string generators,
// separated from recipe composition logic.

// ---------------------------------------------------------------------------
// Environment example templates
// ---------------------------------------------------------------------------

pub fn get_env_example(tool_id: &str) -> String {
    match tool_id {
        // Значения docker-режима обязаны совпадать с docker-compose.yaml
        // (collect_docker_services / generate_docker_compose): app-сервис
        // подключается к контейнерам именно с этими учётками.
        "postgresql" => r#"POSTGRES_USER=postgres
POSTGRES_PASSWORD=12345
POSTGRES_DB=postgres
POSTGRES_PORT=5432
DATABASE_URL=postgresql://postgres:12345@localhost:5432/postgres
"#
        .to_string(),
        "redis" => r#"REDIS_URL=redis://localhost:6379/0
"#
        .to_string(),
        "mongodb" => r#"MONGODB_URI=mongodb://localhost:27017
MONGODB_DB=dbname
"#
        .to_string(),
        "mysql" => r#"MYSQL_ROOT_PASSWORD=root_pwd
MYSQL_DATABASE=mydb
MYSQL_USER=root
MYSQL_PASSWORD=root_pwd
MYSQL_PORT=3306
"#
        .to_string(),
        "kafka" => r#"KAFKA_BOOTSTRAP_SERVERS=localhost:9092
"#
        .to_string(),
        "clickhouse" => r#"CLICKHOUSE_HTTP_URL=http://localhost:8123
CLICKHOUSE_NATIVE_URL=localhost:9000
"#
        .to_string(),
        "airflow" => r#"AIRFLOW__CORE__EXECUTOR=LocalExecutor
AIRFLOW__DATABASE__SQL_ALCHEMY_CONN=postgresql+psycopg2://postgres:12345@postgres:5432/postgres
AIRFLOW__CORE__LOAD_EXAMPLES=False
AIRFLOW_WEBSERVER_PORT=8080
"#
        .to_string(),
        "grafana" => r#"GRAFANA_URL=http://localhost:3001
"#
        .to_string(),
        _ => format!("# Environment variables for {}\n", tool_id),
    }
}

/// Переменные окружения для ЛОКАЛЬНО установленного инфра-инструмента
/// (postgresql, redis, mongodb, ...). Используются, когда пользователь
/// выбрал локальную установку вместо docker-compose: адреса указывают на
/// localhost, а не на имя контейнера.
pub fn get_local_env_example(tool_id: &str) -> String {
    match tool_id {
        "postgresql" => r#"# PostgreSQL (local install)
# Пароль суперпользователя сгенерирован при установке — он показан один раз
# в StackPilot («Generated passwords»). Базу создайте сами:
#   createdb -U postgres <dbname>
POSTGRES_USER=postgres
POSTGRES_PASSWORD=<PASSWORD_FROM_STACKPILOT>
POSTGRES_DB=dbname
POSTGRES_PORT=5432
DATABASE_URL=postgresql://postgres:<PASSWORD_FROM_STACKPILOT>@localhost:5432/dbname
"#
        .to_string(),
        "redis" => r#"# Redis (local install)
# Запустите один раз: redis-server (см. LOCAL_INFRA.md)
REDIS_URL=redis://localhost:6379/0
"#
        .to_string(),
        "mongodb" => r#"# MongoDB (local install)
# Запустите один раз: mongod --dbpath <data-folder> (см. LOCAL_INFRA.md)
MONGODB_URI=mongodb://localhost:27017
MONGODB_DB=dbname
"#
        .to_string(),
        "mysql" => r#"# MySQL (local install)
# Пароль root задаётся при установке (MSI-мастер). Базу создайте сами:
#   mysql -u root -p -e "CREATE DATABASE dbname CHARACTER SET utf8mb4;"
MYSQL_HOST=localhost
MYSQL_PORT=3306
MYSQL_DATABASE=dbname
MYSQL_USER=root
MYSQL_PASSWORD=<ROOT_PASSWORD_FROM_INSTALL>
"#
        .to_string(),
        "kafka" => r#"# Apache Kafka (local install)
# Одноузловой режим (KRaft) — запустите один раз (см. LOCAL_INFRA.md)
KAFKA_BOOTSTRAP_SERVERS=localhost:9092
"#
        .to_string(),
        // Локальный Grafana слушает свой штатный порт 3000 — конфликта
        // с app-сервисом проекта в локальном режиме нет (в docker его
        // выносим на 3001, чтобы не пересекаться с app_port).
        "grafana" => r#"# Grafana (local install)
# Запустите один раз: grafana-server (см. LOCAL_INFRA.md)
GRAFANA_URL=http://localhost:3000
"#
        .to_string(),
        _ => get_env_example(tool_id),
    }
}

/// Инструкция по запуску локально установленных инфра-инструментов
/// (файл LOCAL_INFRA.md в проекте). Пишется, когда пользователь выбрал
/// локальную установку вместо docker-compose.
pub fn generate_local_infra_guide(tools: &[String]) -> String {
    let mut out = String::from(
        "# Local infrastructure\n\n\
You chose to run these services locally instead of Docker. They were installed \
by StackPilot and detected on this machine. Start them once per session — \
they must be running before you launch the project.\n",
    );

    for tool in tools {
        let section: &str = match tool.as_str() {
            "postgresql" => {
                r#"## PostgreSQL

- The superuser password was generated during installation and shown ONCE in
  StackPilot (`Generated passwords` window after installation).
- Create the database before first run:
  ```powershell
  createdb -U postgres dbname
  ```
- Connection: see `POSTGRES_*` / `DATABASE_URL` in `.env.example`.
"#
            }
            "redis" => {
                r#"## Redis

- Start the server once (install directory is on PATH):
  ```powershell
  redis-server
  ```
- Connection: `REDIS_URL=redis://localhost:6379/0`.
"#
            }
            "mongodb" => {
                r#"## MongoDB

- Create a data folder and start the server once (install directory is on PATH):
  ```powershell
  mkdir $env:USERPROFILE\mongo-data
  mongod --dbpath $env:USERPROFILE\mongo-data
  ```
- Connection: `MONGODB_URI=mongodb://localhost:27017`.
"#
            }
            "mysql" => {
                r#"## MySQL

- The root password was set during installation (MSI setup).
- The server runs as a Windows service (`MySQL80`); start it with:
  ```powershell
  net start MySQL80
  ```
- Create the database before first run:
  ```powershell
  mysql -u root -p -e "CREATE DATABASE dbname CHARACTER SET utf8mb4;"
  ```
- Connection: `MYSQL_*` in `.env.example`.
"#
            }
            "kafka" => {
                r#"## Apache Kafka

- Single-node KRaft mode. From the Kafka install directory (`bin/windows`):
  ```powershell
  kafka-storage random-uuid > uuid.txt
  kafka-storage format -t (Get-Content uuid.txt) -c ..\config\kraft\server.properties
  kafka-server-start ..\config\kraft\server.properties
  ```
- Connection: `KAFKA_BOOTSTRAP_SERVERS=localhost:9092`.
"#
            }
            "grafana" => {
                r#"## Grafana

- Start the server once (install directory is on PATH):
  ```powershell
  grafana-server
  ```
- UI: http://localhost:3000 (admin/admin on first run).
- Connection: `GRAFANA_URL=http://localhost:3000`.
"#
            }
            _ => "",
        };
        if !section.is_empty() {
            out.push('\n');
            out.push_str(section);
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Docker service definition
// ---------------------------------------------------------------------------

pub struct DockerService {
    pub name: String,
    pub image: String,
    pub ports: Vec<String>,
    pub environment: Vec<(String, String)>,
    pub volumes: Vec<String>,
    pub depends_on: Vec<String>,
}

// ---------------------------------------------------------------------------
// Docker service collection from tool IDs
// ---------------------------------------------------------------------------

pub fn collect_docker_services(tools: &[String]) -> Vec<DockerService> {
    let mut services = Vec::new();

    for tool in tools.iter() {
        match tool.as_str() {
            "postgresql" => services.push(DockerService {
                name: "postgres".into(),
                image: "postgres:16-alpine".into(),
                ports: vec!["5432:5432".into()],
                environment: vec![
                    ("POSTGRES_USER".into(), "postgres".into()),
                    ("POSTGRES_PASSWORD".into(), "12345".into()),
                    ("POSTGRES_DB".into(), "postgres".into()),
                ],
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),

            "redis" => services.push(DockerService {
                name: "redis".into(),
                image: "redis:7-alpine".into(),
                ports: vec!["6379:6379".into()],
                environment: Vec::new(),
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),

            "mongodb" => services.push(DockerService {
                name: "mongo".into(),
                image: "mongo:8".into(),
                ports: vec!["27017:27017".into()],
                environment: Vec::new(),
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),

            "mysql" => services.push(DockerService {
                name: "mysql".into(),
                image: "mysql:8".into(),
                ports: vec!["3306:3306".into()],
                environment: vec![
                    ("MYSQL_ROOT_PASSWORD".into(), "root_pwd".into()),
                    ("MYSQL_DATABASE".into(), "mydb".into()),
                ],
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),

            "kafka" => {
                services.push(DockerService {
                    name: "zookeeper".into(),
                    image: "confluentinc/cp-zookeeper:latest".into(),
                    ports: vec!["2181:2181".into()],
                    environment: vec![
                        ("ZOOKEEPER_CLIENT_PORT".into(), "2181".into()),
                        ("ZOOKEEPER_TICK_TIME".into(), "2000".into()),
                    ],
                    volumes: Vec::new(),
                    depends_on: Vec::new(),
                });

                services.push(DockerService {
                    name: "kafka".into(),
                    image: "confluentinc/cp-kafka:latest".into(),
                    ports: vec!["9092:9092".into()],
                    environment: vec![
                        ("KAFKA_BROKER_ID".into(), "1".into()),
                        ("KAFKA_ZOOKEEPER_CONNECT".into(), "zookeeper:2181".into()),
                        (
                            "KAFKA_ADVERTISED_LISTENERS".into(),
                            "PLAINTEXT://localhost:9092".into(),
                        ),
                        ("KAFKA_OFFSETS_TOPIC_REPLICATION_FACTOR".into(), "1".into()),
                    ],
                    volumes: Vec::new(),
                    depends_on: vec!["Zookeeper".into()],
                });
            }

            "clickhouse" => services.push(DockerService {
                name: "clickHouse".into(),
                image: "clickhouse/clickhouse-server:latest".into(),
                ports: vec!["8123:8123".into(), "9000:9000".into()],
                environment: Vec::new(),
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),

            "mailpit" => services.push(DockerService {
                name: "mailpit".into(),
                image: "axllent/mailpit:latest".into(),
                ports: vec!["1025:1025".into(), "8025:8025".into()],
                environment: Vec::new(),
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),

            "opentelemetry" => services.push(DockerService {
                name: "otel-collector".into(),
                image: "otel/opentelemetry-collector-contrib:0.114.0".into(),
                ports: vec!["4317:4317".into(), "4318:4318".into()],
                environment: Vec::new(),
                volumes: vec!["./config/otel-collector.yaml:/etc/otelcol/config.yaml".into()],
                depends_on: Vec::new(),
            }),

            "airflow" => services.push(DockerService {
                name: "airflow".into(),
                image: "apache/airflow:2.10.4".into(),
                ports: vec!["8080:8080".into()],
                environment: vec![
                    ("AIRFLOW__CORE__EXECUTOR".into(), "LocalExecutor".into()),
                    (
                        "AIRFLOW__DATABASE__SQL_ALCHEMY_CONN".into(),
                        "postgresql+psycopg2://postgres:12345@postgres:5432/postgres".into(),
                    ),
                    ("AIRFLOW__CORE__LOAD_EXAMPLES".into(), "False".into()),
                    (
                        "AIRFLOW__WEBSERVER__SECRET_KEY".into(),
                        "airflow-secret-key".into(),
                    ),
                ],
                volumes: vec![
                    "./dags:/opt/airflow/dags".into(),
                    "./logs:/opt/airflow/logs".into(),
                ],
                depends_on: vec!["postgres".into()],
            }),

            // grafana: контейнерный порт 3000 — конфликтовал бы с app-сервисом
            // проекта (app_port 3000), поэтому наружу отдаём 3001.
            "grafana" => services.push(DockerService {
                name: "grafana".into(),
                image: "grafana/grafana:11.6.1".into(),
                ports: vec!["3001:3000".into()],
                environment: Vec::new(),
                volumes: Vec::new(),
                depends_on: Vec::new(),
            }),

            _ => {}
        }
    }

    services
}

// ---------------------------------------------------------------------------
// Go toolchain version for build/CI files
// ---------------------------------------------------------------------------
//
// Го-проект создаётся через `go mod init`, а оно записывает в go.mod версию
// ЛОКАЛЬНО установленного Go (например «go 1.25.0»). Если Dockerfile/CI
// жёстко зашпинывали бы старую версию (было golang:1.24-alpine), контейнер
// с более новым go.mod не собрался бы — версии расходились. Поэтому версия
// для golang-образа и go-version в CI берётся из фактически установленного
// Go, а не из константы.

/// Рекомендуемая версия Go из toolchain/tools.json — запасной вариант,
/// когда версию нельзя определить (Go не в PATH и т.п.). Это НИЖНЯЯ граница:
/// образ всегда >= того, что напишет в go.mod установленный Go.
pub const DEFAULT_GO_VERSION: &str = "1.24";

/// Извлекает «X.Y» из вывода `go version` («go version go1.25.3 windows/amd64»).
/// Go всегда версионируется как go1.MINOR.PATCH → возвращаем «1.MINOR».
/// Чистая функция — тестируется без запуска процессов.
pub fn go_minor_version(raw: &str) -> Option<String> {
    // Ищем маркер «go1.» — он фиксирует мажорную версию 1 и съедает её.
    let marker = "go1.";
    let idx = raw.find(marker)?;
    let rest = &raw[idx + marker.len()..];
    let mut minor = String::new();
    for c in rest.chars() {
        if c.is_ascii_digit() {
            minor.push(c);
        } else {
            break;
        }
    }
    if minor.is_empty() {
        return None;
    }
    Some(format!("1.{minor}"))
}

/// Определяет мажор+минор установленного Go через `go version`.
/// При любой ошибке возвращает DEFAULT_GO_VERSION (безопасно: это нижняя
/// граница, образ всегда >= версии, которую напишет go mod init).
pub fn detect_go_version() -> String {
    if let Ok(out) = std::process::Command::new("go")
        .arg("version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    {
        if let Some(v) = go_minor_version(&out) {
            return v;
        }
    }
    DEFAULT_GO_VERSION.to_string()
}

// ---------------------------------------------------------------------------
//
// Аналогично Go: `cargo init` пишет edition по умолчанию из ЛОКАЛЬНО
// установленного rustc (новые версии — edition 2024, требующая rust >= 1.85).
// Если образ был бы жёстко зашпинен на старую версию (было rust:1.83),
// каркас с edition 2024 не собрался бы. Поэтому версия rust-образа берётся
// из фактически установленного rustc, а не из константы.
//
// Минимальная версия rustc для edition 2024 — 1.85. Фоллбэк должен быть
// >= 1.85, иначе собранный каркас с edition 2024 не скомпилируется в образе.

/// Нижняя граница rust-образа, поддерживающая edition 2024 (rustc >= 1.85).
pub const RUST_DOCKERFILE_FLOOR: &str = "1.85";

/// Извлекает «1.X» из вывода `rustc --version` («rustc 1.97.1 (8bab… )»).
/// Чистая функция — тестируется без запуска процессов.
pub fn rust_minor_version(raw: &str) -> Option<String> {
    // «rustc 1.» фиксирует мажорную версию 1 и съедает её.
    let marker = "rustc 1.";
    let idx = raw.find(marker)?;
    let rest = &raw[idx + marker.len()..];
    let mut minor = String::new();
    for c in rest.chars() {
        if c.is_ascii_digit() {
            minor.push(c);
        } else {
            break;
        }
    }
    if minor.is_empty() {
        return None;
    }
    Some(format!("1.{minor}"))
}

/// Определяет мажор+минор установленного rustc. При ошибке — RUST_DOCKERFILE_FLOOR.
pub fn detect_rust_version() -> String {
    if let Ok(out) = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    {
        if let Some(v) = rust_minor_version(&out) {
            return v;
        }
    }
    RUST_DOCKERFILE_FLOOR.to_string()
}

// ---------------------------------------------------------------------------
//
// Аналогично Go/Rust: `dotnet new webapi` пишет TFM (net8.0/net10.0) по
// ЛОКАЛЬНО установленному SDK. Если образ зашпинить на старую версию
// (было dotnet/sdk:8.0), каркас с net10.0 не собрался бы. Версия образа
// берётся из фактически установленного `dotnet --version`.

/// Нижняя граница — fallback, когда версию определить нельзя.
pub const DOTNET_DOCKERFILE_FLOOR: &str = "8.0";

/// Извлекает «X.Y» из вывода `dotnet --version` («10.0.102»).
/// Чистая функция — тестируется без запуска процессов.
pub fn dotnet_minor_version(raw: &str) -> Option<String> {
    let mut parts = raw.trim().split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    if major.is_empty()
        || minor.is_empty()
        || !major.chars().all(|c| c.is_ascii_digit())
        || !minor.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(format!("{major}.{minor}"))
}

/// Определяет мажор+минор установленного .NET SDK. При ошибке — DOTNET_DOCKERFILE_FLOOR.
pub fn detect_dotnet_version() -> String {
    if let Ok(out) = std::process::Command::new("dotnet")
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    {
        if let Some(v) = dotnet_minor_version(&out) {
            return v;
        }
    }
    DOTNET_DOCKERFILE_FLOOR.to_string()
}

// ---------------------------------------------------------------------------
// Dockerfile content generation
// ---------------------------------------------------------------------------

pub fn generate_dockerfile_content(
    lang: &str,
    framework: Option<&str>,
    project_name: &str,
) -> Option<String> {
    match lang {
        "python" => {
            let (base_image, port, cmd) = match framework {
                Some("fastapi") => ("python:3.13-slim", "3000", "[\"python\", \"src/main.py\"]"),
                // Django: runserver обязан принимать запросы из контейнера —
                // без `0.0.0.0:8000` сервер слушает только 127.0.0.1 и наружу
                // недоступен. Раньше CMD был просто `python manage.py` — Django
                // печатал справку и завершался.
                Some("django") => (
                    "python:3.13-slim",
                    "8000",
                    "[\"python\", \"manage.py\", \"runserver\", \"0.0.0.0:8000\"]",
                ),
                Some("flask") => ("python:3.13-slim", "3000", "[\"python\", \"src/app.py\"]"),
                _ => ("python:3.13-slim", "3000", "[\"python\", \"src/main.py\"]"),
            };

            Some(format!(
                r#"FROM {base_image}

WORKDIR /app

# Install dependencies
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy application code
COPY . .

# Expose the port
EXPOSE {port}

# Run the application
CMD {cmd}
"#
            ))
        }

        "rust" => {
            let (port, bin_name) = match framework {
                Some("axum") => ("3000", project_name),
                Some("clap") => return None, // CLI не нужен Docker
                _ => ("3000", project_name),
            };
            // `cargo init` пишет edition по умолчанию установленного rustc
            // (новые версии — edition 2024), поэтому образ берём под него.
            let rust_version = detect_rust_version();

            Some(format!(
                r#"# Build stage
FROM rust:{rust_version}-slim-bookworm AS builder

WORKDIR /app
COPY . .
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app
COPY --from=builder /app/target/release/{bin_name} .

EXPOSE {port}

CMD ["./{bin_name}"]
"#
            ))
        }

        "typescript" | "javascript" | "node" => {
            let (base_image, needs_build, port, _build_steps, start_cmd) = match framework {
                Some("nextjs") => (
                    "node:22-alpine",
                    true,
                    "3000",
                    "RUN npm run build\n",
                    // `next` без подкоманды запускает dev-сервер (`next dev`);
                    // в контейнере нужен `next start`. Плюс 0.0.0.0, иначе
                    // Next.js слушает только localhost.
                    "[\"node_modules/.bin/next\", \"start\", \"-H\", \"0.0.0.0\"]",
                ),
                Some("nuxt") => (
                    "node:22-alpine",
                    true,
                    "3000",
                    "COPY . .\nRUN npm ci && npm run build\n",
                    "[\"node\", \".output/server/index.mjs\"]",
                ),
                Some("sveltekit") => (
                    "node:22-alpine",
                    true,
                    "3000",
                    "COPY . .\nRUN npm ci && npm run build\n",
                    "[\"node\", \"build/index.js\"]",
                ),
                Some("nest") => (
                    "node:22-alpine",
                    true,
                    "3000",
                    "COPY . .\nRUN npm ci && npm run build\n",
                    "[\"node\", \"dist/main.js\"]",
                ),
                Some("fastify") | Some("express") => (
                    "node:22-alpine",
                    false,
                    "3000",
                    "",
                    "[\"node\", \"src/index.js\"]",
                ),
                _ => (
                    "node:22-alpine",
                    false,
                    "3000",
                    "",
                    "[\"node\", \"src/index.js\"]",
                ),
            };

            if needs_build {
                // Для фреймворков, которым нужна сборка
                Some(format!(
                    r#"FROM {base_image}

WORKDIR /app

# Install all dependencies (including dev for build)
COPY package*.json ./
RUN npm ci

# Copy source and build
COPY . .
RUN npm run build

# Prune dev dependencies for production
RUN npm prune --production

EXPOSE {port}

CMD {start_cmd}
"#
                ))
            } else {
                // Для простых серверов без сборки
                Some(format!(
                    r#"FROM {base_image}

WORKDIR /app

# Install production dependencies only
COPY package*.json ./
RUN npm ci --only=production

# Copy application code
COPY . .

EXPOSE {port}

CMD {start_cmd}
"#
                ))
            }
        }

        "go" => {
            if framework == Some("cobra") {
                return None;
            }
            let go_version = detect_go_version();
            Some(format!(
                r#"# Build stage
FROM golang:{go_version}-alpine AS builder

WORKDIR /app
# go.sum может отсутствовать (голый `go mod init` без внешних зависимостей
# его не создаёт), поэтому не копируем go.mod/go.sum отдельно одним COPY —
# иначе сборка упадёт на отсутствующем go.sum. Копируем весь проект и даём
# `go build` самому подтянуть зависимости.
COPY . .
RUN go mod download
RUN CGO_ENABLED=0 go build -o app ./cmd/main.go

# Runtime stage
FROM alpine:3.21

WORKDIR /app
COPY --from=builder /app/app .

EXPOSE 3000

CMD ["./app"]
"#
            ))
        }
        "csharp" => {
            // `dotnet new` пишет TFM по установленному SDK (net8.0/net10.0),
            // поэтому SDK/runtime-образы берём под фактическую версию.
            let dotnet_version = detect_dotnet_version();
            let (runtime_image, port, project_file) = match framework {
                Some("aspnetcore") => (
                    format!("mcr.microsoft.com/dotnet/aspnet:{dotnet_version}"),
                    "EXPOSE 8080",
                    format!("{}.csproj", project_name),
                ),
                _ => (
                    format!("mcr.microsoft.com/dotnet/runtime:{dotnet_version}"),
                    "",
                    format!("{}.csproj", project_name),
                ),
            };

            Some(format!(
                r#"FROM mcr.microsoft.com/dotnet/sdk:{dotnet_version} AS build
WORKDIR /src
COPY {project_file} .
RUN dotnet restore
COPY . .
RUN dotnet publish -c Release -o /app/publish

FROM {runtime_image} AS final
WORKDIR /app
{port}
COPY --from=build /app/publish .
ENTRYPOINT ["dotnet", "{project_name}.dll"]
"#
            ))
        }
        "java" => {
            let has_spring = framework == Some("spring-boot");
            if !has_spring {
                return None; // Только Spring Boot поддерживает Docker из коробки
            }
            Some(format!(
                r#"FROM eclipse-temurin:21-jdk-alpine AS build
WORKDIR /app
COPY mvnw pom.xml ./
COPY .mvn .mvn
RUN ./mvnw dependency:go-offline
COPY src ./src
RUN ./mvnw package -DskipTests

FROM eclipse-temurin:21-jre-alpine AS final
WORKDIR /app
COPY --from=build /app/target/*.jar app.jar
EXPOSE 8080
ENTRYPOINT ["java", "-jar", "app.jar"]
"#
            ))
        }
        "php" => {
            let has_laravel = framework == Some("laravel") || framework == Some("symfony");
            if !has_laravel {
                return None;
            }
            Some(format!(
                r#"FROM php:8.3-fpm-alpine

RUN docker-php-ext-install pdo pdo_mysql

COPY --from=composer:2 /usr/bin/composer /usr/bin/composer

WORKDIR /app
COPY composer.json composer.lock ./
RUN composer install --no-dev --optimize-autoloader

COPY . .
RUN php artisan config:cache || true

EXPOSE 8000
CMD ["php", "artisan", "serve", "--host=0.0.0.0", "--port=8000"]
"#
            ))
        }
        "elixir" => {
            let is_phoenix = framework == Some("phoenix");
            if !is_phoenix {
                return None;
            }
            Some(format!(
                r#"FROM hexpm/elixir:1.17-erlang-27-alpine AS build
WORKDIR /app
RUN mix local.hex --force && mix local.rebar --force
COPY mix.exs mix.lock ./
RUN mix deps.get --only prod
COPY . .
RUN mix compile

FROM hexpm/elixir:1.17-erlang-27-alpine AS final
WORKDIR /app
COPY --from=build /app/_build/prod/rel/{project_name} .
EXPOSE 4000
CMD ["./bin/{project_name}", "start"]
"#
            ))
        }
        "cpp" => {
            let is_qt = framework == Some("qt");
            if is_qt {
                Some(format!(
                    r#"FROM stateoftheartio/qt6:6.7-gcc-ubuntu-24.04 AS build
WORKDIR /app
COPY . .
RUN mkdir build && cd build && \
    qt-cmake -DCMAKE_BUILD_TYPE=Release .. && \
    cmake --build .

FROM ubuntu:24.04 AS final
RUN apt-get update && apt-get install -y \
    libqt6gui6 libqt6core6 libqt6widgets6 \
    libgl1-mesa-glx \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /app/build/{project_name} .
ENTRYPOINT ["./{project_name}", "-platform", "offscreen"]"#
                ))
            } else {
                // Обычный C/C++-скаффолд пишет Makefile (не CMakeLists.txt),
                // поэтому собираем `make`, а не cmake. Бинарь — {project_name}.
                Some(format!(
                    r#"FROM ubuntu:24.04 AS build
RUN apt-get update && apt-get install -y \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .
RUN make

FROM ubuntu:24.04 AS final
WORKDIR /app

COPY --from=build /app/{project_name} .
ENTRYPOINT ["./{project_name}"]"#
                ))
            }
        }
        "zig" => {
            // Зависимости (zap) подтягиваются автоматически: `zig build` сам
            // выполняет fetch-фазу из build.zig.zon (флаг --fetch останавливается
            // ПОСЛЕ загрузки зависимостей и не собирает бинарь — zig-out/bin
            // остался бы пустым). Образ 0.14 — минимальная версия для zap v0.10.1
            // (в zon пишется minimum_zig_version = "0.14.0").
            Some(format!(
                r#"FROM ziglang/zig:0.14.0 AS build
WORKDIR /app

COPY build.zig build.zig.zon ./
COPY src/ ./src/

RUN zig build -Doptimize=ReleaseFast

FROM scratch
WORKDIR /

COPY --from=build /app/zig-out/bin/{project_name} /{project_name}

ENTRYPOINT ["/{project_name}"]
"#
            ))
        }
        "kotlin" => {
            let has_fw = framework == Some("ktor") || framework == Some("spring-boot");
            if has_fw {
                if framework == Some("ktor") {
                    // ktor биндит Netty на 3000 (см. scaffold). Обычный
                    // `gradle build` даёт ТОНКИЙ jar — `java -jar` без
                    // classpath не стартует. application-плагин позволяет
                    // `gradle installDist` собрать самодостаточный
                    // дистрибутив; имя = rootProject.name (safe_name).
                    let app_name = project_name.replace(['-', ' '], "_").replace('"', "_");
                    Some(format!(
                        r#"FROM gradle:8.10-jdk21 AS build
WORKDIR /app
COPY build.gradle.kts settings.gradle.kts ./
RUN gradle dependencies --no-daemon
COPY src ./src
RUN gradle installDist --no-daemon

FROM eclipse-temurin:21-jre-alpine AS final
WORKDIR /app
COPY --from=build /app/build/install/ ./
EXPOSE 3000
CMD ["./{app_name}/bin/{app_name}"]
"#
                    ))
                } else {
                    // Spring Boot: `gradle build` даёт толстый bootJar —
                    // `java -jar` работает как есть.
                    Some(format!(
                        r#"FROM gradle:8.10-jdk21 AS build
WORKDIR /app
COPY build.gradle.kts settings.gradle.kts ./
RUN gradle dependencies --no-daemon
COPY src ./src
RUN gradle build -x test --no-daemon

FROM eclipse-temurin:21-jre-alpine AS final
WORKDIR /app
EXPOSE 8080
# Копируем все JAR файлы и находим тот, что без plain/sources
COPY --from=build /app/build/libs/ ./libs/
RUN cp $(ls ./libs/*.jar | grep -v -E 'plain|sources|javadoc') app.jar
ENTRYPOINT ["java", "-jar", "app.jar"]
"#
                    ))
                }
            } else {
                None // Без фреймворка не генерируем Dockerfile
            }
        }
        "swift" => {
            let has_fw = framework == Some("vapor");
            if has_fw {
                Some(format!(
                    r#"FROM swift:6.0-noble AS build
WORKDIR /build
COPY Package.swift Package.resolved ./
RUN swift package resolve
COPY . .
RUN swift build -c release --static-swift-backtrace

FROM swift:6.0-noble-slim AS final
WORKDIR /app
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*
EXPOSE 8080
COPY --from=build /build/.build/release/{project_name} ./App
COPY --from=build /build/Public ./Public
ENTRYPOINT ["./App"]
"#
                ))
            } else {
                Some(format!(
                    r#"FROM swift:6.0-noble AS build
WORKDIR /build
COPY Package.swift ./
RUN swift package resolve
COPY . .
RUN swift build -c release

FROM swift:6.0-noble-slim AS final
WORKDIR /app
COPY --from=build /build/.build/release/{project_name} ./
ENTRYPOINT ["./{project_name}"]
"#
                ))
            }
        }
        "dart" => {
            // `dart create` кладёт пакет в подпапку `{safe_name}/` (см. scaffold),
            // а не в корень — поэтому собираем внутри этой подпапки.
            let safe_name = project_name.replace('-', "_");
            Some(format!(
                r#"# Этап сборки
FROM dart:3.5 AS build
WORKDIR /app
COPY . .
WORKDIR /app/{safe_name}
RUN dart pub get
RUN dart compile exe bin/main.dart -o bin/main

# Этап запуска
FROM scratch
COPY --from=build /app/{safe_name}/bin/main /main
ENTRYPOINT ["/main"]
"#
            ))
        }
        "gleam" => {
            // `gleam new` кладёт проект в подпапку `{project_name}/` (см.
            // scaffold), а не в корень — собираем внутри этой подпапки.
            let project_dir = project_name;
            Some(format!(
                r#"FROM ghcr.io/gleam-lang/gleam:v1.4-erlang-alpine AS build
WORKDIR /app
COPY . .
WORKDIR /app/{project_dir}
RUN gleam deps download
RUN gleam export erlang-shipment

FROM erlang:27-alpine AS final
WORKDIR /app
COPY --from=build /app/{project_dir}/build/erlang-shipment ./
ENTRYPOINT ["/app/entrypoint.sh"]
"#
            ))
        }
        _ => None, // Неизвестный язык — не генерируем Dockerfile
    }
}

// ---------------------------------------------------------------------------
// Docker Compose generation
// ---------------------------------------------------------------------------

/// Именованный том Docker или bind-mount? Bind-mount начинается с ".", "/",
/// "\\", "~" или Windows drive letter (C:\...). Только именованные тома можно
/// объявлять в глобальной секции `volumes:` docker-compose.yaml — bind-mount
/// обязан жить исключительно внутри сервиса.
///
/// Жёсткое правило шаблонизатора: в глобальный блок volumes: попадают ТОЛЬКО
/// элементы, host-часть которых не содержит разделителей пути "/" или "\\"
/// (имя именованного тома не может содержать слэши). Относительные пути без
/// "./"-префикса ("dags/data:/opt/app/data") — тоже bind-mounts.
fn is_named_volume(volume: &str) -> bool {
    if volume.is_empty() {
        return false;
    }
    // Windows drive letter: "C:\data:/data" — двоеточие внутри host-пути
    // (проверяем по всей строке, до split по ':').
    let bytes = volume.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return false;
    }
    let host_part = volume.split(':').next().unwrap_or(volume);
    if host_part.is_empty() {
        return false;
    }
    let first = host_part.as_bytes()[0];
    if matches!(first, b'.' | b'/' | b'\\' | b'~') {
        return false;
    }
    // Имя именованного тома не содержит слэшей; любой "/" или "\\" в
    // host-части означает локальный путь хоста — в глобальный volumes: ему
    // нельзя (Docker Compose: «Property is not allowed»).
    if host_part.contains('/') || host_part.contains('\\') {
        return false;
    }
    true
}

pub fn generate_docker_compose(
    services: &[DockerService],
    project_name: &str,
    app_port: &str,
    build_context: &str,
    include_app: bool,
) -> String {
    let mut result = String::new();
    let mut volumes_section = String::new();
    // Контекст сборки приложения: "." в корневой раскладке, "./backend" в
    // split-проектах (Dockerfile живёт внутри backend/, см. steps_for_docker).
    let build_ctx = if build_context.is_empty() {
        "."
    } else {
        build_context
    };

    result.push_str("version: '3.8'\n\nservices:\n");

    // App-сервис добавляется ТОЛЬКО если для языка/фреймворка генерируется
    // Dockerfile (include_app). Иначе `docker compose up --build` ссылался бы
    // на несуществующий build-контекст/Dockerfile (rust+clap, go+cobra,
    // java без spring-boot и т.п.).
    if include_app {
        result.push_str(&format!(
            r#"  app:
    build: {build_ctx}
    container_name: {project_name}_app
    ports:
      - "{app_port}:{app_port}"
"#
        ));
    }

    // Если есть сервисы — добавляем depends_on
    if include_app && !services.is_empty() {
        result.push_str("    depends_on:\n");
        for service in services.iter() {
            result.push_str(&format!("      - {}\n", service.name));
        }
        // Стандартные переменные окружения для подключения к сервисам.
        // environment: пишется ТОЛЬКО при наличии записей — пустой блок
        // парсится как null и ломает compose («environment must be a
        // mapping») при сервисах вроде zookeeper/kafka, для которых
        // переменных нет.
        let mut app_env: Vec<&str> = Vec::new();
        for service in services.iter() {
            match service.name.as_str() {
                "postgres" => {
                    app_env.push("DATABASE_URL=postgresql://postgres:12345@postgres:5432/postgres")
                }
                "redis" => app_env.push("REDIS_URL=redis://redis:6379/0"),
                _ => {}
            }
        }
        if !app_env.is_empty() {
            result.push_str("    environment:\n");
            for line in app_env {
                result.push_str(&format!("      - {}\n", line));
            }
        }
    }
    result.push('\n');

    // Сервисы БД/кешей
    for service in services.iter() {
        result.push_str(&format!("  {}:\n", service.name));
        result.push_str(&format!("    image: {}\n", service.image));
        result.push_str(&format!(
            "    container_name: {}_{}\n",
            project_name, service.name
        ));

        if !service.ports.is_empty() {
            result.push_str("    ports:\n");
            for port in &service.ports {
                result.push_str(&format!("      - \"{}\"\n", port));
            }
        }

        if !service.environment.is_empty() {
            result.push_str("    environment:\n");
            for (key, value) in &service.environment {
                result.push_str(&format!("      {}: {}\n", key.to_uppercase(), value));
            }
        }

        if !service.volumes.is_empty() {
            result.push_str("    volumes:\n");
            for volume in &service.volumes {
                result.push_str(&format!("      - {}\n", volume));
                // В глобальную секцию volumes в конце файла попадают ТОЛЬКО
                // именованные тома (postgres_data). Bind-mounts (./dags:...) —
                // это локальные пути хоста, их объявлять на верхнем уровне
                // нельзя (Docker Compose: «Property is not allowed»).
                if is_named_volume(volume) {
                    let vol_name = volume.split(':').next().unwrap_or(volume);
                    volumes_section.push_str(&format!("\n  {}:", vol_name));
                }
            }
        }
        if !service.depends_on.is_empty() {
            result.push_str("    depends_on:\n");
            for dep in &service.depends_on {
                result.push_str(&format!("      - {}\n", dep.to_lowercase()))
            }
        }

        result.push('\n');
    }

    // Секция volumes в конце файла
    if !volumes_section.is_empty() {
        result.push_str("volumes:");
        result.push_str(&volumes_section);
        result.push('\n');
    }

    result
}

// ---------------------------------------------------------------------------
// .dockerignore
// ---------------------------------------------------------------------------

pub fn dockerignore_content(lang: &str) -> String {
    let common = ".git\n.gitignore\n.env\n*.md\n";
    let specific = match lang {
        "rust" => "target/\n",
        "python" => "__pycache__/\n.venv/\n*.pyc\n",
        _ => "node_modules/\ndist/\n",
    };
    format!("{}{}", common, specific)
}

// ---------------------------------------------------------------------------
// .gitignore
// ---------------------------------------------------------------------------

pub fn gitignore_content(languages: &[String]) -> String {
    let mut content = String::from(
        "# OS generated files\n\
         .DS_Store\n\
         .DS_Store?\n\
         ._*\n\
         .Spotlight-V100\n\
         .Trashes\n\
         ehthumbs.db\n\
         Thumbs.db\n\
         \n\
         # IDE\n\
         .vscode/\n\
         .idea/\n\
         *.swp\n\
         *.swo\n\
         *~\n\
         \n\
         # Environment\n\
         .env\n\
         .env.local\n\
         .env.*.local\n\
         \n\
         # Logs\n\
         *.log\n\
         logs/\n\
         \n",
    );

    for lang in languages {
        match lang.as_str() {
            "rust" => {
                content.push_str(
                    "# Rust\n\
                                  target/\n\
                                  **/*.rs.bk\n\
                                  *.pdb\n\
                                  \n",
                );
            }
            "python" => {
                content.push_str(
                    "# Python\n\
                                  __pycache__/\n\
                                  *.py[cod]\n\
                                  *$py.class\n\
                                  *.so\n\
                                  .Python\n\
                                  build/\n\
                                  develop-eggs/\n\
                                  dist/\n\
                                  downloads/\n\
                                  eggs/\n\
                                  .eggs/\n\
                                  lib/\n\
                                  lib64/\n\
                                  parts/\n\
                                  sdist/\n\
                                  var/\n\
                                  wheels/\n\
                                  *.egg-info/\n\
                                  .installed.cfg\n\
                                  *.egg\n\
                                  MANIFEST\n\
                                  *.manifest\n\
                                  *.spec\n\
                                  pip-log.txt\n\
                                  pip-delete-this-directory.txt\n\
                                  htmlcov/\n\
                                  .tox/\n\
                                  .nox/\n\
                                  .coverage\n\
                                  .coverage.*\n\
                                  .cache\n\
                                  nosetests.xml\n\
                                  coverage.xml\n\
                                  *.cover\n\
                                  .hypothesis/\n\
                                  .pytest_cache/\n\
                                  *.mo\n\
                                  *.pot\n\
                                  venv/\n\
                                  .venv/\n\
                                  ENV/\n\
                                  env/\n\
                                  \n",
                );
            }
            "typescript" | "javascript" | "node" => {
                content.push_str(
                    "# Node\n\
                                  node_modules/\n\
                                  npm-debug.log*\n\
                                  yarn-debug.log*\n\
                                  yarn-error.log*\n\
                                  lerna-debug.log*\n\
                                  .pnpm-debug.log*\n\
                                  report.[0-9]*.[0-9]*.[0-9]*.[0-9]*.json\n\
                                  pids\n\
                                  *.pid\n\
                                  *.seed\n\
                                  *.pid.lock\n\
                                  lib-cov\n\
                                  coverage/\n\
                                  .nyc_output\n\
                                  .grunt\n\
                                  bower_components\n\
                                  .lock-wscript\n\
                                  build/Release\n\
                                  jspm_packages/\n\
                                  typings/\n\
                                  .npm\n\
                                  .eslintcache\n\
                                  .node_repl_history\n\
                                  *.tgz\n\
                                  .yarn-integrity\n\
                                  .next/\n\
                                  .nuxt/\n\
                                  dist/\n\
                                  \n",
                );
            }
            "go" => {
                content.push_str(
                    "# Go\n\
                                  *.exe\n\
                                  *.exe~\n\
                                  *.dll\n\
                                  *.so\n\
                                  *.dylib\n\
                                  *.test\n\
                                  *.out\n\
                                  go.work\n\
                                  \n",
                );
            }
            "java" => {
                content.push_str(
                    "# Java\n\
                                  *.class\n\
                                  *.jar\n\
                                  *.war\n\
                                  *.nar\n\
                                  *.ear\n\
                                  *.zip\n\
                                  *.tar.gz\n\
                                  *.rar\n\
                                  hs_err_pid*\n\
                                  .gradle/\n\
                                  build/\n\
                                  target/\n\
                                  \n",
                );
            }
            "csharp" => {
                content.push_str(
                    "# .NET\n\
                                  bin/\n\
                                  obj/\n\
                                  *.user\n\
                                  *.suo\n\
                                  *.cache\n\
                                  *.docstates\n\
                                  packages/\n\
                                  \n",
                );
            }
            "cpp" | "c" => {
                content.push_str(
                    "# C/C++\n\
                                  *.o\n\
                                  *.obj\n\
                                  *.exe\n\
                                  *.out\n\
                                  *.app\n\
                                  *.a\n\
                                  *.so\n\
                                  *.dylib\n\
                                  \n",
                );
            }
            "php" => {
                content.push_str(
                    "# PHP\n\
                                  vendor/\n\
                                  composer.lock\n\
                                  \n",
                );
            }
            "zig" => {
                content.push_str(
                    "# Zig\n\
                                  zig-out/\n\
                                  zig-cache/\n\
                                  \n",
                );
            }
            "swift" => {
                content.push_str(
                    "# Swift\n\
                                  .build/\n\
                                  DerivedData/\n\
                                  *.xcodeproj\n\
                                  *.xcworkspace\n\
                                  \n",
                );
            }
            "kotlin" => {
                content.push_str(
                    "# Kotlin\n\
                                  .gradle/\n\
                                  build/\n\
                                  .idea/\n\
                                  *.iml\n\
                                  out/\n\
                                  local.properties\n\
                                  \n",
                );
            }
            "elixir" => {
                content.push_str(
                    "# Elixir\n\
                                  _build/\n\
                                  deps/\n\
                                  .elixir_ls/\n\
                                  \n",
                );
            }
            "dart" => {
                content.push_str(
                    "# Dart\n\
                                  .dart_tool/\n\
                                  .packages\n\
                                  build/\n\
                                  pubspec.lock\n\
                                  \n",
                );
            }
            "gleam" => {
                content.push_str(
                    "# Gleam\n\
                                  build/\n\
                                  \n",
                );
            }
            _ => {}
        }
    }

    content
}

// ---------------------------------------------------------------------------
// CI workflow content
// ---------------------------------------------------------------------------

pub fn generate_ci_content(lang: &str, _framework: Option<&str>, _project_name: &str) -> String {
    match lang {
        "rust" => format!(
            r#"name: CI

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

env:
  CARGO_TERM_COLOR: always

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
      - name: Cache dependencies
        uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target
          key: ${{{{ runner.os }}}}-cargo-${{{{ hashFiles('**/Cargo.lock') }}}}
      - name: Run tests
        run: cargo test --verbose
      - name: Build
        run: cargo build --release
"#
        ),

        "python" => format!(
            r#"name: CI

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Set up Python
        uses: actions/setup-python@v5
        with:
          python-version: '3.13'
      - name: Install dependencies
        run: |
          python -m pip install --upgrade pip
          pip install -r requirements.txt
      - name: Lint with ruff
        run: |
          pip install ruff
          ruff check .
      - name: Test with pytest
        run: |
          pip install pytest
          pytest
"#
        ),

        "typescript" | "javascript" | "node" => format!(
            r#"name: CI

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Use Node.js
        uses: actions/setup-node@v4
        with:
          node-version: '22'
          cache: 'npm'
      - name: Install dependencies
        run: npm ci
      - name: Run tests
        run: npm test
      - name: Build
        run: npm run build --if-present
"#
        ),

        "go" => {
            let go_version = detect_go_version();
            format!(
                r#"name: CI

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Set up Go
        uses: actions/setup-go@v5
        with:
          go-version: '{go_version}'
      - name: Test
        run: go test ./...
      - name: Build
        run: go build -v ./...
"#
            )
        }

        "java" => format!(
            r#"name: CI

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Set up JDK 21
        uses: actions/setup-java@v4
        with:
          java-version: '21'
          distribution: 'temurin'
      - name: Setup Gradle
        uses: gradle/gradle-build-action@v3
      - name: Run tests
        run: ./gradlew test
      - name: Build
        run: ./gradlew build -x test
"#
        ),

        _ => format!(
            r#"name: CI

on:
  push:
    branches: [ main, master ]
  pull_request:
    branches: [ main, master ]

jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Run build script
        run: echo "Add build steps here for {lang} project"
"#
        ),
    }
}

// ---------------------------------------------------------------------------
// README content
// ---------------------------------------------------------------------------
//
// README generation was replaced by the typed, composable subsystem in
// engine/readme.rs (see readme::generate_readme). The old single-purpose
// generate_readme(name, lang, framework, project_type, tools, structure) only
// saw the FIRST language and FIRST framework, so it has been removed — all
// README content now comes from the new module, which receives the complete
// WizardContext and ProjectLayout.

// ---------------------------------------------------------------------------
// VS Code settings
// ---------------------------------------------------------------------------

pub fn generate_vscode_settings(lang: &str) -> String {
    match lang {
        "rust" => r#"{
    "rust-analyzer.checkOnSave.command": "clippy",
    "[rust]": {
        "editor.formatOnSave": true
    }
}"#
        .to_string(),

        "python" => r#"{
    "python.defaultInterpreterPath": "${workspaceFolder}/.venv/bin/python",
    "python.analysis.typeCheckingMode": "basic",
    "[python]": {
        "editor.formatOnSave": true,
        "editor.defaultFormatter": "charliermarsh.ruff",
        "editor.codeActionsOnSave": {
            "source.organizeImports": "explicit"
        }
    }
}"#
        .to_string(),

        "typescript" | "javascript" | "node" => r#"{
    "typescript.tsdk": "node_modules/typescript/lib",
    "editor.formatOnSave": true,
    "editor.defaultFormatter": "esbenp.prettier-vscode",
    "[typescript]": {
        "editor.defaultFormatter": "esbenp.prettier-vscode"
    }
}"#
        .to_string(),

        "go" => r#"{
    "go.useLanguageServer": true,
    "go.lintTool": "golangci-lint",
    "go.formatTool": "goimports",
    "[go]": {
        "editor.formatOnSave": true,
        "editor.codeActionsOnSave": {
            "source.organizeImports": "explicit"
        }
    }
}"#
        .to_string(),

        _ => r#"{
    "editor.formatOnSave": true
}"#
        .to_string(),
    }
}

// ---------------------------------------------------------------------------
// VS Code extensions
// ---------------------------------------------------------------------------

pub fn generate_vscode_extensions(lang: &str) -> String {
    match lang {
        "rust" => r#"{
    "recommendations": [
        "rust-lang.rust-analyzer",
        "tamasfe.even-better-toml"
    ]
}"#
        .to_string(),

        "python" => r#"{
    "recommendations": [
        "ms-python.python",
        "charliermarsh.ruff",
        "ms-python.mypy-type-checker"
    ]
}"#
        .to_string(),

        "typescript" | "javascript" | "node" => r#"{
    "recommendations": [
        "dbaeumer.vscode-eslint",
        "esbenp.prettier-vscode"
    ]
}"#
        .to_string(),

        "go" => r#"{
    "recommendations": [
        "golang.go"
    ]
}"#
        .to_string(),

        "java" => r#"{
    "recommendations": [
        "vscjava.vscode-java-pack"
    ]
}"#
        .to_string(),

        "csharp" => r#"{
    "recommendations": [
        "ms-dotnettools.csharp"
    ]
}"#
        .to_string(),

        _ => r#"{
    "recommendations": []
}"#
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_mounts_stay_out_of_global_volumes_block() {
        // airflow монтирует ./dags и ./logs — это bind-mounts, их НЕЛЬЗЯ
        // объявлять в глобальной секции volumes (Property is not allowed).
        let services = collect_docker_services(&["airflow".into(), "postgresql".into()]);
        let compose = generate_docker_compose(&services, "myproj", "3000", ".", true);

        // Bind-mount остаётся внутри сервиса
        assert!(compose.contains("./dags:/opt/airflow/dags"), "{compose}");
        assert!(compose.contains("./logs:/opt/airflow/logs"), "{compose}");

        // Глобальная секция volumes: без именованных томов её не должно
        // быть вовсе (раньше туда попадали ./dags и ./logs).
        assert!(
            !compose.contains("\nvolumes:\n\n  ./"),
            "bind-mount попал в глобальный volumes: {compose}"
        );
        assert!(
            !compose.contains("\nvolumes:"),
            "глобальная volumes не нужна без именованных томов: {compose}"
        );
    }

    #[test]
    fn named_volumes_are_declared_globally() {
        let services = vec![DockerService {
            name: "postgres".into(),
            image: "postgres:16-alpine".into(),
            ports: vec!["5432:5432".into()],
            environment: vec![],
            volumes: vec!["postgres_data:/var/lib/postgresql/data".into()],
            depends_on: vec![],
        }];
        let compose = generate_docker_compose(&services, "myproj", "3000", ".", true);
        assert!(
            compose.contains("volumes:\n\n  postgres_data:")
                || compose.contains("\nvolumes:\n  postgres_data:"),
            "{compose}"
        );
    }

    #[test]
    fn is_named_volume_detects_bind_mounts() {
        assert!(!is_named_volume("./dags:/opt/airflow/dags"));
        assert!(!is_named_volume("/host/path:/container/path"));
        assert!(!is_named_volume("~/data:/data"));
        assert!(!is_named_volume(r"C:\data:/data"));
        assert!(!is_named_volume(r"\\server\share:/data"));
        // Относительный путь БЕЗ "./": host-часть содержит "/" — тоже
        // bind-mount, в глобальный volumes: ему нельзя.
        assert!(!is_named_volume("dags/data:/opt/app/data"));
        assert!(is_named_volume("postgres_data:/var/lib/postgresql/data"));
    }

    #[test]
    fn relative_bind_mounts_stay_out_of_global_volumes_block() {
        // host-часть не начинается с "./", но содержит "/" (например,
        // "dags/data:...") — глобальный блок volumes: объявлять такие
        // элементы не должен.
        let services = vec![DockerService {
            name: "app".into(),
            image: "custom-app:latest".into(),
            ports: vec![],
            environment: vec![],
            volumes: vec!["dags/data:/opt/app/data".into()],
            depends_on: vec![],
        }];
        let compose = generate_docker_compose(&services, "myproj", "3000", ".", true);
        assert!(compose.contains("- dags/data:/opt/app/data"), "{compose}");
        assert!(
            !compose.contains("\nvolumes:"),
            "относительный bind-mount не объявляется глобально: {compose}"
        );
    }

    #[test]
    fn go_minor_version_parses_full_patch_output() {
        assert_eq!(
            go_minor_version("go version go1.25.3 windows/amd64"),
            Some("1.25".into())
        );
        assert_eq!(go_minor_version("go version go1.24.0"), Some("1.24".into()));
        assert_eq!(
            go_minor_version("go version go1.22 linux/amd64"),
            Some("1.22".into())
        );
    }

    #[test]
    fn go_minor_version_rejects_garbage() {
        assert_eq!(go_minor_version("not a version"), None);
        assert_eq!(go_minor_version(""), None);
        // Символ 'go' без номера после него — не версия.
        assert_eq!(go_minor_version("go version"), None);
    }

    #[test]
    fn rust_minor_version_parses_rustc_output() {
        assert_eq!(
            rust_minor_version("rustc 1.97.1 (8bab26f4f 2026-07-14)"),
            Some("1.97".into())
        );
        assert_eq!(rust_minor_version("rustc 1.85.0"), Some("1.85".into()));
        assert_eq!(rust_minor_version("garbage"), None);
        assert_eq!(rust_minor_version("rustc"), None);
    }

    #[test]
    fn dotnet_minor_version_parses_sdk_output() {
        assert_eq!(dotnet_minor_version("10.0.102\n"), Some("10.0".into()));
        assert_eq!(dotnet_minor_version("8.0.404"), Some("8.0".into()));
        assert_eq!(dotnet_minor_version("garbage"), None);
        assert_eq!(dotnet_minor_version(""), None);
        assert_eq!(dotnet_minor_version("8"), None);
    }

    #[test]
    fn rust_dockerfile_uses_detected_version_not_hardcoded() {
        // rust:1.83 не умеет edition 2024, которое `cargo init` пишет под
        // новый rustc (>=1.85). Образ обязан использовать detected версию.
        let dockerfile =
            generate_dockerfile_content("rust", Some("axum"), "myapi").expect("rust dockerfile");
        assert!(dockerfile.contains("FROM rust:"), "{dockerfile}");
        assert!(
            !dockerfile.contains("FROM rust:1.83"),
            "устаревший rust:1.83 не соберёт edition 2024: {dockerfile}"
        );
    }

    #[test]
    fn csharp_dockerfile_uses_detected_version_not_hardcoded() {
        // dotnet new webapi пишет net10.0 под SDK 10.x — образ 8.0 не соберёт.
        let dockerfile = generate_dockerfile_content("csharp", Some("aspnetcore"), "myweb")
            .expect("csharp dockerfile");
        assert!(dockerfile.contains("dotnet/sdk:"), "{dockerfile}");
        assert!(
            !dockerfile.contains("dotnet/sdk:8.0"),
            "устаревший sdk:8.0 не соберёт net10: {dockerfile}"
        );
    }

    #[test]
    fn ktor_dockerfile_exposes_server_actual_port() {
        // ktor-каркас биндит Netty на 3000 (см. scaffold embeddedServer),
        // поэтому EXPOSE обязан быть 3000, а не 8080.
        let dockerfile =
            generate_dockerfile_content("kotlin", Some("ktor"), "myapi").expect("ktor dockerfile");
        assert!(dockerfile.contains("EXPOSE 3000"), "{dockerfile}");
        assert!(!dockerfile.contains("EXPOSE 8080"), "{dockerfile}");
    }

    #[test]
    fn go_dockerfile_uses_detected_version_not_hardcoded() {
        // Dockerfile для Go НЕ должен содержать жёстко зашитую версию, иначе
        // он разъедется с go.mod, который `go mod init` пишет под установленный Go.
        let dockerfile =
            generate_dockerfile_content("go", Some("gin"), "myapi").expect("go dockerfile");
        assert!(dockerfile.contains("FROM golang:"), "{dockerfile}");
        assert!(dockerfile.contains("-alpine AS builder"), "{dockerfile}");
        // Никакого «golang:1.24-alpine» захардкоженного.
        assert!(
            !dockerfile.contains("FROM golang:1.24-alpine"),
            "Dockerfile должен брать версию Go динамически: {dockerfile}"
        );
    }

    #[test]
    fn go_ci_uses_detected_version_not_hardcoded() {
        let ci = generate_ci_content("go", None, "myapi");
        assert!(
            !ci.contains("go-version: '1.24'"),
            "CI захардкожена версия: {ci}"
        );
        assert!(
            ci.contains("go-version: '"),
            "CI должна использовать detected версию: {ci}"
        );
    }

    #[test]
    fn django_dockerfile_runs_runserver_not_bare_manage_py() {
        // Голый `python manage.py` печатает справку Django и завершается.
        // В контейнере должен запускаться runserver на 0.0.0.0:8000.
        let dockerfile = generate_dockerfile_content("python", Some("django"), "myapp")
            .expect("django dockerfile");
        assert!(dockerfile.contains("EXPOSE 8000"), "{dockerfile}");
        assert!(
            !dockerfile.contains("CMD [\"python\", \"manage.py\"]"),
            "голый manage.py не запускает сервер: {dockerfile}"
        );
        assert!(
            dockerfile.contains("\"manage.py\", \"runserver\", \"0.0.0.0:8000\""),
            "django CMD должен запускать runserver на 0.0.0.0:8000: {dockerfile}"
        );
    }

    #[test]
    fn nextjs_dockerfile_uses_next_start_not_bare_next() {
        // Голый `next` поднимает dev-сервер, а не production-сервер.
        let dockerfile = generate_dockerfile_content("typescript", Some("nextjs"), "myweb")
            .expect("nextjs dockerfile");
        assert!(
            dockerfile.contains("next\", \"start\", \"-H\", \"0.0.0.0\""),
            "{dockerfile}"
        );
    }
}
