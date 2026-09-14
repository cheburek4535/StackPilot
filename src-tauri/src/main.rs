// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // DMA-BUF рендерер WebKitGTK не может выделить GBM-буфер на некоторых
    // связках GPU-драйверов на Linux (замечено на гибридах с проприетарным
    // NVIDIA + AMD iGPU) — вебвью падает при старте с «Failed to create
    // GBM buffer» или протокольной ошибкой Wayland. Отключение переключает
    // на software/EGL-путь, который работает везде; тем, кому нужен
    // аппаратный DMA-BUF, достаточно выставить переменную самостоятельно
    // перед запуском — эта проверка её не тронет.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    stackpilot_lib::run()
}
