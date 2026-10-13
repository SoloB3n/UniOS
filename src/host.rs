//! Access to the host system: CPU architecture, installed tools, environment.
//!
//! Everything that depends on the machine goes through [`Host`], so planning
//! logic can be tested against a fake host.

use std::env;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::detect::Arch;

pub trait Host {
    fn arch(&self) -> Arch;
    /// Full path of an executable found in `$PATH`.
    fn which(&self, name: &str) -> Option<PathBuf>;
    fn var(&self, key: &str) -> Option<String>;
    fn is_executable(&self, path: &Path) -> bool {
        is_executable(path)
    }

    /// The first of `names` found in `$PATH`.
    fn which_any(&self, names: &[&str]) -> Option<PathBuf> {
        names.iter().find_map(|n| self.which(n))
    }
}

pub fn is_executable(path: &Path) -> bool {
    path.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

/// The real machine uos is running on.
pub struct SystemHost;

impl Host for SystemHost {
    fn arch(&self) -> Arch {
        Arch::from_name(env::consts::ARCH)
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        if name.contains('/') {
            let p = PathBuf::from(name);
            return is_executable(&p).then_some(p);
        }
        env::split_paths(&env::var_os("PATH")?).map(|dir| dir.join(name)).find(|p| is_executable(p))
    }

    fn var(&self, key: &str) -> Option<String> {
        env::var(key).ok().filter(|v| !v.is_empty())
    }
}
