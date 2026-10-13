//! UniOS: run Linux, Windows and macOS programs from the same system.
//!
//! The `uos` dispatcher recognises an executable from its magic bytes
//! ([`detect`]), picks the compatibility layer ([`runners`]) and starts it,
//! applying per-program [`profiles`]. [`macos`] handles .dmg images and .app
//! bundles, [`install`] puts programs in the applications menu.

pub mod cli;
pub mod detect;
pub mod error;
pub mod host;
pub mod install;
pub mod macos;
pub mod paths;
pub mod profiles;
pub mod runners;

#[cfg(test)]
mod testutil;
