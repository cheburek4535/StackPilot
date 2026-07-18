// Plugins are fully independent submodules.
// Delete this entire folder and the app still compiles
// as long as the `plugins` feature is disabled.
//
// Each plugin lives in its own folder and is a self-contained
// module with its own models, commands, and services.

pub mod mini_ide;
