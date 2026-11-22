//! Desktop setup that can be checked without opening a window.

#[cfg(target_os = "linux")]
mod command_install;
pub mod terminal_command;
