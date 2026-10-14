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
    //
    // На гибриде NVIDIA + AMD/Intel DMA-BUF через NVIDIA EGL падает с «Failed to
    // create GBM buffer», поэтому там вебвью отдаётся Mesa (iGPU) через glvnd —
    // рендер остаётся аппаратным. Явно заданный пользователем EGL-вендор не трогаем.
    #[cfg(target_os = "linux")]
    {
        let args: Vec<String> = std::env::args().collect();
        let software_rendering = std::env::var_os("STACKPILOT_DISABLE_GPU").is_some()
            || args.iter().any(|arg| arg == "--disable-gpu" || arg == "--software-rendering");

        if software_rendering && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }

        const MESA_EGL: &str = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
        if !software_rendering
            && std::env::var_os("__EGL_VENDOR_LIBRARY_FILENAMES").is_none()
            && hybrid_nvidia_gpu()
            && std::path::Path::new(MESA_EGL).exists()
        {
            std::env::set_var("__EGL_VENDOR_LIBRARY_FILENAMES", MESA_EGL);
        }
    }

    stackpilot_lib::run()
}

/// Есть ли NVIDIA и вместе с ней другая видеокарта (AMD/Intel) — по
/// vendor id DRM-устройств в /sys/class/drm.
#[cfg(target_os = "linux")]
fn hybrid_nvidia_gpu() -> bool {
    let Ok(entries) = std::fs::read_dir("/sys/class/drm") else {
        return false;
    };
    let (mut nvidia, mut other) = (false, false);
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // card0, card1… без коннекторов вида card0-DP-1.
        if !name.starts_with("card") || name.contains('-') {
            continue;
        }
        match std::fs::read_to_string(entry.path().join("device/vendor")) {
            Ok(v) if v.trim() == "0x10de" => nvidia = true,
            Ok(_) => other = true,
            Err(_) => {}
        }
    }
    nvidia && other
}
