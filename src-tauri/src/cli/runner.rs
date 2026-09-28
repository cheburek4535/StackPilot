use colored::Colorize;
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::modules::project_creator::engine::RecipeEngine;
use crate::modules::project_creator::models::{
    ExecutionEvent, ExecutionEventType, ExecutionPlan, ExecutionResult, OverallStatus, StepStatus,
};

pub async fn execute_with_progress(
    engine: Arc<dyn RecipeEngine>,
    plan: ExecutionPlan,
) -> Result<ExecutionResult, String> {
    let (tx, mut rx) = mpsc::channel::<ExecutionEvent>(64);
    let cancel_flag = Arc::new(AtomicBool::new(false));

    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ")
            .template("{spinner:.green} {msg}")
            .expect("Valid spinner template"),
    );
    spinner.enable_steady_tick(Duration::from_millis(80));

    let total_steps = plan.steps.len();
    spinner.set_message(format!("Starting execution ({} steps)...", total_steps));

    // Spawn execution in background tokio task
    let exec_task = tokio::spawn(async move {
        engine.execute(plan, tx, cancel_flag).await
    });

    // Listen to events and display progress
    while let Some(event) = rx.recv().await {
        match event.event_type {
            ExecutionEventType::StepStarted => {
                let step_num = format!("[{}/{}]", event.step_index + 1, event.total_steps);
                spinner.set_message(format!(
                    "{} {}",
                    step_num.dimmed(),
                    event.step_name.bold()
                ));
            }
            ExecutionEventType::StepProgress { stdout: _, stderr: _ } => {
                // If needed in verbose mode, we can show recent log line
            }
            ExecutionEventType::StepCompleted { status, duration_ms } => {
                let step_num = format!("[{}/{}]", event.step_index + 1, event.total_steps);
                let duration_str = format!("({}ms)", duration_ms).dimmed();

                match status {
                    StepStatus::Success { message } => {
                        let msg_suffix = if message.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", message.dimmed())
                        };
                        spinner.println(format!(
                            "  {} {} {}{}",
                            "✔".green().bold(),
                            step_num.dimmed(),
                            event.step_name,
                            duration_str
                        ));
                        if !msg_suffix.is_empty() {
                            spinner.println(format!("    {}", msg_suffix));
                        }
                    }
                    StepStatus::Skipped { reason } => {
                        spinner.println(format!(
                            "  {} {} {} {}",
                            "○".dimmed(),
                            step_num.dimmed(),
                            event.step_name.dimmed(),
                            format!("(Skipped: {})", reason).dimmed()
                        ));
                    }
                    StepStatus::Failed { error } => {
                        spinner.println(format!(
                            "  {} {} {} {}",
                            "✖".red().bold(),
                            step_num.dimmed(),
                            event.step_name.bold().red(),
                            duration_str
                        ));
                        spinner.println(format!("    {} {}", "Error:".red(), error));
                    }
                    _ => {}
                }
            }
            ExecutionEventType::AllCompleted { .. } => {}
            ExecutionEventType::Error { message } => {
                spinner.println(format!("  {} Critical error: {}", "✖".red().bold(), message));
            }
        }
    }

    spinner.finish_and_clear();

    let result = exec_task
        .await
        .map_err(|e| format!("Task execution join error: {e}"))?;

    match &result.overall {
        OverallStatus::Success => Ok(result),
        OverallStatus::PartialFailure { failed_steps } => Err(format!(
            "Execution completed with partial failures in steps: {}",
            failed_steps.join(", ")
        )),
        OverallStatus::Aborted { last_step, reason } => Err(format!(
            "Execution aborted at step {:?}: {}",
            last_step.as_deref().unwrap_or("unknown"),
            reason
        )),
    }
}
