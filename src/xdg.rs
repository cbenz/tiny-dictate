//! The base directories the tool reads from the environment.
//!
//! One place for the XDG fallbacks, so the runtime directory, the durable sessions and the
//! configuration agree on where they live: a variable when it is set, `$HOME` with its documented
//! suffix otherwise.

use std::path::PathBuf;

/// The directory an environment variable names, or `$HOME` plus its suffix; `None` when neither
/// is set.
fn base(variable: &str, home_suffix: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(home_suffix)))
}

/// The state directory: `$XDG_STATE_HOME`, `$HOME/.local/state` by default. The temporary
/// directory is the last resort, so a machine without a home still has somewhere to write.
pub fn state() -> PathBuf {
    base("XDG_STATE_HOME", ".local/state").unwrap_or_else(std::env::temp_dir)
}

/// The runtime directory: `$XDG_RUNTIME_DIR`, falling back to the state directory.
pub fn runtime() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(state)
}

/// The configuration directory: `$XDG_CONFIG_HOME`, `$HOME/.config` by default; `None` when
/// neither variable names one.
pub fn config() -> Option<PathBuf> {
    base("XDG_CONFIG_HOME", ".config")
}
