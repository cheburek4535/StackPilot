pub mod args;
pub mod commands;
pub mod i18n;
pub mod interactive;
pub mod runner;
pub mod tool_executor;
pub mod ui;

use clap::Parser;
use colored::Colorize;

pub async fn run() {
    // Sync process PATH from system environment so newly installed SDKs/tools are visible
    let _ = crate::modules::toolchain::core::path_service::sync_process_path().await;

    // Parse CLI arguments
    let cli = args::Cli::parse();

    let res = match cli.command {
        Some(args::Commands::Create(args)) => commands::create::execute(args).await,
        Some(args::Commands::List(args)) => commands::list::execute(args),
        Some(args::Commands::Doctor(args)) => commands::doctor::execute(args).await,
        Some(args::Commands::Install(args)) => commands::tool::execute_install(args).await,
        Some(args::Commands::Update(args)) => commands::tool::execute_update(args).await,
        Some(args::Commands::Status(args)) => commands::tool::execute_status(args).await,
        Some(args::Commands::Launch(args)) => commands::launch::execute(args).await,
        Some(args::Commands::Analyze(args)) => commands::analyze::execute(args),
        Some(args::Commands::Open(args)) => commands::open::execute(args),
        Some(args::Commands::Env(args)) => commands::env::execute(args).await,
        Some(args::Commands::Version) => {
            println!("stkpil {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        None => {
            ui::print_banner();
            use clap::CommandFactory;
            let mut cmd = args::Cli::command();
            let _ = cmd.print_help();
            println!();
            Ok(())
        }
    };

    if let Err(e) = res {
        eprintln!("\n{} {}\n", "✖ Error:".bold().red(), e);
        std::process::exit(1);
    }
}
