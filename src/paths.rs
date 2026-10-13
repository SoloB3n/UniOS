//! Per-user and system directories used by uos (XDG base directories).

use std::env;
use std::path::{Path, PathBuf};

/// Where uos reads and writes its files. Built from the environment in
/// production and from a temporary directory in tests.
#[derive(Debug, Clone)]
pub struct Dirs {
    pub home: PathBuf,
    /// `$XDG_DATA_HOME`, usually `~/.local/share`.
    pub data: PathBuf,
    /// `$XDG_CONFIG_HOME`, usually `~/.config`.
    pub config: PathBuf,
    /// `$XDG_CACHE_HOME`, usually `~/.cache`.
    pub cache: PathBuf,
    /// System-wide profiles, `/etc/uos/profiles`.
    pub system_profiles: PathBuf,
}

fn xdg(var: &str, home: &Path, fallback: &str) -> PathBuf {
    match env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => home.join(fallback),
    }
}

impl Dirs {
    pub fn from_env() -> Dirs {
        let home = env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"));
        Dirs {
            data: xdg("XDG_DATA_HOME", &home, ".local/share"),
            config: xdg("XDG_CONFIG_HOME", &home, ".config"),
            cache: xdg("XDG_CACHE_HOME", &home, ".cache"),
            system_profiles: PathBuf::from("/etc/uos/profiles"),
            home,
        }
    }

    /// All directories rooted in `root`, for tests.
    pub fn under(root: &Path) -> Dirs {
        let home = root.join("home");
        Dirs {
            data: home.join(".local/share"),
            config: home.join(".config"),
            cache: home.join(".cache"),
            system_profiles: root.join("etc-profiles"),
            home,
        }
    }

    pub fn uos_data(&self) -> PathBuf {
        self.data.join("uos")
    }

    pub fn prefixes(&self) -> PathBuf {
        self.uos_data().join("prefixes")
    }

    pub fn icons(&self) -> PathBuf {
        self.uos_data().join("icons")
    }

    /// Installed applications, macOS style.
    pub fn applications(&self) -> PathBuf {
        self.home.join("Applications")
    }

    /// Desktop menu entries.
    pub fn desktop_entries(&self) -> PathBuf {
        self.data.join("applications")
    }

    pub fn user_profiles(&self) -> PathBuf {
        self.config.join("uos").join("profiles")
    }

    /// Profile search order: the user's directory wins over the system one.
    pub fn profile_dirs(&self) -> [PathBuf; 2] {
        [self.user_profiles(), self.system_profiles.clone()]
    }

    pub fn dmg_cache(&self) -> PathBuf {
        self.cache.join("uos").join("dmg")
    }
}
