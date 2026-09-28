use colored::Colorize;
use inquire::Confirm;
use std::path::PathBuf;
use std::sync::Arc;

use crate::cli::args::CreateArgs;
use crate::cli::interactive::wizard::run_interactive_wizard;
use crate::cli::runner;
use crate::cli::ui;
use crate::modules::project_creator::engine::preview::build_project_file_preview;
use crate::modules::project_creator::engine::{DefaultRecipeEngine, RecipeEngine};
use crate::modules::project_creator::models::{FileEntry, WizardContext};
use crate::modules::project_creator::normalize::normalize_context;
use crate::modules::project_creator::validate::{current_os, first_error, validate_context};
use crate::modules::project_creator::wizard::WizardEngine;

pub async fn execute(args: CreateArgs) -> Result<(), String> {
    let wizard = WizardEngine::new();
    let tree = wizard.get_wizard_tree();

    let is_interactive = args.interactive
        || (args.backend.is_none()
            && args.frontend.is_none()
            && args.tools.is_empty()
            && !args.yes);

    let (mut context, dest_path, ide_to_open) = if is_interactive {
        let res = run_interactive_wizard(tree, args.path.clone())?;
        (res.context, res.dest_path, res.open_ide)
    } else {
        build_context_from_flags(tree, &args)?
    };

    // 1. Canonical normalization
    let norm_errors = normalize_context(tree, &mut context);
    if !norm_errors.is_empty() {
        for err in norm_errors {
            eprintln!("  {} {}", "✖ ERROR:".bold().red(), err);
        }
        return Err("Project context normalization failed with errors.".into());
    }

    // 2. Canonical stack validation
    let issues = validate_context(tree, &mut context, current_os());
    if let Some(err) = first_error(&issues) {
        ui::print_issues(&issues);
        return Err(format!("Stack validation failed: {err}"));
    }

    // Print any non-blocking warnings
    ui::print_issues(&issues);

    // 3. Print Summary
    ui::print_project_summary(&context, &dest_path, ide_to_open.as_deref(), args.dry_run);

    // 4. Confirmation if interactive and not --yes
    if is_interactive && !args.yes && !args.dry_run {
        let confirmed = Confirm::new("Proceed with project generation?")
            .with_default(true)
            .prompt()
            .unwrap_or(false);

        if !confirmed {
            println!("{}", "Operation cancelled by user.".yellow());
            return Ok(());
        }
    }

    // Ensure parent directories exist
    if !args.dry_run {
        std::fs::create_dir_all(&dest_path)
            .map_err(|e| format!("Failed to create destination directory '{}': {e}", dest_path.display()))?;
    }

    // 5. Plan recipe
    let engine: Arc<dyn RecipeEngine> = Arc::new(DefaultRecipeEngine::new());
    let plan = engine.plan(&context, &dest_path)?;

    // 6. Dry run preview
    if args.dry_run {
        println!("{}", "📋 Planned Recipe Steps:".bold().cyan());
        for (i, step) in plan.steps.iter().enumerate() {
            println!("  {}. {}", i + 1, step.label().bold());
        }

        println!("\n{}", "📁 Generated File Tree Preview:".bold().cyan());
        let preview = build_project_file_preview(&plan);
        for file in &preview.files {
            print_preview_file(file, 1);
        }

        println!("\n{}", "Summary:".bold());
        println!("  Files directly created:  {}", preview.summary.certain_count);
        println!("  Files expected:          {}", preview.summary.expected_count);
        println!("  Directories created:     {}", preview.summary.dir_count);
        println!("\n{}", "✔ Dry run complete. No disk changes were made.".green());
        return Ok(());
    }

    // 7. Execute Recipe
    runner::execute_with_progress(Arc::clone(&engine), plan).await?;

    let project_name = context.project_name.as_deref().unwrap_or("project");
    ui::print_success(project_name, &dest_path);

    // 8. Open in IDE if requested
    if let Some(ref ide) = ide_to_open {
        println!("Opening project in {}...", ide.bold().cyan());
        if let Err(e) = super::open::open_in_ide(&dest_path, ide) {
            eprintln!("  {} {}", "⚠ Warning:".bold().yellow(), e);
        }
    }

    Ok(())
}

fn build_context_from_flags(
    tree: &crate::modules::project_creator::models::WizardTreeData,
    args: &CreateArgs,
) -> Result<(WizardContext, PathBuf, Option<String>), String> {
    let dest_path = args.path.clone().unwrap_or_else(|| PathBuf::from("."));

    let project_name = args.name.clone().unwrap_or_else(|| {
        if dest_path.as_os_str() == "." || dest_path.as_os_str().is_empty() {
            std::env::current_dir()
                .ok()
                .and_then(|cwd| cwd.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_else(|| "my-project".into())
        } else {
            dest_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "my-project".into())
        }
    });

    let mut frameworks = Vec::new();
    let mut backend_languages = Vec::new();
    let mut frontend_languages = Vec::new();

    if let Some(ref b) = args.backend {
        frameworks.push(b.clone());
        if let Some(fw) = tree.frameworks.iter().find(|f| &f.id == b) {
            if !fw.recommended_language.is_empty() {
                backend_languages.push(fw.recommended_language.clone());
            }
        }
    }

    if let Some(ref f) = args.frontend {
        frameworks.push(f.clone());
        if let Some(fw) = tree.frameworks.iter().find(|item| &item.id == f) {
            if !fw.recommended_language.is_empty() {
                frontend_languages.push(fw.recommended_language.clone());
            }
        }
    }

    let project_type = if let Some(ref t) = args.project_type {
        let lower = t.to_lowercase();
        let normalized = match lower.as_str() {
            "fullstack" | "web" | "spa" | "frontend" => "web-app",
            "api" | "backend" | "rest" => "rest-api",
            "cli" => "cli-tool",
            "desktop" => "desktop-app",
            "mobile" => "mobile-app",
            "bot" => "telegram-bot",
            "etl" | "pipeline" => "data-pipeline",
            "ext" | "extension" => "browser-extension",
            "empty" => "custom",
            other => other,
        };
        Some(normalized.to_string())
    } else if args.backend.is_some() && args.frontend.is_some() {
        Some("web-app".into())
    } else if args.backend.is_some() {
        Some("rest-api".into())
    } else if args.frontend.is_some() {
        Some("web-app".into())
    } else {
        Some("custom".into())
    };

    let mut languages = args.languages.clone();
    languages.extend(backend_languages.clone());
    languages.extend(frontend_languages.clone());

    let ide_to_open = args.open.clone();

    let context = WizardContext {
        project_path: Some(dest_path.clone()),
        project_name: Some(project_name),
        is_existing: false,
        project_type,
        languages,
        backend_languages,
        frontend_languages,
        frameworks,
        tools: args.tools.clone(),
        local_infra_tools: vec![],
        features: vec![],
        infrastructure: vec![],
        docker: args.docker,
        testing: false,
        ci: false,
        git_init: args.git,
        vscode_config: ide_to_open.is_some(),
        answers: Default::default(),
        environment_binding_id: None,
        readme_locale: None,
    };

    Ok((context, dest_path, ide_to_open))
}

fn print_preview_file(entry: &FileEntry, indent: usize) {
    let prefix = "  ".repeat(indent);
    if entry.is_dir {
        println!("{}{}/", prefix, entry.name.bold().blue());
        for child in &entry.children {
            print_preview_file(child, indent + 1);
        }
    } else {
        println!("{}{}", prefix, entry.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_context_from_flags_django_react() {
        let wizard = WizardEngine::new();
        let tree = wizard.get_wizard_tree();

        let args = CreateArgs {
            path: Some(PathBuf::from("scratch/my-web-app")),
            name: Some("my-web-app".into()),
            project_type: None,
            backend: Some("django".into()),
            frontend: Some("react".into()),
            tools: vec!["postgresql".into(), "redis".into()],
            languages: vec![],
            open: Some("code".into()),
            git: true,
            docker: false,
            dry_run: true,
            yes: true,
            interactive: false,
        };

        let (mut ctx, dest_path, open_ide) = build_context_from_flags(tree, &args).unwrap();
        assert_eq!(ctx.project_type.as_deref(), Some("web-app"));
        assert_eq!(ctx.project_name.as_deref(), Some("my-web-app"));
        assert_eq!(dest_path, PathBuf::from("scratch/my-web-app"));
        assert_eq!(open_ide.as_deref(), Some("code"));

        let norm_errors = normalize_context(tree, &mut ctx);
        assert!(norm_errors.is_empty(), "Normalization failed: {:?}", norm_errors);

        let issues = validate_context(tree, &mut ctx, current_os());
        assert!(first_error(&issues).is_none(), "Validation failed: {:?}", issues);
    }

    #[test]
    fn test_build_context_from_flags_fastapi() {
        let wizard = WizardEngine::new();
        let tree = wizard.get_wizard_tree();

        let args = CreateArgs {
            path: Some(PathBuf::from("scratch/api-service")),
            name: Some("api-service".into()),
            project_type: None,
            backend: Some("fastapi".into()),
            frontend: None,
            tools: vec!["sqlite".into()],
            languages: vec![],
            open: None,
            git: true,
            docker: false,
            dry_run: true,
            yes: true,
            interactive: false,
        };

        let (mut ctx, _dest_path, _open_ide) = build_context_from_flags(tree, &args).unwrap();
        assert_eq!(ctx.project_type.as_deref(), Some("rest-api"));
        assert_eq!(ctx.backend_languages, vec!["python"]);

        let norm_errors = normalize_context(tree, &mut ctx);
        assert!(norm_errors.is_empty());

        let issues = validate_context(tree, &mut ctx, current_os());
        assert!(first_error(&issues).is_none());
    }
}

