//! The `uos` command line.
//!
//! ```text
//! uos run [--isolate] [--profile NAME] [--dry-run] PROGRAM [ARGS...]
//! uos info PROGRAM
//! uos install [--dry-run] PROGRAM
//! uos list
//! uos uninstall NAME
//! uos profile [--new] PROGRAM
//! uos doctor
//! ```

use std::ffi::OsString;
use std::fs;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Parser, Subcommand};
use serde_json::{json, Map, Value};

use crate::detect::{detect, Arch, BinaryInfo, Family};
use crate::error::Result;
use crate::host::Host;
use crate::install::{self, Outcome};
use crate::macos::{self, Extractor};
use crate::paths::Dirs;
use crate::profiles::{self, Profile};
use crate::runners::{self, Plan};

const BINFMT_DIR: &str = "/proc/sys/fs/binfmt_misc";

#[derive(Parser)]
#[command(name = "uos", version, about = "Run Linux, Windows and macOS programs.")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a program with the right compatibility layer
    Run {
        /// Use a Wine prefix dedicated to this program
        #[arg(long)]
        isolate: bool,
        /// Use this profile instead of the detected one
        #[arg(long, value_name = "NAME")]
        profile: Option<String>,
        /// Print the command instead of running it
        #[arg(long)]
        dry_run: bool,
        /// The program, followed by its own arguments (all passed through untouched)
        #[arg(value_name = "PROGRAM [ARGS]", required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<OsString>,
    },
    /// Show format, architecture, profile and runner of a program
    Info { program: PathBuf },
    /// Install a program and add it to the applications menu
    Install {
        /// For installers: print the command instead of running it
        #[arg(long)]
        dry_run: bool,
        program: PathBuf,
    },
    /// List the programs installed with uos
    List,
    /// Remove a program installed with uos
    Uninstall {
        /// Id, name or file shown by `uos list`
        name: String,
    },
    /// Show or create the profile of a program
    Profile {
        /// Create a template profile in ~/.config/uos/profiles
        #[arg(long)]
        new: bool,
        program: PathBuf,
    },
    /// Check which compatibility layers are installed
    Doctor,
}

/// Everything a command needs from the outside world.
pub struct Ctx<'a> {
    pub host: &'a dyn Host,
    pub dirs: &'a Dirs,
    pub extractor: &'a dyn Extractor,
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    /// Replace the process with the planned command (false in tests).
    pub exec: bool,
}

impl Ctx<'_> {
    fn error(&mut self, msg: impl std::fmt::Display) {
        let _ = writeln!(self.err, "uos: {msg}");
    }
}

/// Parse `args` (including the program name) and run the command.
pub fn run(args: impl IntoIterator<Item = OsString>, ctx: &mut Ctx) -> i32 {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            let code = if e.use_stderr() { 2 } else { 0 };
            if e.use_stderr() {
                let _ = write!(ctx.err, "{e}");
            } else {
                let _ = write!(ctx.out, "{e}");
            }
            return code;
        }
    };
    match cli.command {
        Cmd::Run { isolate, profile, dry_run, command } => {
            let (program, args) = command.split_first().expect("clap requires PROGRAM");
            cmd_run(ctx, Path::new(program), args, isolate, profile.as_deref(), dry_run)
        }
        Cmd::Info { program } => cmd_info(ctx, &program),
        Cmd::Install { dry_run, program } => cmd_install(ctx, &program, dry_run),
        Cmd::List => cmd_list(ctx),
        Cmd::Uninstall { name } => cmd_uninstall(ctx, &name),
        Cmd::Profile { new, program } => cmd_profile(ctx, &program, new),
        Cmd::Doctor => cmd_doctor(ctx),
    }
}

/// Detect the format; .dmg images are extracted and replaced by their content.
fn resolve(ctx: &Ctx, program: &Path) -> Result<BinaryInfo> {
    let info = detect(program)?;
    if info.family == Family::MacDmg {
        return Ok(detect(&macos::resolve_dmg(program, ctx.dirs, ctx.extractor)?)?);
    }
    Ok(info)
}

fn execute(ctx: &mut Ctx, plan: &Plan, dry_run: bool) -> i32 {
    if dry_run || !ctx.exec {
        let _ = writeln!(ctx.out, "{}", plan.describe());
        return 0;
    }
    if let Some(prefix) = plan.env.get("WINEPREFIX") {
        if let Err(e) = fs::create_dir_all(prefix) {
            ctx.error(format!("cannot create the Wine prefix {prefix}: {e}"));
            return 1;
        }
    }
    let err = Command::new(&plan.argv[0]).args(&plan.argv[1..]).envs(&plan.env).exec();
    ctx.error(format!("cannot start {}: {err}", plan.argv[0].to_string_lossy()));
    if err.kind() == std::io::ErrorKind::NotFound {
        127
    } else {
        126
    }
}

fn missing(ctx: &mut Ctx, program: &Path) -> Option<i32> {
    if fs::symlink_metadata(program).is_ok() {
        return None;
    }
    ctx.error(format!("file not found: {}", program.display()));
    Some(127)
}

fn cmd_run(
    ctx: &mut Ctx,
    program: &Path,
    args: &[OsString],
    isolate: bool,
    profile: Option<&str>,
    dry_run: bool,
) -> i32 {
    if let Some(code) = missing(ctx, program) {
        return code;
    }
    let planned = (|| {
        let info = resolve(ctx, program)?;
        let profile = match profile {
            Some(name) => Some(profiles::by_name(name, ctx.dirs)?),
            None => profiles::find(&info, ctx.dirs)?,
        };
        runners::plan(&info, args, isolate, profile.as_ref(), ctx.host, ctx.dirs)
    })();
    match planned {
        Ok(plan) => execute(ctx, &plan, dry_run),
        Err(e) => {
            ctx.error(format!("{}: {e}", program.display()));
            126
        }
    }
}

fn cmd_info(ctx: &mut Ctx, program: &Path) -> i32 {
    let info = match detect(program) {
        Ok(info) => info,
        Err(e) => {
            ctx.error(format!("{}: {e}", program.display()));
            return 1;
        }
    };
    let mut out = Map::new();
    out.insert("path".into(), json!(info.path));
    out.insert("family".into(), json!(info.family));
    out.insert("archs".into(), json!(info.archs));
    if let Value::Object(details) = json!(info.details) {
        out.extend(details);
    }
    if let Some(exe) = &info.executable {
        out.insert("executable".into(), json!(exe));
    }
    if info.family == Family::MacDmg {
        out.insert("runner".into(), json!("dmg (extracted, then the .app it contains)"));
    } else {
        let planned = profiles::find(&info, ctx.dirs).and_then(|profile| {
            let source = profile.as_ref().map(|p: &Profile| p.source.clone());
            runners::plan(&info, &[], false, profile.as_ref(), ctx.host, ctx.dirs).map(|p| (source, p))
        });
        match planned {
            Ok((source, plan)) => {
                out.insert("profile".into(), json!(source));
                out.insert("runner".into(), json!(plan.runner));
            }
            Err(e) => {
                out.insert("runner".into(), Value::Null);
                out.insert("problem".into(), json!(e.to_string()));
            }
        }
    }
    let _ = writeln!(ctx.out, "{}", serde_json::to_string_pretty(&Value::Object(out)).unwrap_or_default());
    0
}

fn cmd_install(ctx: &mut Ctx, program: &Path, dry_run: bool) -> i32 {
    if let Some(code) = missing(ctx, program) {
        return code;
    }
    match install::install(program, ctx.host, ctx.dirs, ctx.extractor) {
        Ok(Outcome::Installer(plan)) => {
            let _ = writeln!(ctx.err, "uos: starting the installer ({})", plan.runner);
            execute(ctx, &plan, dry_run)
        }
        Ok(Outcome::MenuEntry(entry)) => {
            let _ = writeln!(ctx.out, "Installed. Menu entry: {}", entry.display());
            0
        }
        Err(e) => {
            ctx.error(format!("{}: {e}", program.display()));
            1
        }
    }
}

fn cmd_list(ctx: &mut Ctx) -> i32 {
    let entries = install::installed(ctx.dirs);
    if entries.is_empty() {
        let _ = writeln!(ctx.out, "No programs installed with uos.");
    }
    for e in entries {
        let _ = writeln!(ctx.out, "{:30} {:30} {}", e.id, e.name, e.target);
    }
    0
}

fn cmd_uninstall(ctx: &mut Ctx, name: &str) -> i32 {
    match install::uninstall(name, ctx.dirs) {
        Ok(removed) => {
            for path in removed {
                let _ = writeln!(ctx.out, "removed {}", path.display());
            }
            0
        }
        Err(e) => {
            ctx.error(e);
            1
        }
    }
}

fn cmd_profile(ctx: &mut Ctx, program: &Path, new: bool) -> i32 {
    if let Some(code) = missing(ctx, program) {
        return code;
    }
    let info = match resolve(ctx, program) {
        Ok(info) => info,
        Err(e) => {
            ctx.error(format!("{}: {e}", program.display()));
            return 1;
        }
    };
    if new {
        let stem = info.path.file_stem().unwrap_or_default().to_string_lossy();
        let path = ctx.dirs.user_profiles().join(format!("{}.toml", runners::safe_name(&stem).to_lowercase()));
        if path.exists() {
            ctx.error(format!("the profile already exists: {}", path.display()));
            return 1;
        }
        let written =
            fs::create_dir_all(ctx.dirs.user_profiles()).and_then(|_| fs::write(&path, profiles::template(&info)));
        if let Err(e) = written {
            ctx.error(format!("{}: {e}", path.display()));
            return 1;
        }
        let _ = writeln!(ctx.out, "{}", path.display());
        return 0;
    }
    match profiles::find(&info, ctx.dirs) {
        Ok(Some(profile)) => {
            let _ = writeln!(ctx.out, "{}", profile.source.display());
            0
        }
        Ok(None) => {
            let _ = writeln!(ctx.out, "No profile matches (create one with: uos profile --new PROGRAM)");
            1
        }
        Err(e) => {
            ctx.error(e);
            1
        }
    }
}

fn cmd_doctor(ctx: &mut Ctx) -> i32 {
    let host_arch = ctx.host.arch();
    let _ = writeln!(ctx.out, "UniOS {} — host architecture: {host_arch}\n", env!("CARGO_PKG_VERSION"));
    let mut layers: Vec<(&str, &[&str])> = vec![
        ("Windows (Wine)", &["wine", "wine64"]),
        ("macOS (Darling)", &["darling"]),
        (".dmg images (7-Zip)", &["7zz", "7z"]),
        ("MS-DOS (DOSBox)", &["dosbox-x", "dosbox-staging", "dosbox"]),
        ("Generic CPU emulator", &["qemu-x86_64", "qemu-aarch64"]),
        ("Java", &["java"]),
    ];
    if host_arch != Arch::X86_64 {
        layers.push(("x86 translator on ARM", &["box64", "FEXInterpreter"]));
    }
    let mut missing = 0;
    let mut row = |out: &mut dyn Write, label: &str, found: Option<PathBuf>, absent: &str| {
        missing += usize::from(found.is_none());
        let mark = if found.is_some() { "OK " } else { "-- " };
        let what = found.map(|p| p.display().to_string()).unwrap_or_else(|| absent.to_string());
        let _ = writeln!(out, "  [{mark}] {label:26} {what}");
    };
    for (label, bins) in layers {
        row(ctx.out, label, ctx.host.which_any(bins), "not installed");
    }
    if host_arch == Arch::Aarch64 {
        row(ctx.out, "x86_64 Wine (for box64)", runners::x86_wine(ctx.host), "not installed (/opt/uos/wine-x86_64)");
    }

    let _ = writeln!(ctx.out);
    match fs::read_dir(BINFMT_DIR) {
        Ok(entries) => {
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
                .filter(|n| n.starts_with("uos-"))
                .collect();
            names.sort();
            if names.is_empty() {
                let _ =
                    writeln!(ctx.out, "  binfmt_misc: no uos-* entries (.exe/Mach-O files won't start with ./program)");
            } else {
                let _ = writeln!(ctx.out, "  binfmt_misc: registered {}", names.join(", "));
            }
        }
        Err(_) => {
            let _ = writeln!(ctx.out, "  binfmt_misc: not mounted");
        }
    }
    let dirs: Vec<String> = ctx.dirs.profile_dirs().iter().map(|d| d.display().to_string()).collect();
    let _ = writeln!(ctx.out, "  profiles: {} in {}", profiles::count(ctx.dirs), dirs.join(", "));
    i32::from(missing > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::macos::SevenZip;
    use crate::testutil::*;

    struct Run {
        code: i32,
        out: String,
        err: String,
    }

    fn uos(t: &Tmp, host: &FakeHost, args: &[&str]) -> Run {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let extractor = SevenZip::from_host(host);
        let mut ctx = Ctx { host, dirs: &t.dirs, extractor: &extractor, out: &mut out, err: &mut err, exec: false };
        let argv = std::iter::once("uos").chain(args.iter().copied()).map(OsString::from);
        let code = run(argv, &mut ctx);
        Run { code, out: String::from_utf8(out).unwrap(), err: String::from_utf8(err).unwrap() }
    }

    fn parse(args: &[&str]) -> Cmd {
        Cli::try_parse_from(std::iter::once("uos").chain(args.iter().copied())).unwrap().command
    }

    #[test]
    fn binfmt_style_invocation_keeps_program_args() {
        // binfmt_misc runs: uos-binfmt <program> <args...> -> uos run -- <program> <args...>
        let Cmd::Run { command, dry_run, .. } = parse(&["run", "--", "/x/app.exe", "-x", "--help"]) else { panic!() };
        assert_eq!(command, ["/x/app.exe", "-x", "--help"].map(OsString::from));
        assert!(!dry_run);
    }

    #[test]
    fn options_after_the_program_belong_to_it() {
        let Cmd::Run { command, dry_run, isolate, .. } = parse(&["run", "--isolate", "app.exe", "--dry-run", "-v"])
        else {
            panic!()
        };
        assert!(isolate);
        assert!(!dry_run);
        assert_eq!(command, ["app.exe", "--dry-run", "-v"].map(OsString::from));
    }

    #[test]
    fn run_requires_a_program() {
        assert!(Cli::try_parse_from(["uos", "run"]).is_err());
        assert!(Cli::try_parse_from(["uos", "run", "--dry-run"]).is_err());
    }

    #[test]
    fn dry_run() {
        let t = Tmp::new();
        let prog = t.write("prog", &elf(0x3E, 2));
        let r = uos(&t, &FakeHost::new(Arch::X86_64, &[]), &["run", "--dry-run", prog.to_str().unwrap(), "a"]);
        assert_eq!(r.code, 0);
        assert!(r.out.starts_with("[native] "), "{}", r.out);
        assert!(r.out.trim_end().ends_with(" a"));
    }

    #[test]
    fn missing_file() {
        let t = Tmp::new();
        let r = uos(&t, &FakeHost::new(Arch::X86_64, &[]), &["run", "/nonexistent/x.exe"]);
        assert_eq!(r.code, 127);
        assert!(r.err.contains("file not found"));
    }

    #[test]
    fn run_error_exit_code() {
        let t = Tmp::new();
        let exe = t.write("x.exe", &pe(0x8664, PeOpts::default()));
        let r = uos(&t, &FakeHost::new(Arch::X86_64, &[]), &["run", exe.to_str().unwrap()]);
        assert_eq!(r.code, 126);
        assert!(r.err.contains("Wine is not installed"), "{}", r.err);
    }

    #[test]
    fn forced_profile() {
        let t = Tmp::new();
        fs::create_dir_all(t.dirs.user_profiles()).unwrap();
        fs::write(t.dirs.user_profiles().join("forced.toml"), "[run]\nargs = [\"-safe\"]\n").unwrap();
        let exe = t.write("x.exe", &pe(0x8664, PeOpts::default()));
        let r = uos(
            &t,
            &FakeHost::new(Arch::X86_64, &["wine"]),
            &["run", "--dry-run", "--profile", "forced", exe.to_str().unwrap()],
        );
        assert!(r.out.contains("profile forced") && r.out.contains("-safe"), "{}", r.out);
    }

    #[test]
    fn new_profile_template_is_found_afterwards() {
        let t = Tmp::new();
        let exe = t.write("My Game.exe", &pe(0x8664, PeOpts::default()));
        let host = FakeHost::new(Arch::X86_64, &[]);
        let r = uos(&t, &host, &["profile", "--new", exe.to_str().unwrap()]);
        assert_eq!(r.code, 0, "{}", r.err);
        assert!(r.out.trim().ends_with("my_game.toml"));
        let r = uos(&t, &host, &["profile", exe.to_str().unwrap()]);
        assert!(r.out.trim().ends_with("my_game.toml"), "{}", r.out);
        let r = uos(&t, &host, &["profile", "--new", exe.to_str().unwrap()]);
        assert_eq!(r.code, 1);
    }

    #[test]
    fn info_reports_runner_and_problem() {
        let t = Tmp::new();
        let exe = t.write("x.exe", &pe(0x8664, PeOpts::default()));
        let r = uos(&t, &FakeHost::new(Arch::X86_64, &["wine"]), &["info", exe.to_str().unwrap()]);
        // The ISO self-test greps for exactly this line
        assert!(r.out.contains("\"runner\": \"wine\""), "{}", r.out);
        assert!(r.out.contains("\"format\": \"PE32+\""));
        let r = uos(&t, &FakeHost::new(Arch::X86_64, &[]), &["info", exe.to_str().unwrap()]);
        assert!(r.out.contains("\"runner\": null") && r.out.contains("\"problem\""), "{}", r.out);
    }

    #[test]
    fn install_list_uninstall() {
        let t = Tmp::new();
        let app = t.make_app("dl", "Calc", "org.example.calc", true);
        let host = FakeHost::new(Arch::X86_64, &[]);
        let r = uos(&t, &host, &["install", app.to_str().unwrap()]);
        assert_eq!(r.code, 0, "{}", r.err);
        assert!(r.out.contains("uos-org-example-calc.desktop"));
        assert!(uos(&t, &host, &["list"]).out.contains("org-example-calc"));
        let r = uos(&t, &host, &["uninstall", "Calc"]);
        assert_eq!(r.out.lines().count(), 3);
        assert!(uos(&t, &host, &["list"]).out.contains("No programs installed"));
    }

    #[test]
    fn install_exe_dry_run() {
        let t = Tmp::new();
        let exe = t.write("setup.exe", &pe(0x14C, PeOpts::default()));
        let r = uos(&t, &FakeHost::new(Arch::X86_64, &["wine"]), &["install", "--dry-run", exe.to_str().unwrap()]);
        assert_eq!(r.code, 0);
        assert!(r.err.contains("starting the installer (wine)"));
        assert!(r.out.starts_with("[wine]"));
    }

    #[test]
    fn doctor_lists_layers() {
        let t = Tmp::new();
        let r = uos(&t, &FakeHost::new(Arch::Aarch64, &["wine", "box64"]), &["doctor"]);
        assert_eq!(r.code, 1);
        assert!(r.out.contains("[OK ] Windows (Wine)"));
        assert!(r.out.contains("[-- ] macOS (Darling)"));
        assert!(r.out.contains("x86 translator on ARM"));
        assert!(r.out.contains("x86_64 Wine (for box64)"));
    }

    #[test]
    fn version_flag() {
        let t = Tmp::new();
        let r = uos(&t, &FakeHost::new(Arch::X86_64, &[]), &["--version"]);
        assert_eq!(r.code, 0);
        assert_eq!(r.out.trim(), format!("uos {}", env!("CARGO_PKG_VERSION")));
    }
}
