// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // На Linux WebKitGTK использует аппаратное ускорение через DMA-BUF рендерер.
    // Ранее здесь безусловно выставлялся WEBKIT_DISABLE_DMABUF_RENDERER=1, что отключало GPU
    // для всех дистрибутивов и сваливало WebKit в программный рендеринг через CPU (SHM/Cairo),
    // вызывая 600-700 МБ потребления ОЗУ и катастрофические лаги.
    //
    // Теперь по умолчанию аппаратное ускорение включено (Mesa Intel/AMD, современные драйверы NVIDIA).
    // Для систем со старыми сбойными драйверами NVIDIA на Wayland предусмотрен безопасный фоллбек:
    // запуск с ключом `--disable-gpu`, `--software-rendering` либо переменной `STACKPILOT_DISABLE_GPU=1`.
    #[cfg(target_os = "linux")]
    {
        let args: Vec<String> = std::env::args().collect();
        let software_rendering = std::env::var_os("STACKPILOT_DISABLE_GPU").is_some()
            || args.iter().any(|arg| arg == "--disable-gpu" || arg == "--software-rendering");

        if software_rendering && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    stackpilot_lib::run()
}
