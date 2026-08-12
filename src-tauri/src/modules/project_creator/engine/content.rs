// Content generation functions — pure data/string generators,
// separated from recipe composition logic.

// ---------------------------------------------------------------------------
// Environment example templates
// ---------------------------------------------------------------------------

pub fn get_env_example(tool_id: &str) -> String {
    match tool_id {
        "postgresql" => r#"POSTGRES_USER=user
POSTGRES_PASSWORD=password
POSTGRES_DB=dbname
POSTGRES_PORT=5432
DATABASE_URL=postgresql://user:password@localhost:5432/dbname
"#.to_string(),
        "redis" => r#"REDIS_URL=redis://localhost:6379/0
"#.to_string(),
        "mongodb" => r#"MONGODB_URI=mongodb://localhost:27017
MONGODB_DB=dbname
"#.to_string(),
        "mysql" => r#"MYSQL_ROOT_PASSWORD=rootpassword
MYSQL_DATABASE=dbname
MYSQL_USER=user
MYSQL_PASSWORD=password
MYSQL_PORT=3306
"#.to_string(),
        "airflow" => r#"AIRFLOW__CORE__EXECUTOR=LocalExecutor
AIRFLOW__DATABASE__SQL_ALCHEMY_CONN=postgresql+psycopg2://postgres:12345@postgres:5432/postgres
AIRFLOW__CORE__LOAD_EXAMPLES=False
AIRFLOW_WEBSERVER_PORT=8080
"#.to_string(),
        "grafana" => r#"GRAFANA_URL=http://localhost:3001
"#.to_string(),
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
            "postgresql" => r#"## PostgreSQL

- The superuser password was generated during installation and shown ONCE in
  StackPilot (`Generated passwords` window after installation).
- Create the database before first run:
  ```powershell
  createdb -U postgres dbname
  ```
- Connection: see `POSTGRES_*` / `DATABASE_URL` in `.env.example`.
"#,
            "redis" => r#"## Redis

- Start the server once (install directory is on PATH):
  ```powershell
  redis-server
  ```
- Connection: `REDIS_URL=redis://localhost:6379/0`.
"#,
            "mongodb" => r#"## MongoDB

- Create a data folder and start the server once (install directory is on PATH):
  ```powershell
  mkdir $env:USERPROFILE\mongo-data
  mongod --dbpath $env:USERPROFILE\mongo-data
  ```
- Connection: `MONGODB_URI=mongodb://localhost:27017`.
"#,
            "mysql" => r#"## MySQL

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
"#,
            "kafka" => r#"## Apache Kafka

- Single-node KRaft mode. From the Kafka install directory (`bin/windows`):
  ```powershell
  kafka-storage random-uuid > uuid.txt
  kafka-storage format -t (Get-Content uuid.txt) -c ..\config\kraft\server.properties
  kafka-server-start ..\config\kraft\server.properties
  ```
- Connection: `KAFKA_BOOTSTRAP_SERVERS=localhost:9092`.
"#,
            "grafana" => r#"## Grafana

- Start the server once (install directory is on PATH):
  ```powershell
  grafana-server
  ```
- UI: http://localhost:3000 (admin/admin on first run).
- Connection: `GRAFANA_URL=http://localhost:3000`.
"#,
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
                        ("KAFKA_ADVERTISED_LISTENERS".into(), "PLAINTEXT://localhost:9092".into()),
                        ("KAFKA_OFFSETS_TOPIC_REPLICATION_FACTOR".into(), "1".into()),
                    ],
                    volumes: Vec::new(),
                    depends_on: vec!["Zookeeper".into()],
                });
            },

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

            "airflow" => services.push(DockerService {
                name: "airflow".into(),
                image: "apache/airflow:2.10.4".into(),
                ports: vec!["8080:8080".into()],
                environment: vec![
                    ("AIRFLOW__CORE__EXECUTOR".into(), "LocalExecutor".into()),
                    ("AIRFLOW__DATABASE__SQL_ALCHEMY_CONN".into(), "postgresql+psycopg2://postgres:12345@postgres:5432/postgres".into()),
                    ("AIRFLOW__CORE__LOAD_EXAMPLES".into(), "False".into()),
                    ("AIRFLOW__WEBSERVER__SECRET_KEY".into(), "airflow-secret-key".into()),
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
// Dockerfile content generation
// ---------------------------------------------------------------------------

pub fn generate_dockerfile_content(
    lang: &str,
    framework: Option<&str>,
    project_name: &str,
) -> Option<String> {
    match lang {
        "python" => {
            let (base_image, entrypoint, port) = match framework {
                Some("fastapi") => (
                    "python:3.13-slim",
                    "src/main.py",
                    "3000",
                ),
                Some("django") => (
                    "python:3.13-slim",
                    "manage.py",        // Django запускается иначе
                    "8000",             // Django default port
                ),
                Some("flask") => (
                    "python:3.13-slim",
                    "src/app.py",
                    "3000",
                ),
                _ => (
                    "python:3.13-slim",
                    "src/main.py",
                    "3000",
                ),
            };

            Some(format!(r#"FROM {base_image}

WORKDIR /app

# Install dependencies
COPY requirements.txt .
RUN pip install --no-cache-dir -r requirements.txt

# Copy application code
COPY . .

# Expose the port
EXPOSE {port}

# Run the application
CMD ["python", "{entrypoint}"]
"#))
        }

        "rust" => {
            let (port, bin_name) = match framework {
                Some("axum") => ("3000", project_name),
                Some("clap") => return None, // CLI не нужен Docker
                _ => ("3000", project_name),
            };

            Some(format!(r#"# Build stage
FROM rust:1.83-slim-bookworm AS builder

WORKDIR /app
COPY . .
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app
COPY --from=builder /app/target/release/{bin_name} .

EXPOSE {port}

CMD ["./{bin_name}"]
"#))
        }

        "typescript" | "javascript" | "node" => {
    let (base_image, needs_build, entrypoint, port, _build_steps) = match framework {
        Some("nextjs") => (
            "node:22-alpine",
            true,
            "node_modules/.bin/next",
            "3000",
            "RUN npm run build\n",  // Next.js запускается через next start
        ),
        Some("nuxt") => (
            "node:22-alpine",
            true,
            ".output/server/index.mjs",
            "3000",
            "COPY . .\nRUN npm ci && npm run build\n",
        ),
        Some("sveltekit") => (
            "node:22-alpine",
            true,
            "build/index.js",
            "3000",
            "COPY . .\nRUN npm ci && npm run build\n",
        ),
        Some("nest") => (
            "node:22-alpine",
            true,
            "dist/main.js",
            "3000",
            "COPY . .\nRUN npm ci && npm run build\n",
        ),
        Some("fastify") | Some("express") => (
            "node:22-alpine",
            false,
            "src/index.js",
            "3000",
            "",
        ),
        _ => (
            "node:22-alpine",
            false,
            "src/index.js",
            "3000",
            "",
        ),
    };

    if needs_build {
        // Для фреймворков, которым нужна сборка
        Some(format!(r#"FROM {base_image}

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

CMD ["node", "{entrypoint}"]
"#))
    } else {
        // Для простых серверов без сборки
        Some(format!(r#"FROM {base_image}

WORKDIR /app

# Install production dependencies only
COPY package*.json ./
RUN npm ci --only=production

# Copy application code
COPY . .

EXPOSE {port}

CMD ["node", "{entrypoint}"]
"#))
    }
}

        "go" => {
            if framework == Some("cobra") {return None}
            Some(format!(r#"# Build stage
FROM golang:1.24-alpine AS builder

WORKDIR /app
COPY go.mod go.sum ./
RUN go mod download
COPY . .
RUN CGO_ENABLED=0 go build -o app ./cmd/main.go

# Runtime stage
FROM alpine:3.21

WORKDIR /app
COPY --from=builder /app/app .

EXPOSE 3000

CMD ["./app"]
"#))
        }
        "csharp" => {
    let (runtime_image, port, project_file) = match framework {
        Some("aspnetcore") => (
            "mcr.microsoft.com/dotnet/aspnet:8.0",
            "EXPOSE 8080",
            format!("{}.csproj", project_name)
        ),
        _ => (
            "mcr.microsoft.com/dotnet/runtime:8.0",
            "",
            format!("{}.csproj", project_name)
        ),
    };

    Some(format!(r#"FROM mcr.microsoft.com/dotnet/sdk:8.0 AS build
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
"#))
},
        "java" => {
        let has_spring = framework == Some("spring-boot");
        if !has_spring {
            return None; // Только Spring Boot поддерживает Docker из коробки
        }
    Some(format!(r#"FROM eclipse-temurin:21-jdk-alpine AS build
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
"#))
},
        "php" => {
    let has_laravel = framework == Some("laravel") || framework == Some("symfony");
    if !has_laravel {
        return None;
    }
    Some(format!(r#"FROM php:8.3-fpm-alpine

RUN docker-php-ext-install pdo pdo_mysql

COPY --from=composer:2 /usr/bin/composer /usr/bin/composer

WORKDIR /app
COPY composer.json composer.lock ./
RUN composer install --no-dev --optimize-autoloader

COPY . .
RUN php artisan config:cache || true

EXPOSE 8000
CMD ["php", "artisan", "serve", "--host=0.0.0.0", "--port=8000"]
"#))
},
        "elixir" => {
    let is_phoenix = framework == Some("phoenix");
    if !is_phoenix {
        return None;
    }
    Some(format!(r#"FROM hexpm/elixir:1.17-erlang-27-alpine AS build
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
"#))
},
        "cpp" => {
        let is_qt = framework == Some("qt");
        if is_qt {
            Some(format!(r#"FROM stateoftheartio/qt6:6.7-gcc-ubuntu-24.04 AS build
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
ENTRYPOINT ["./{project_name}", "-platform", "offscreen"]"#))
        } else {
            Some(format!(r#"FROM ubuntu:24.04 AS build
RUN apt-get update && apt-get install -y \
    build-essential \
    cmake \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .
RUN mkdir build && cd build && \
    cmake -DCMAKE_BUILD_TYPE=Release .. && \
    cmake --build .

FROM ubuntu:24.04 AS final
WORKDIR /app

COPY --from=build /app/build/{project_name} .
ENTRYPOINT ["./{project_name}"]"#))
        }
    },
        "zig" => {
// --fetch подтягивает зависимости из build.zig.zon (нужно для zap).
// Образ 0.14 — минимальная версия для zap v0.4 (minimum_zig_version).
Some(format!(r#"FROM ziglang/zig:0.14.0 AS build
WORKDIR /app

COPY build.zig build.zig.zon ./
COPY src/ ./src/

RUN zig build --fetch -Doptimize=ReleaseFast

FROM scratch
WORKDIR /

COPY --from=build /app/zig-out/bin/{project_name} /{project_name}

ENTRYPOINT ["/{project_name}"]
"#))
    },
        "kotlin" => {
    let has_fw = framework == Some("ktor") || framework == Some("spring-boot");
    if has_fw {
        Some(format!(r#"FROM gradle:8.10-jdk21 AS build
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
"#))
    } else {
        None  // Без фреймворка не генерируем Dockerfile
    }
},
        "swift" => {
    let has_fw = framework == Some("vapor");
    if has_fw {
        Some(format!(r#"FROM swift:6.0-noble AS build
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
"#))
    } else {
        Some(format!(r#"FROM swift:6.0-noble AS build
WORKDIR /build
COPY Package.swift ./
RUN swift package resolve
COPY . .
RUN swift build -c release

FROM swift:6.0-noble-slim AS final
WORKDIR /app
COPY --from=build /build/.build/release/{project_name} ./
ENTRYPOINT ["./{project_name}"]
"#))
    }
},
        "dart" => {
        Some(format!(r#"# Этап сборки
FROM dart:3.5 AS build
WORKDIR /app
COPY pubspec.yaml ./
RUN dart pub get
COPY . .
RUN dart compile exe bin/main.dart -o bin/main

# Этап запуска
FROM scratch
COPY --from=build /app/bin/main /main
ENTRYPOINT ["/main"]
"#))
    },
        "gleam" => {
    Some(format!(r#"FROM ghcr.io/gleam-lang/gleam:v1.4-erlang-alpine AS build
WORKDIR /app
COPY gleam.toml manifest.toml ./
RUN gleam deps download
COPY . .
RUN gleam export erlang-shipment

FROM erlang:27-alpine AS final
WORKDIR /app
COPY --from=build /app/build/erlang-shipment ./
ENTRYPOINT ["/app/entrypoint.sh"]
"#))
},
        _ => None, // Неизвестный язык — не генерируем Dockerfile
    }
}

// ---------------------------------------------------------------------------
// Docker Compose generation
// ---------------------------------------------------------------------------

pub fn generate_docker_compose(services: &[DockerService], project_name: &str, app_port: &str) -> String {
    let mut result = String::new();
    let mut volumes_section = String::new();

    result.push_str("version: '3.8'\n\nservices:\n");

    // App service всегда добавляется
    result.push_str(&format!(r#"  app:
    build: .
    container_name: {project_name}_app
    ports:
      - "{app_port}:{app_port}"
"#));

    // Если есть сервисы — добавляем depends_on
    if !services.is_empty() {
        result.push_str("    depends_on:\n");
        for service in services.iter() {
            result.push_str(&format!("      - {}\n", service.name));
        }
        result.push_str("    environment:\n");
        // Стандартные переменные окружения для подключения к сервисам
        for service in services.iter() {
            match service.name.as_str() {
                "postgres" => result.push_str("      - DATABASE_URL=postgresql://user:password@postgres:5432/dbname\n"),
                "redis" => result.push_str("      - REDIS_URL=redis://redis:6379/0\n"),
                _ => {}
            }
        }
    }
    result.push('\n');

    // Сервисы БД/кешей
    for service in services.iter() {
        result.push_str(&format!("  {}:\n", service.name));
        result.push_str(&format!("    image: {}\n", service.image));
        result.push_str(&format!("    container_name: {}_{}\n", project_name, service.name));

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
                // Извлекаем имя volume для секции volumes в конце файла
                let vol_name = volume.split(':').next().unwrap_or(volume);
                volumes_section.push_str(&format!("\n  {}:", vol_name));
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
         \n"
    );

    for lang in languages {
        match lang.as_str() {
            "rust" => {
                content.push_str("# Rust\n\
                                  target/\n\
                                  **/*.rs.bk\n\
                                  *.pdb\n\
                                  \n");
            }
            "python" => {
                content.push_str("# Python\n\
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
                                  \n");
            }
            "typescript" | "javascript" | "node" => {
                content.push_str("# Node\n\
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
                                  \n");
            }
            "go" => {
                content.push_str("# Go\n\
                                  *.exe\n\
                                  *.exe~\n\
                                  *.dll\n\
                                  *.so\n\
                                  *.dylib\n\
                                  *.test\n\
                                  *.out\n\
                                  go.work\n\
                                  \n");
            }
            "java" => {
                content.push_str("# Java\n\
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
                                  \n");
            }
            "csharp" => {
                content.push_str("# .NET\n\
                                  bin/\n\
                                  obj/\n\
                                  *.user\n\
                                  *.suo\n\
                                  *.cache\n\
                                  *.docstates\n\
                                  packages/\n\
                                  \n");
            }
            "cpp" | "c" => {
                content.push_str("# C/C++\n\
                                  *.o\n\
                                  *.obj\n\
                                  *.exe\n\
                                  *.out\n\
                                  *.app\n\
                                  *.a\n\
                                  *.so\n\
                                  *.dylib\n\
                                  \n");
            }
            "php" => {
                content.push_str("# PHP\n\
                                  vendor/\n\
                                  composer.lock\n\
                                  \n");
            }
            "zig" => {
                content.push_str("# Zig\n\
                                  zig-out/\n\
                                  zig-cache/\n\
                                  \n");
            }
            "swift" => {
                content.push_str("# Swift\n\
                                  .build/\n\
                                  DerivedData/\n\
                                  *.xcodeproj\n\
                                  *.xcworkspace\n\
                                  \n");
            }
            "kotlin" => {
                content.push_str("# Kotlin\n\
                                  .gradle/\n\
                                  build/\n\
                                  .idea/\n\
                                  *.iml\n\
                                  out/\n\
                                  local.properties\n\
                                  \n");
            }
            "elixir" => {
                content.push_str("# Elixir\n\
                                  _build/\n\
                                  deps/\n\
                                  .elixir_ls/\n\
                                  \n");
            }
            "dart" => {
                content.push_str("# Dart\n\
                                  .dart_tool/\n\
                                  .packages\n\
                                  build/\n\
                                  pubspec.lock\n\
                                  \n");
            }
            "gleam" => {
                content.push_str("# Gleam\n\
                                  build/\n\
                                  \n");
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
        "rust" => format!(r#"name: CI

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
"#),

        "python" => format!(r#"name: CI

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
"#),

        "typescript" | "javascript" | "node" => format!(r#"name: CI

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
"#),

        "go" => format!(r#"name: CI

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
          go-version: '1.24'
      - name: Test
        run: go test ./...
      - name: Build
        run: go build -v ./...
"#),

        "java" => format!(r#"name: CI

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
"#),

        _ => format!(r#"name: CI

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
"#),
    }
}

// ---------------------------------------------------------------------------
// README content
// ---------------------------------------------------------------------------

pub fn generate_readme(name: &str, lang: &str, framework: Option<&str>, project_type: &str, tools: &[String]) -> String {
    let fw_str = framework.map(|f| format!(" with {}", f)).unwrap_or_default();
    let tools_str = if tools.is_empty() {
        String::new()
    } else {
        format!("\n\n## Tools & Services\n\n{}", tools.iter()
            .map(|t| format!("- {}", t))
            .collect::<Vec<_>>()
            .join("\n"))
    };

    format!(r#"# {name}

A {project_type} built on {lang}{fw_str}.

## Getting Started

### Prerequisites

- {lang} installed on your system

### Installation

```bash
# Clone the repository
git clone <repo-url>
cd {name}

# Install dependencies
# (instructions depend on the language)
```bash

### Running the application
```bash
# Run the application
# (add specific instructions here)
```bash

###Project Structure
```text
{name}/
├── src/           # Source code
├── tests/         # Test files
└── README.md      # This file
```text

{tools_str}


***Made with StackPilot***"#)

}

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
}"#.to_string(),

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
}"#.to_string(),

        "typescript" | "javascript" | "node" => r#"{
    "typescript.tsdk": "node_modules/typescript/lib",
    "editor.formatOnSave": true,
    "editor.defaultFormatter": "esbenp.prettier-vscode",
    "[typescript]": {
        "editor.defaultFormatter": "esbenp.prettier-vscode"
    }
}"#.to_string(),

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
}"#.to_string(),

        _ => r#"{
    "editor.formatOnSave": true
}"#.to_string(),
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
}"#.to_string(),

        "python" => r#"{
    "recommendations": [
        "ms-python.python",
        "charliermarsh.ruff",
        "ms-python.mypy-type-checker"
    ]
}"#.to_string(),

        "typescript" | "javascript" | "node" => r#"{
    "recommendations": [
        "dbaeumer.vscode-eslint",
        "esbenp.prettier-vscode"
    ]
}"#.to_string(),

        "go" => r#"{
    "recommendations": [
        "golang.go"
    ]
}"#.to_string(),

        "java" => r#"{
    "recommendations": [
        "vscjava.vscode-java-pack"
    ]
}"#.to_string(),

        "csharp" => r#"{
    "recommendations": [
        "ms-dotnettools.csharp"
    ]
}"#.to_string(),

        _ => r#"{
    "recommendations": []
}"#.to_string(),
    }
}