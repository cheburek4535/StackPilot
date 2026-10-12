#[tokio::main]
async fn main() {
    let _ = stackpilot_lib::platform::paths::register_stkpil_in_app_paths();
    stackpilot_lib::cli::run().await;
}
