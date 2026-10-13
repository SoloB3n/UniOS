use std::io;

use uos::cli::{self, Ctx};
use uos::host::SystemHost;
use uos::macos::SevenZip;
use uos::paths::Dirs;

fn main() {
    // Die quietly when the output is cut short (e.g. `uos doctor | head`)
    // instead of panicking on a broken pipe.
    // SAFETY: restoring the default disposition of SIGPIPE has no preconditions.
    unsafe {
        signal(SIGPIPE, SIG_DFL);
    }

    let host = SystemHost;
    let dirs = Dirs::from_env();
    let extractor = SevenZip::from_host(&host);
    let (mut out, mut err) = (io::stdout(), io::stderr());
    let mut ctx = Ctx { host: &host, dirs: &dirs, extractor: &extractor, out: &mut out, err: &mut err, exec: true };
    std::process::exit(cli::run(std::env::args_os(), &mut ctx));
}

const SIGPIPE: i32 = 13;
const SIG_DFL: usize = 0;

extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}
