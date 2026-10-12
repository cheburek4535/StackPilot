use crate::modules::project_creator::engine::preflight;
use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{ErrorMode, Step, StepCondition, WizardContext};

pub struct TelegrafProvider;

impl RecipeProvider for TelegrafProvider {
    fn id(&self) -> &'static str {
        "telegraf"
    }

    fn generate_steps(
        &self,
        project_path: &str,
        project_name: &str,
        context: &WizardContext,
        segment: Option<&str>,
    ) -> Vec<Step> {
        let mut steps = vec![write_file(
            "telegraf_bot",
            "Create Telegram bot",
            "src/bot.js",
            &format!(
                r#"require('dotenv').config();
const {{ Telegraf }} = require('telegraf');

const token = process.env.TELEGRAM_BOT_TOKEN;
if (!token) {{
    console.error('TELEGRAM_BOT_TOKEN is not set. Copy .env.example to .env and fill in the token.');
    process.exit(1);
}}

const bot = new Telegraf(token);

bot.start((ctx) => ctx.reply('Hello from {}!'));

bot.launch();
process.once('SIGINT', () => bot.stop('SIGINT'));
process.once('SIGTERM', () => bot.stop('SIGTERM'));
"#,
                project_name
            ),
        )];

        if context.frameworks.iter().any(|f| f == "nest") {
            let pkg_path = match segment {
                Some(dir) => format!("{}/package.json", dir),
                None => "package.json".to_string(),
            };
            steps.push(Step::Command {
                id: "telegraf_pkg_patch".into(),
                label: "Add Telegraf to NestJS dependencies".into(),
                description: "Patch package.json created by the NestJS scaffold to add the telegraf dependency (NestJS owns package.json)".into(),
                command: "node".into(),
                args: vec![
                    "-e".into(),
                    "const fs=require('fs');const p='package.json';const j=JSON.parse(fs.readFileSync(p,'utf8'));j.dependencies=j.dependencies||{};j.dependencies['telegraf']='^4.16.3';j.dependencies['dotenv']='^16.4.5';fs.writeFileSync(p,JSON.stringify(j,null,2)+'\\n')".into(),
                ],
                working_dir: Some(project_path.to_string()),
                env: None,
                timeout_secs: Some(30),
                condition: Some(StepCondition::FileExists { path: pkg_path.clone() }),
                on_error: ErrorMode::Skip,
                interactive: vec![],
            });
            steps.push(preflight::package_json_check_step(
                "telegraf_pkg_check",
                "Validate Telegraf dependency",
                &pkg_path,
                &["telegraf"],
            ));
        } else {
            steps.push(write_file(
                "telegraf_package",
                "Telegraf package.json",
                "package.json",
                &format!(
                    r#"{{
  "name": "{}",
  "version": "1.0.0",
  "main": "src/bot.js",
  "dependencies": {{
    "telegraf": "^4.16.3",
    "dotenv": "^16.4.5"
  }}
}}
"#,
                    project_name
                ),
            ));
            let pkg_path = match segment {
                Some(dir) => format!("{}/package.json", dir),
                None => "package.json".to_string(),
            };
            steps.push(preflight::package_json_check_step(
                "telegraf_pkg_check",
                "Validate Telegraf package.json",
                &pkg_path,
                &["telegraf"],
            ));
        }

        steps
    }
}
