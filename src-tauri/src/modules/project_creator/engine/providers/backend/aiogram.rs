use crate::modules::project_creator::engine::providers::{write_file, RecipeProvider};
use crate::modules::project_creator::models::{Step, WizardContext};

pub struct AiogramProvider;

impl RecipeProvider for AiogramProvider {
    fn id(&self) -> &'static str {
        "aiogram"
    }

    fn generate_steps(
        &self,
        _project_path: &str,
        project_name: &str,
        _context: &WizardContext,
        _segment: Option<&str>,
    ) -> Vec<Step> {
        vec![
            write_file(
                "aiogram_bot",
                "Create Telegram bot",
                "src/bot.py",
                &format!(
                    r#"import asyncio
import os
from dotenv import load_dotenv
from aiogram import Bot, Dispatcher, types
from aiogram.filters import Command

load_dotenv()

BOT_TOKEN = os.getenv("TELEGRAM_BOT_TOKEN")
if not BOT_TOKEN:
    raise RuntimeError(
        "TELEGRAM_BOT_TOKEN is not set. Copy .env.example to .env and fill in the token."
    )

bot = Bot(token=BOT_TOKEN)
dp = Dispatcher()

@dp.message(Command("start"))
async def cmd_start(message: types.Message):
    await message.answer("Hello from {}!")

async def main():
    await dp.start_polling(bot)

if __name__ == "__main__":
    asyncio.run(main())
"#,
                    project_name
                ),
            ),
        ]
    }
}
