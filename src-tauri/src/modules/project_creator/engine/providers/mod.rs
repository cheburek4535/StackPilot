use crate::modules::project_creator::models::{Step, WizardContext};
use std::collections::HashMap;

pub mod backend;
pub mod desktop;
pub mod frontend;
pub mod legacy;
pub mod mobile;

/// Трейт для генерации шагов (Steps) конкретного фреймворка или инструмента.
pub trait RecipeProvider: Send + Sync {
    /// Уникальный идентификатор фреймворка (совпадает с wizard_tree.json)
    fn id(&self) -> &'static str;

    /// Генерация шагов для сборки каркаса проекта.
    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step>;
}

/// Реестр всех зарегистрированных провайдеров.
pub struct ProviderRegistry {
    providers: HashMap<&'static str, Box<dyn RecipeProvider>>,
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            providers: HashMap::new(),
        };

        // Регистрация провайдеров (будет пополняться по мере рефакторинга)
        // Backend
        registry.register(Box::new(backend::aiogram::AiogramProvider));
        registry.register(Box::new(backend::aspnetcore::AspNetCoreProvider));
        registry.register(Box::new(backend::axum::AxumProvider));
        registry.register(Box::new(backend::clap::ClapProvider));
        registry.register(Box::new(backend::cobra::CobraProvider));
        registry.register(Box::new(backend::django::DjangoProvider));
        registry.register(Box::new(backend::express::ExpressProvider));
        registry.register(Box::new(backend::fastapi::FastApiProvider));
        registry.register(Box::new(backend::fastify::FastifyProvider));
        registry.register(Box::new(backend::flask::FlaskProvider));
        registry.register(Box::new(backend::gin::GinProvider));
        registry.register(Box::new(backend::ktor::KtorProvider));
        registry.register(Box::new(backend::laravel::LaravelProvider));
        registry.register(Box::new(backend::nest::NestProvider));
        registry.register(Box::new(backend::phoenix::PhoenixProvider));
        registry.register(Box::new(backend::spring_boot::SpringBootProvider));
        registry.register(Box::new(backend::symfony::SymfonyProvider));
        registry.register(Box::new(backend::telegraf::TelegrafProvider));
        registry.register(Box::new(backend::vapor::VaporProvider));
        registry.register(Box::new(backend::zap::ZapProvider));
        registry.register(Box::new(backend::zig_cli::ZigCliProvider));
 
        // Frontend
        registry.register(Box::new(frontend::nextjs::NextjsProvider));
        registry.register(Box::new(frontend::nuxt::NuxtProvider));
        registry.register(Box::new(frontend::plasmo::PlasmoProvider));
        registry.register(Box::new(frontend::react::ReactProvider));
        registry.register(Box::new(frontend::solidjs::SolidjsProvider));
        registry.register(Box::new(frontend::svelte::SvelteProvider));
        registry.register(Box::new(frontend::sveltekit::SveltekitProvider));
        registry.register(Box::new(frontend::vue::VueProvider));

        // Mobile
        registry.register(Box::new(mobile::android::AndroidProvider));
        registry.register(Box::new(mobile::expo::ExpoProvider));
        registry.register(Box::new(mobile::flutter::FlutterProvider));
        registry.register(Box::new(mobile::jetpack_compose::JetpackComposeProvider));
        registry.register(Box::new(mobile::react_native::ReactNativeProvider));

        // Desktop
        registry.register(Box::new(desktop::electron::ElectronProvider));
        registry.register(Box::new(desktop::maui::MauiProvider));
        registry.register(Box::new(desktop::qt::QtProvider));
        registry.register(Box::new(desktop::qt::QtQmlProvider));
        registry.register(Box::new(desktop::qt::QtWidgetsProvider));
        registry.register(Box::new(desktop::qt::QtWebengineProvider));
        registry.register(Box::new(desktop::qt::QtKirigamiProvider));
        registry.register(Box::new(desktop::swiftui::SwiftuiProvider));
        registry.register(Box::new(desktop::tauri::TauriProvider));

        // New Backend
        registry.register(Box::new(backend::rails::RailsProvider));
        registry.register(Box::new(backend::hono::HonoProvider));
        registry.register(Box::new(backend::actix_web::ActixWebProvider));
        registry.register(Box::new(backend::echo::EchoProvider));
        registry.register(Box::new(backend::adonisjs::AdonisjsProvider));
        registry.register(Box::new(backend::blazor::BlazorProvider));
        registry.register(Box::new(backend::fiber::FiberProvider));

        // New Frontend
        registry.register(Box::new(frontend::angular::AngularProvider));
        registry.register(Box::new(frontend::vite::ViteProvider));
        registry.register(Box::new(frontend::astro::AstroProvider));
        registry.register(Box::new(frontend::remix::RemixProvider));

        registry
    }

    pub fn register(&mut self, provider: Box<dyn RecipeProvider>) {
        self.providers.insert(provider.id(), provider);
    }

    pub fn get(&self, id: &str) -> Option<&dyn RecipeProvider> {
        self.providers.get(id).map(|p| p.as_ref())
    }
}

// =========================================================================
// Helpers for Step generation
// =========================================================================

use crate::modules::project_creator::models::{ErrorMode, InteractiveEntry};

pub fn cmd(
    id: &str,
    label: &str,
    desc: &str,
    command: &str,
    args: Vec<&str>,
    project_path: &str,
) -> Step {
    Step::Command {
        id: id.to_string(),
        label: label.to_string(),
        description: desc.to_string(),
        command: command.to_string(),
        args: args.into_iter().map(String::from).collect(),
        working_dir: Some(project_path.to_string()),
        env: None,
        timeout_secs: Some(300),
        condition: None,
        on_error: ErrorMode::Skip,
        interactive: vec![],
    }
}

pub fn cmd_i(
    id: &str,
    label: &str,
    desc: &str,
    command: &str,
    args: Vec<&str>,
    interactive: Vec<InteractiveEntry>,
    project_path: &str,
) -> Step {
    Step::Command {
        id: id.to_string(),
        label: label.to_string(),
        description: desc.to_string(),
        command: command.to_string(),
        args: args.into_iter().map(String::from).collect(),
        working_dir: Some(project_path.to_string()),
        env: None,
        timeout_secs: Some(300),
        condition: None,
        on_error: ErrorMode::Skip,
        interactive,
    }
}

pub fn write_file(id: &str, label: &str, path: &str, content: &str) -> Step {
    Step::WriteFile {
        id: id.to_string(),
        label: label.to_string(),
        description: format!("Create {}", path),
        path: path.to_string(),
        content: content.to_string(),
        overwrite: false,
        policy: None,
        condition: None,
        on_error: ErrorMode::Skip,
    }
}
