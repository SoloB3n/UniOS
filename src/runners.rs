//! Choice of the compatibility layer and construction of the command to run.
//!
//! Each binary family is handed to a "runner":
//!
//! * ELF       -> native (with a CPU emulator if the architecture differs)
//! * PE / MSI  -> Wine
//! * DOS       -> DOSBox
//! * Mach-O    -> Darling (.pkg -> Darling's installer)
//! * script    -> native (the kernel picks the interpreter)
//!
//! .dmg images are containers: resolve them first with
//! [`crate::macos::resolve_dmg`], which returns the .app or .pkg to run.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::detect::{Arch, BinaryInfo, Family};
use crate::error::{Error, Result};
use crate::host::Host;
use crate::paths::Dirs;
use crate::profiles::Profile;

/// x86_64 Wine build, run under box64/FEX on ARM hosts.
pub const X86_WINE_CANDIDATES: [&str; 2] = ["/opt/uos/wine-x86_64/bin/wine", "/opt/uos/wine-x86_64/bin/wine64"];

#[derive(Debug, Clone)]
pub struct Plan {
    pub runner: &'static str,
    pub argv: Vec<OsString>,
    pub env: BTreeMap<String, String>,
    /// Name of the profile that was applied, if any.
    pub profile: Option<String>,
}

impl Plan {
    /// Human-readable form, as printed by `--dry-run`.
    pub fn describe(&self) -> String {
        let tag = match &self.profile {
            Some(p) => format!("{}, profile {p}", self.runner),
            None => self.runner.to_string(),
        };
        let mut parts: Vec<String> = self.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        parts.extend(self.argv.iter().map(|a| a.to_string_lossy().into_owned()));
        format!("[{tag}] {}", parts.join(" "))
    }
}

/// User-space emulators for binaries of another CPU architecture, in order
/// of preference (dynamic translators are much faster than qemu).
fn cpu_emulators(guest: &Arch, host: &Arch) -> &'static [&'static str] {
    match (guest, host) {
        (Arch::X86_64, Arch::Aarch64) => &["box64", "FEXInterpreter", "qemu-x86_64"],
        (Arch::I386, Arch::Aarch64) => &["box86", "FEXInterpreter", "qemu-i386"],
        (Arch::Aarch64, Arch::X86_64) => &["qemu-aarch64"],
        (Arch::Arm, Arch::X86_64) => &["qemu-arm"],
        (Arch::Riscv64, Arch::X86_64 | Arch::Aarch64) => &["qemu-riscv64"],
        _ => &[],
    }
}

fn native_compatible(guest: &Arch, host: &Arch) -> bool {
    guest == host || (*host == Arch::X86_64 && *guest == Arch::I386)
}

pub fn cpu_emulator(guest: &Arch, host: &dyn Host, prefer: Option<&str>) -> Option<PathBuf> {
    let mut names: Vec<&str> = prefer.into_iter().collect();
    names.extend_from_slice(cpu_emulators(guest, &host.arch()));
    host.which_any(&names)
}

/// Make `name` safe to use as a directory name.
pub fn safe_name(name: &str) -> String {
    let replaced: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || "._+-".contains(c) { c } else { '_' }).collect();
    let trimmed = replaced.trim_matches(|c| c == '.' || c == '_');
    if trimmed.is_empty() {
        "app".to_string()
    } else {
        trimmed.to_string()
    }
}

fn file_stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn wine_prefix(info: &BinaryInfo, isolate: bool, profile: Option<&Profile>, dirs: &Dirs) -> PathBuf {
    let name = match profile.and_then(|p| p.run.prefix.clone()) {
        Some(prefix) => prefix,
        None if isolate || profile.is_some_and(|p| p.run.isolate) => file_stem(&info.path),
        None => "default".to_string(),
    };
    dirs.prefixes().join(safe_name(&name))
}

/// Darling mounts the host filesystem under /Volumes/SystemRoot.
pub fn darling_path(host_path: &Path) -> PathBuf {
    let abs = std::path::absolute(host_path).unwrap_or_else(|_| host_path.to_path_buf());
    let mut p = OsString::from("/Volumes/SystemRoot");
    p.push(abs.as_os_str());
    PathBuf::from(p)
}

pub fn x86_wine(host: &dyn Host) -> Option<PathBuf> {
    host.var("UOS_X86_WINE")
        .into_iter()
        .chain(X86_WINE_CANDIDATES.iter().map(|s| s.to_string()))
        .map(PathBuf::from)
        .find(|p| host.is_executable(p))
}

/// Base Wine command, possibly preceded by the CPU translator.
fn wine_command(info: &BinaryInfo, host: &dyn Host, profile: Option<&Profile>) -> Result<Vec<OsString>> {
    let mut wine = None;
    if let Some(p) = profile {
        if let Some(custom) = &p.run.wine {
            if !host.is_executable(custom) {
                return Err(Error::Runner(format!(
                    "profile {} sets wine={}, which does not exist",
                    p.name,
                    custom.display()
                )));
            }
            wine = Some(custom.clone());
        }
    }

    let guest_x86 = info.family != Family::Pe || matches!(info.arch(), Some(Arch::I386 | Arch::X86_64));
    if host.arch() == Arch::Aarch64 && guest_x86 {
        // Native ARM Wine only runs ARM64 Windows programs: x86 ones need an
        // x86_64 Wine running under box64/FEX.
        let wine = wine.or_else(|| x86_wine(host)).ok_or_else(|| {
            Error::Runner(
                "x86 Windows program on ARM: needs an x86_64 Wine in /opt/uos/wine-x86_64 \
                 (or in the UOS_X86_WINE variable)"
                    .into(),
            )
        })?;
        let emu = cpu_emulator(&Arch::X86_64, host, profile.and_then(|p| p.run.emulator.as_deref()))
            .ok_or_else(|| Error::Runner("x86 Windows program on ARM: needs box64 or FEX".into()))?;
        return Ok(vec![emu.into(), wine.into()]);
    }

    let wine = wine
        .or_else(|| host.which_any(&["wine", "wine64"]))
        .ok_or_else(|| Error::Runner("Wine is not installed: cannot run Windows programs".into()))?;
    Ok(vec![wine.into()])
}

pub fn plan(
    info: &BinaryInfo,
    user_args: &[OsString],
    isolate: bool,
    profile: Option<&Profile>,
    host: &dyn Host,
    dirs: &Dirs,
) -> Result<Plan> {
    let host_arch = host.arch();
    let mut env: BTreeMap<String, String> = profile.map(|p| p.run.env.clone()).unwrap_or_default();
    let mut args: Vec<OsString> = profile.map(|p| p.run.args.iter().map(OsString::from).collect()).unwrap_or_default();
    args.extend_from_slice(user_args);
    let path: OsString = std::path::absolute(&info.path).unwrap_or_else(|_| info.path.clone()).into();
    let profile_name = profile.map(|p| p.name.clone());
    let make = |runner: &'static str, argv: Vec<OsString>, env: BTreeMap<String, String>| Plan {
        runner,
        argv,
        env,
        profile: profile_name.clone(),
    };
    let with_args = |mut argv: Vec<OsString>| {
        argv.extend(args.iter().cloned());
        argv
    };

    match info.family {
        Family::Elf | Family::Script => {
            if let (Family::Elf, Some(arch)) = (info.family, info.arch()) {
                if !native_compatible(arch, &host_arch) {
                    let emu =
                        cpu_emulator(arch, host, profile.and_then(|p| p.run.emulator.as_deref())).ok_or_else(|| {
                            Error::Runner(format!(
                                "Linux {arch} binary on a {host_arch} host: no CPU emulator installed \
                                 (try box64, FEX or qemu-user)"
                            ))
                        })?;
                    return Ok(make("native+emu", with_args(vec![emu.into(), path]), env));
                }
            }
            Ok(make("native", with_args(vec![path]), env))
        }

        Family::Pe | Family::Msi | Family::WinBatch => {
            if info.family == Family::Pe && info.details.dll == Some(true) {
                return Err(Error::Runner("this is a Windows DLL, not a runnable program".into()));
            }
            let mut argv = wine_command(info, host, profile)?;
            env.insert("WINEPREFIX".into(), wine_prefix(info, isolate, profile, dirs).to_string_lossy().into_owned());
            if let Some(overrides) = profile.and_then(Profile::wine_dll_overrides) {
                env.insert("WINEDLLOVERRIDES".into(), overrides);
            }
            match info.family {
                Family::Msi => argv.extend(["msiexec".into(), "/i".into(), path]),
                Family::WinBatch => argv.extend(["cmd".into(), "/c".into(), path]),
                _ => argv.push(path),
            }
            Ok(make("wine", with_args(argv), env))
        }

        Family::Dos => {
            let dosbox = host
                .which_any(&["dosbox-x", "dosbox-staging", "dosbox"])
                .ok_or_else(|| Error::Runner("DOSBox is not installed: cannot run MS-DOS programs".into()))?;
            Ok(make("dosbox", with_args(vec![dosbox.into(), path, "-exit".into()]), env))
        }

        Family::MacPkg => {
            let darling = host
                .which("darling")
                .ok_or_else(|| Error::Runner("Darling is not installed: cannot install .pkg packages".into()))?;
            let argv = vec![
                darling.into(),
                "shell".into(),
                "installer".into(),
                "-pkg".into(),
                darling_path(&info.path).into(),
                "-target".into(),
                "/".into(),
            ];
            Ok(make("darling-installer", argv, env))
        }

        Family::Macho | Family::MachoFat | Family::MacApp => {
            let target = if info.family == Family::MacApp {
                info.executable.clone().ok_or_else(|| Error::Runner(".app bundle without an executable".into()))?
            } else {
                info.path.clone()
            };
            if host_arch == Arch::X86_64
                && !info.archs.is_empty()
                && !info.archs.iter().any(|a| matches!(a, Arch::X86_64 | Arch::I386))
            {
                let mut names: Vec<&str> = info.archs.iter().map(Arch::name).collect();
                names.sort();
                names.dedup();
                return Err(Error::Runner(format!(
                    "the macOS program only contains {} code: Darling needs an x86_64 slice",
                    names.join(", ")
                )));
            }
            let darling = host
                .which("darling")
                .ok_or_else(|| Error::Runner("Darling is not installed: cannot run macOS programs".into()))?;
            Ok(make("darling", with_args(vec![darling.into(), "shell".into(), darling_path(&target).into()]), env))
        }

        Family::MacDmg => {
            Err(Error::Runner(".dmg disk image: it must be extracted first (uos run does this by itself)".into()))
        }

        Family::JavaClass => {
            let java = host.which("java").ok_or_else(|| Error::Runner("Java is not installed".into()))?;
            let dir = Path::new(&path).parent().map(Path::to_path_buf).unwrap_or_default();
            let class = file_stem(&info.path);
            Ok(make("java", with_args(vec![java.into(), "-cp".into(), dir.into(), class.into()]), env))
        }

        Family::Unknown => Err(Error::Runner("unrecognized format".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::detect;
    use crate::testutil::*;

    fn plan_for(t: &Tmp, name: &str, data: &[u8], host: &FakeHost, isolate: bool) -> Result<Plan> {
        let info = detect(&t.write(name, data)).unwrap();
        plan(&info, &["--flag".into()], isolate, None, host, &t.dirs)
    }

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn native_elf() {
        let t = Tmp::new();
        let p = plan_for(&t, "prog", &elf(0x3E, 2), &FakeHost::new(Arch::X86_64, &[]), false).unwrap();
        assert_eq!(p.runner, "native");
        assert_eq!(p.argv[1..], os(&["--flag"]));
    }

    #[test]
    fn i386_elf_runs_natively_on_x86_64() {
        let t = Tmp::new();
        let p = plan_for(&t, "prog", &elf(0x03, 1), &FakeHost::new(Arch::X86_64, &[]), false).unwrap();
        assert_eq!(p.runner, "native");
    }

    #[test]
    fn x86_elf_on_arm_prefers_box64() {
        let t = Tmp::new();
        let host = FakeHost::new(Arch::Aarch64, &["qemu-x86_64", "box64"]);
        let p = plan_for(&t, "prog", &elf(0x3E, 2), &host, false).unwrap();
        assert_eq!((p.runner, p.argv[0].to_str().unwrap()), ("native+emu", "/usr/bin/box64"));
    }

    #[test]
    fn foreign_elf_without_emulator() {
        let t = Tmp::new();
        let err = plan_for(&t, "prog", &elf(0xF3, 2), &FakeHost::new(Arch::X86_64, &[]), false).unwrap_err();
        assert!(err.to_string().contains("riscv64"), "{err}");
    }

    #[test]
    fn windows_exe_uses_wine_with_prefix() {
        let t = Tmp::new();
        let host = FakeHost::new(Arch::X86_64, &["wine"]);
        let p = plan_for(&t, "app.exe", &pe(0x8664, PeOpts::default()), &host, false).unwrap();
        assert_eq!(p.runner, "wine");
        assert_eq!(p.argv[0], "/usr/bin/wine");
        assert!(p.env["WINEPREFIX"].ends_with("prefixes/default"));
    }

    #[test]
    fn isolated_prefix() {
        let t = Tmp::new();
        let host = FakeHost::new(Arch::X86_64, &["wine"]);
        let p = plan_for(&t, "My Game.exe", &pe(0x8664, PeOpts::default()), &host, true).unwrap();
        assert!(p.env["WINEPREFIX"].ends_with("prefixes/My_Game"), "{}", p.env["WINEPREFIX"]);
    }

    #[test]
    fn windows_x86_exe_on_arm_uses_x86_wine_under_box64() {
        let t = Tmp::new();
        let wine = t.write_executable("opt/wine-x86_64/bin/wine", b"#!/bin/sh\n");
        let host = FakeHost::new(Arch::Aarch64, &["wine", "box64"]).with_var("UOS_X86_WINE", wine.to_str().unwrap());
        let p = plan_for(&t, "app.exe", &pe(0x8664, PeOpts::default()), &host, false).unwrap();
        assert_eq!(p.argv[..2], [OsString::from("/usr/bin/box64"), wine.into()]);
    }

    #[test]
    fn windows_x86_exe_on_arm_without_x86_wine() {
        let t = Tmp::new();
        let host = FakeHost::new(Arch::Aarch64, &["wine", "box64"]);
        let err = plan_for(&t, "app.exe", &pe(0x8664, PeOpts::default()), &host, false).unwrap_err();
        assert!(err.to_string().contains("x86_64 Wine"), "{err}");
    }

    #[test]
    fn windows_arm64_exe_on_arm_uses_native_wine() {
        let t = Tmp::new();
        let host = FakeHost::new(Arch::Aarch64, &["wine"]);
        let p = plan_for(&t, "app.exe", &pe(0xAA64, PeOpts::default()), &host, false).unwrap();
        assert_eq!(p.argv[0], "/usr/bin/wine");
    }

    #[test]
    fn msi() {
        let t = Tmp::new();
        let mut data = vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
        data.resize(16, 0);
        let p = plan_for(&t, "setup.msi", &data, &FakeHost::new(Arch::X86_64, &["wine"]), false).unwrap();
        assert_eq!(p.argv[1..3], os(&["msiexec", "/i"]));
    }

    #[test]
    fn batch() {
        let t = Tmp::new();
        let p = plan_for(&t, "go.bat", b"@echo off\r\n", &FakeHost::new(Arch::X86_64, &["wine"]), false).unwrap();
        assert_eq!(p.argv[1..3], os(&["cmd", "/c"]));
    }

    #[test]
    fn dll_is_rejected() {
        let t = Tmp::new();
        let opts = PeOpts { dll: true, ..PeOpts::default() };
        let err = plan_for(&t, "x.dll", &pe(0x8664, opts), &FakeHost::new(Arch::X86_64, &["wine"]), false).unwrap_err();
        assert!(err.to_string().contains("DLL"));
    }

    #[test]
    fn missing_wine() {
        let t = Tmp::new();
        let err = plan_for(&t, "app.exe", &pe(0x8664, PeOpts::default()), &FakeHost::new(Arch::X86_64, &[]), false)
            .unwrap_err();
        assert!(err.to_string().contains("Wine"));
    }

    #[test]
    fn macho_uses_darling_with_translated_path() {
        let t = Tmp::new();
        let p = plan_for(&t, "prog", &macho(0x0100_0007), &FakeHost::new(Arch::X86_64, &["darling"]), false).unwrap();
        assert_eq!(p.runner, "darling");
        assert_eq!(p.argv[1], "shell");
        assert!(p.argv[2].to_str().unwrap().starts_with("/Volumes/SystemRoot/"));
    }

    #[test]
    fn app_bundle_runs_inner_executable() {
        let t = Tmp::new();
        let info = detect(&t.make_app("", "Hello", "com.example.hello", false)).unwrap();
        let p = plan(&info, &[], false, None, &FakeHost::new(Arch::X86_64, &["darling"]), &t.dirs).unwrap();
        assert!(p.argv[2].to_str().unwrap().ends_with("Hello.app/Contents/MacOS/Hello"));
    }

    #[test]
    fn arm64_only_macho_rejected_on_x86_host() {
        let t = Tmp::new();
        let err =
            plan_for(&t, "prog", &macho(0x0100_000C), &FakeHost::new(Arch::X86_64, &["darling"]), false).unwrap_err();
        assert!(err.to_string().contains("x86_64"));
    }

    #[test]
    fn universal_macho_accepted_on_x86_host() {
        let t = Tmp::new();
        let host = FakeHost::new(Arch::X86_64, &["darling"]);
        let p = plan_for(&t, "prog", &fat(&[0x0100_000C, 0x0100_0007]), &host, false).unwrap();
        assert_eq!(p.runner, "darling");
    }

    #[test]
    fn pkg_runs_darling_installer() {
        let t = Tmp::new();
        let p = plan_for(&t, "Setup.pkg", &pkg(), &FakeHost::new(Arch::X86_64, &["darling"]), false).unwrap();
        assert_eq!(p.runner, "darling-installer");
        assert_eq!(p.argv[2..4], os(&["installer", "-pkg"]));
        assert!(p.argv[4].to_str().unwrap().starts_with("/Volumes/SystemRoot/"));
    }

    #[test]
    fn dos() {
        let t = Tmp::new();
        let mut data = b"MZ".to_vec();
        data.resize(102, 0);
        let p = plan_for(&t, "game.exe", &data, &FakeHost::new(Arch::X86_64, &["dosbox"]), false).unwrap();
        assert_eq!(p.runner, "dosbox");
    }

    #[test]
    fn safe_names() {
        assert_eq!(safe_name("My Game"), "My_Game");
        assert_eq!(safe_name("..hidden_"), "hidden");
        assert_eq!(safe_name("///"), "app");
    }

    #[test]
    fn describe_shows_profile_and_env() {
        let mut env = BTreeMap::new();
        env.insert("WINEPREFIX".to_string(), "/p".to_string());
        let p = Plan { runner: "wine", argv: os(&["/usr/bin/wine", "a.exe"]), env, profile: Some("x".into()) };
        assert_eq!(p.describe(), "[wine, profile x] WINEPREFIX=/p /usr/bin/wine a.exe");
    }
}
