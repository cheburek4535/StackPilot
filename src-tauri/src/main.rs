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
    //
    // Но software-путь заметно тормозит. На гибриде NVIDIA + AMD/Intel
    // вместо этого отдаём вебвью Mesa (iGPU) через glvnd: DMA-BUF там
    // работает и рендер остаётся аппаратным.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        const MESA_EGL: &str = "/usr/share/glvnd/egl_vendor.d/50_mesa.json";
        let user_picked_egl = std::env::var_os("__EGL_VENDOR_LIBRARY_FILENAMES").is_some();
        if !user_picked_egl && hybrid_nvidia_gpu() && std::path::Path::new(MESA_EGL).exists() {
            std::env::set_var("__EGL_VENDOR_LIBRARY_FILENAMES", MESA_EGL);
        } else if !user_picked_egl {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
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
