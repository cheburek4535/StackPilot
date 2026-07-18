// Mini IDE Plugin
// ===============
// Requires feature: `plugins` (enabled by default)
//
// Provides:
// - LSP client (completions, diagnostics, hover, go-to-def)
// - Integrated editor enhancements
// - Code actions / refactoring stubs
//
// When disabled: the app still works — file browsing is done
// by the workspace::file_explorer module. Only IDE-specific
// features (LSP, diagnostics, advanced editing) are missing.

pub mod models;
pub mod service;
pub mod commands;

pub use service::IdeService;
