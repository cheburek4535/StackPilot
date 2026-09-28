use colored::Colorize;

use crate::cli::args::ListArgs;
use crate::modules::project_creator::wizard::WizardEngine;

pub fn execute(args: ListArgs) -> Result<(), String> {
    let wizard = WizardEngine::new();
    let tree = wizard.get_wizard_tree();

    let cat = args.category.to_lowercase();

    if cat == "all" || cat == "types" || cat == "type" {
        println!("\n{}", "📦 Supported Project Types:".bold().cyan());
        for pt in &tree.project_types {
            let label = crate::cli::i18n::tr(&pt.label);
            let desc = crate::cli::i18n::tr(&pt.description);
            println!(
                "  • {:<20} {:<24} {}",
                pt.id.bold().bright_green(),
                format!("[{}]", label).dimmed(),
                desc.bright_black()
            );
        }
    }

    if cat == "all" || cat == "frameworks" || cat == "fw" {
        println!("\n{}", "🛠  Supported Frameworks:".bold().cyan());
        for fw in &tree.frameworks {
            let side_str = format!("[{}]", fw.side).bright_black();
            let lang_str = if fw.recommended_language.is_empty() {
                String::new()
            } else {
                format!("({})", fw.recommended_language).yellow().to_string()
            };
            let desc = crate::cli::i18n::tr(&fw.description);
            println!(
                "  • {:<18} {:<10} {:<12} {}",
                fw.id.bold().bright_green(),
                side_str,
                lang_str,
                desc.dimmed()
            );
        }
    }

    if cat == "all" || cat == "tools" || cat == "infra" {
        println!("\n{}", "🔧 Supported Tools & Services:".bold().cyan());
        for tool in &tree.tools {
            let cat_str = format!("[{}]", tool.category).bright_blue();
            let desc = crate::cli::i18n::tr(&tool.description);
            println!(
                "  • {:<16} {:<16} {}",
                tool.id.bold().bright_green(),
                cat_str,
                desc.dimmed()
            );
        }
    }

    if cat == "all" || cat == "languages" || cat == "lang" {
        println!("\n{}", "🌐 Supported Languages:".bold().cyan());
        for lang in &tree.languages {
            let cat_str = lang.category.as_deref().unwrap_or("general");
            println!(
                "  • {:<16} {:<14} {}",
                lang.id.bold().bright_green(),
                format!("[{}]", cat_str).bright_magenta(),
                lang.label.dimmed()
            );
        }
    }

    println!();
    Ok(())
}
