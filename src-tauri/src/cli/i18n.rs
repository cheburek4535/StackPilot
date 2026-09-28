/// Resolve i18n translation key to human-readable English text for CLI display.
pub fn tr(key: &str) -> String {
    if !key.starts_with("wizard.") {
        return key.to_string();
    }

    match key {
        // Project types
        "wizard.pt.rest_api.label" => "REST API".into(),
        "wizard.pt.rest_api.desc" => "Backend service with HTTP endpoints".into(),
        "wizard.pt.web_app.label" => "Web Application".into(),
        "wizard.pt.web_app.desc" => "Full-stack web application".into(),
        "wizard.pt.desktop_app.label" => "Desktop Application".into(),
        "wizard.pt.desktop_app.desc" => "Native desktop application (Tauri / Electron / Qt)".into(),
        "wizard.pt.cli_tool.label" => "CLI Tool".into(),
        "wizard.pt.cli_tool.desc" => "Command-line interface tool".into(),
        "wizard.pt.telegram_bot.label" => "Telegram Bot".into(),
        "wizard.pt.telegram_bot.desc" => "Automated Telegram bot".into(),
        "wizard.pt.library.label" => "Library".into(),
        "wizard.pt.library.desc" => "Reusable library or package".into(),
        "wizard.pt.mobile_app.label" => "Mobile App".into(),
        "wizard.pt.mobile_app.desc" => "Cross-platform or native mobile application".into(),
        "wizard.pt.browser_extension.label" => "Browser Extension".into(),
        "wizard.pt.browser_extension.desc" => "Extension for Chrome, Firefox, etc.".into(),
        "wizard.pt.data_pipeline.label" => "Data Pipeline / ETL".into(),
        "wizard.pt.data_pipeline.desc" => "Data processing pipeline or ETL job".into(),
        "wizard.pt.embedded.label" => "Embedded / IoT".into(),
        "wizard.pt.embedded.desc" => "Firmware or IoT application".into(),
        "wizard.pt.custom.label" => "Custom Stack".into(),
        "wizard.pt.custom.desc" => "Build your own recipe from scratch".into(),

        // Common frameworks
        "wizard.fw.django.desc" => "Batteries-included Python web framework".into(),
        "wizard.fw.fastapi.desc" => "Modern async Python web framework with OpenAPI".into(),
        "wizard.fw.flask.desc" => "Lightweight Python web framework".into(),
        "wizard.fw.react.desc" => "UI component library with Vite".into(),
        "wizard.fw.vue.desc" => "Progressive UI framework with Vite".into(),
        "wizard.fw.svelte.desc" => "Compiled UI framework with Vite".into(),
        "wizard.fw.nextjs.desc" => "React full-stack framework with SSR".into(),
        "wizard.fw.nuxt.desc" => "Full-stack Vue framework".into(),
        "wizard.fw.sveltekit.desc" => "Full-stack Svelte framework".into(),
        "wizard.fw.astro.desc" => "Content-driven web framework with islands architecture".into(),
        "wizard.fw.solidjs.desc" => "Full-stack Solid.js framework".into(),
        "wizard.fw.angular.desc" => "Comprehensive TypeScript framework by Google".into(),
        "wizard.fw.vite.desc" => "Fast frontend build tool and dev server".into(),
        "wizard.fw.remix.desc" => "Full-stack React framework focused on web standards".into(),
        "wizard.fw.nest.desc" => "Structured Node.js / TypeScript framework".into(),
        "wizard.fw.express.desc" => "Minimal Node.js web framework".into(),
        "wizard.fw.fastify.desc" => "Fast, low-overhead Node.js web server".into(),
        "wizard.fw.hono.desc" => "Ultrafast lightweight web framework".into(),
        "wizard.fw.adonisjs.desc" => "Full-featured TypeScript web framework".into(),
        "wizard.fw.actix_web.desc" => "Powerful and extremely fast Rust web framework".into(),
        "wizard.fw.axum.desc" => "Ergonomic Rust web framework built on Tokio".into(),
        "wizard.fw.clap.desc" => "Rust CLI argument parser with derive macros".into(),
        "wizard.fw.tauri.desc" => "Lightweight desktop app with web frontend".into(),
        "wizard.fw.gin.desc" => "Fast Go HTTP framework".into(),
        "wizard.fw.fiber.desc" => "Express-inspired Go web framework on Fasthttp".into(),
        "wizard.fw.echo.desc" => "High performance, minimalist HTTP framework for Go".into(),
        "wizard.fw.cobra.desc" => "Powerful Go CLI framework".into(),
        "wizard.fw.spring_boot.desc" => "Production-grade Java web/REST framework".into(),
        "wizard.fw.laravel.desc" => "PHP full-stack web framework".into(),
        "wizard.fw.symfony.desc" => "Modular PHP web framework".into(),
        "wizard.fw.aspnetcore.desc" => "C# .NET high-performance web framework".into(),
        "wizard.fw.blazor.desc" => "Interactive client-side web UI with C#".into(),
        "wizard.fw.ktor.desc" => "Async Kotlin web/REST framework".into(),
        "wizard.fw.jetpack_compose.desc" => "Modern Android UI toolkit in Kotlin".into(),
        "wizard.fw.flutter.desc" => "Dart cross-platform mobile/web/desktop".into(),
        "wizard.fw.react_native.desc" => "JS/TS mobile app framework".into(),
        "wizard.fw.expo.desc" => "Managed React Native development".into(),
        "wizard.fw.electron.desc" => "Cross-platform desktop app with Node.js".into(),
        "wizard.fw.aiogram.desc" => "Async Python Telegram bot framework".into(),
        "wizard.fw.telegraf.desc" => "Modern Node.js Telegram bot framework".into(),
        "wizard.fw.zig_cli.desc" => "Native CLI with Zig standard library".into(),
        "wizard.fw.zap.desc" => "High-performance Zig web framework".into(),
        "wizard.fw.phoenix.desc" => "Elixir web framework for real-time apps".into(),
        "wizard.fw.rails.desc" => "Full-stack web application framework in Ruby".into(),

        // Common tools
        "wizard.tool.postgresql.desc" => "Powerful open source object-relational database".into(),
        "wizard.tool.redis.desc" => "In-memory cache and message broker".into(),
        "wizard.tool.docker.desc" => "Containerization platform & compose setup".into(),
        "wizard.tool.airflow.desc" => "Workflow orchestrator for data pipelines".into(),
        "wizard.tool.terraform.desc" => "Infrastructure as Code tool".into(),
        "wizard.tool.sqlite.desc" => "Lightweight embedded relational database".into(),
        "wizard.tool.mysql.desc" => "Popular open source relational database".into(),
        "wizard.tool.mongodb.desc" => "NoSQL document database".into(),
        "wizard.tool.clickhouse.desc" => "Columnar database for real-time analytics".into(),
        "wizard.tool.kafka.desc" => "Distributed event streaming platform".into(),
        "wizard.tool.pytest.desc" => "Python testing framework".into(),
        "wizard.tool.vitest.desc" => "Fast unit test framework powered by Vite".into(),
        "wizard.tool.prisma.desc" => "Next-generation ORM for Node.js / TypeScript".into(),
        "wizard.tool.drizzle.desc" => "Lightweight TypeScript ORM".into(),
        "wizard.tool.sqlalchemy.desc" => "Python SQL toolkit and ORM".into(),
        "wizard.tool.alembic.desc" => "Database migrations for SQLAlchemy".into(),
        "wizard.tool.ruff.desc" => "Extremely fast Python linter and formatter".into(),
        "wizard.tool.tailwind.desc" => "Utility-first CSS framework".into(),
        "wizard.tool.bun.desc" => "Fast all-in-one JavaScript runtime & package manager".into(),

        // Fallback for unlisted keys
        other => {
            let stripped = other
                .trim_start_matches("wizard.")
                .trim_start_matches("pt.")
                .trim_start_matches("fw.")
                .trim_start_matches("tool.")
                .trim_end_matches(".desc")
                .trim_end_matches(".label");
            stripped.replace('_', " ")
        }
    }
}
