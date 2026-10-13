//! Error type shared by every module.

use std::fmt;
use std::io;

#[derive(Debug)]
pub enum Error {
    /// The program cannot be started on this system.
    Runner(String),
    /// A profile file is missing or invalid.
    Profile(String),
    /// A macOS container (.dmg, .app) could not be handled.
    Mac(String),
    /// A program could not be installed or uninstalled.
    Install(String),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Runner(m) | Error::Profile(m) | Error::Mac(m) | Error::Install(m) => f.write_str(m),
            Error::Io(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
