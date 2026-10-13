//! Installing programs and adding them to the applications menu.
//!
//! * .app / .dmg      -> copied to ~/Applications, menu entry with the .icns icon
//! * AppImage / ELF   -> copied to ~/Applications, menu entry
//! * .exe / .msi      -> the installer runs under Wine (which creates its own entries)
//! * .pkg             -> installed with Darling's installer
//!
//! Entries created by uos are `uos-*.desktop` files whose `X-UOS-Installed`
//! key points to the copied program, so `uos uninstall` knows what to remove.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::detect::{detect, Family};
use crate::error::{Error, Result};
use crate::host::Host;
use crate::macos::{self, Extractor};
use crate::paths::Dirs;
use crate::runners::{self, Plan};

const DESKTOP_PREFIX: &str = "uos-";

pub enum Outcome {
    /// The program was copied and this menu entry was created.
    MenuEntry(PathBuf),
    /// Installing means running an installer (Wine or Darling).
    Installer(Plan),
}

/// Quote an argument for the Exec key (Desktop Entry Specification).
pub fn desktop_quote(arg: &str) -> String {
    let arg = arg.replace('%', "%%");
    let safe = |c: char| c.is_ascii_alphanumeric() || "_./+:@=,-".contains(c);
    if !arg.is_empty() && arg.chars().all(safe) {
        return arg;
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "app".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Desktop entry values cannot contain line breaks.
fn single_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

pub fn write_desktop_entry(
    dirs: &Dirs,
    slug: &str,
    name: &str,
    target: &Path,
    icon: Option<&Path>,
    comment: &str,
) -> Result<PathBuf> {
    fs::create_dir_all(dirs.desktop_entries())?;
    let path = dirs.desktop_entries().join(format!("{DESKTOP_PREFIX}{slug}.desktop"));
    let target_str = target.to_string_lossy();
    let icon = icon.map(|i| i.to_string_lossy().into_owned()).unwrap_or_else(|| "application-x-executable".into());
    let text = format!(
        "[Desktop Entry]\nType=Application\nName={}\nComment={comment}\nExec=uos run {} %U\nIcon={}\n\
         Terminal=false\nCategories=Utility;\nX-UOS-Installed={}\n",
        single_line(name),
        desktop_quote(&single_line(&target_str)),
        single_line(&icon),
        single_line(&target_str),
    );
    fs::write(&path, text)?;
    Ok(path)
}

fn copy_tree(src: &Path, dest: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(fs::read_link(src)?, dest)
    } else if meta.is_dir() {
        fs::create_dir(dest)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_tree(&entry.path(), &dest.join(entry.file_name()))?;
        }
        fs::set_permissions(dest, meta.permissions())
    } else {
        fs::copy(src, dest).map(|_| ())
    }
}

fn copy_into_applications(src: &Path, dirs: &Dirs) -> Result<PathBuf> {
    fs::create_dir_all(dirs.applications())?;
    let name = src.file_name().ok_or_else(|| Error::Install(format!("invalid path {}", src.display())))?;
    let dest = dirs.applications().join(name);
    if std::path::absolute(src)? == dest {
        return Ok(dest);
    }
    if fs::symlink_metadata(&dest).is_ok() {
        return Err(Error::Install(format!(
            "{} already exists: uninstall it first with uos uninstall",
            dest.display()
        )));
    }
    copy_tree(src, &dest)?;
    if dest.is_file() {
        let mut perms = fs::metadata(&dest)?.permissions();
        perms.set_mode(perms.mode() | 0o111);
        fs::set_permissions(&dest, perms)?;
    }
    Ok(dest)
}

fn install_app_bundle(app: &Path, dirs: &Dirs) -> Result<PathBuf> {
    let info = detect(app)?;
    if info.family != Family::MacApp {
        return Err(Error::Install(format!("{} is not a valid .app bundle", app.display())));
    }
    let dest = copy_into_applications(app, dirs)?;
    let stem = app.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let name = info.details.name.clone().unwrap_or(stem);
    let slug = slug(info.details.bundle_id.as_deref().unwrap_or(&name));
    let mut icon = None;
    if let Some(png) = macos::bundle_icon(&dest) {
        fs::create_dir_all(dirs.icons())?;
        let path = dirs.icons().join(format!("{slug}.png"));
        fs::write(&path, png)?;
        icon = Some(path);
    }
    write_desktop_entry(dirs, &slug, &name, &dest, icon.as_deref(), "macOS application (Darling)")
}

/// Menu name for an AppImage: "Krita-5.2-x86_64.AppImage" -> "Krita-5.2".
fn linux_app_name(base: &str) -> String {
    const SUFFIX: &str = ".appimage";
    if base.len() <= SUFFIX.len() || !base.to_ascii_lowercase().ends_with(SUFFIX) {
        return base.to_string();
    }
    let mut name = &base[..base.len() - SUFFIX.len()];
    for arch in ["x86_64", "amd64", "aarch64", "arm64"] {
        if name.len() >= arch.len() && name[name.len() - arch.len()..].eq_ignore_ascii_case(arch) {
            name = &name[..name.len() - arch.len()];
            break;
        }
    }
    let name = name.strip_suffix(['-', '_', '.']).unwrap_or(name);
    if name.is_empty() {
        base.to_string()
    } else {
        name.to_string()
    }
}

fn install_linux_binary(path: &Path, dirs: &Dirs) -> Result<PathBuf> {
    let dest = copy_into_applications(path, dirs)?;
    let base = dest.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let name = linux_app_name(&base);
    write_desktop_entry(dirs, &slug(&name), &name, &dest, None, "Linux application")
}

pub fn install(path: &Path, host: &dyn Host, dirs: &Dirs, extractor: &dyn Extractor) -> Result<Outcome> {
    let info = detect(path)?;
    match info.family {
        Family::MacDmg => install(&macos::resolve_dmg(path, dirs, extractor)?, host, dirs, extractor),
        Family::MacApp => Ok(Outcome::MenuEntry(install_app_bundle(path, dirs)?)),
        Family::Pe | Family::Msi | Family::MacPkg => {
            Ok(Outcome::Installer(runners::plan(&info, &[], false, None, host, dirs)?))
        }
        Family::Elf => Ok(Outcome::MenuEntry(install_linux_binary(path, dirs)?)),
        other => Err(Error::Install(format!(
            "don't know how to install a file of type '{}'",
            serde_json::to_value(other).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
        ))),
    }
}

#[derive(Debug, Clone, Default)]
pub struct Installed {
    pub id: String,
    pub desktop: PathBuf,
    pub name: String,
    pub icon: String,
    pub target: String,
}

/// Programs installed by uos.
pub fn installed(dirs: &Dirs) -> Vec<Installed> {
    let Ok(entries) = fs::read_dir(dirs.desktop_entries()) else { return Vec::new() };
    let mut files: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let file_name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let Some(id) = file_name.strip_prefix(DESKTOP_PREFIX).and_then(|s| s.strip_suffix(".desktop")) else {
            continue;
        };
        let mut entry = Installed { id: id.to_string(), desktop: path.clone(), ..Installed::default() };
        for line in fs::read_to_string(&path).unwrap_or_default().lines() {
            match line.split_once('=') {
                Some(("Name", v)) => entry.name = v.to_string(),
                Some(("Icon", v)) => entry.icon = v.to_string(),
                Some(("X-UOS-Installed", v)) => entry.target = v.to_string(),
                _ => {}
            }
        }
        out.push(entry);
    }
    out
}

/// Remove menu entry, icon and copied program. Returns the removed paths.
pub fn uninstall(ident: &str, dirs: &Dirs) -> Result<Vec<PathBuf>> {
    let matches: Vec<Installed> = installed(dirs)
        .into_iter()
        .filter(|e| {
            let base = Path::new(&e.target).file_name().map(|n| n.to_string_lossy().into_owned());
            e.id == ident || e.name == ident || base.as_deref() == Some(ident)
        })
        .collect();
    let entry = match matches.as_slice() {
        [] => {
            return Err(Error::Install(format!("no installed program matches '{ident}' (see uos list)")));
        }
        [one] => one,
        many => {
            let ids: Vec<&str> = many.iter().map(|e| e.id.as_str()).collect();
            return Err(Error::Install(format!("'{ident}' is ambiguous: {}", ids.join(", "))));
        }
    };

    let mut removed = Vec::new();
    // Only ever remove what sits directly inside ~/Applications
    let target = PathBuf::from(&entry.target);
    if !entry.target.is_empty() && target.parent() == Some(dirs.applications().as_path()) {
        if let Ok(meta) = fs::symlink_metadata(&target) {
            if meta.is_dir() {
                fs::remove_dir_all(&target)?;
            } else {
                fs::remove_file(&target)?;
            }
            removed.push(target);
        }
    }
    let icon = PathBuf::from(&entry.icon);
    if icon.parent() == Some(dirs.icons().as_path()) && icon.is_file() {
        fs::remove_file(&icon)?;
        removed.push(icon);
    }
    fs::remove_file(&entry.desktop)?;
    removed.push(entry.desktop.clone());
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::Arch;
    use crate::macos::SevenZip;
    use crate::testutil::*;

    fn no_7z() -> SevenZip {
        SevenZip::from_host(&FakeHost::new(Arch::X86_64, &[]))
    }

    fn entry_path(outcome: Outcome) -> PathBuf {
        match outcome {
            Outcome::MenuEntry(p) => p,
            Outcome::Installer(_) => panic!("expected a menu entry"),
        }
    }

    #[test]
    fn install_app_bundle_with_icon() {
        let t = Tmp::new();
        let app = t.make_app("dl", "Hello", "com.example.hello", true);
        let host = FakeHost::new(Arch::X86_64, &[]);
        let entry = entry_path(install(&app, &host, &t.dirs, &no_7z()).unwrap());
        let dest = t.dirs.applications().join("Hello.app");
        assert!(dest.is_dir());
        let text = fs::read_to_string(&entry).unwrap();
        assert!(text.contains("Name=Hello\n"));
        assert!(text.contains(&format!("Exec=uos run {} %U\n", dest.display())));
        let icon = text.lines().find_map(|l| l.strip_prefix("Icon=")).unwrap();
        assert!(fs::read(icon).unwrap().starts_with(b"\x89PNG"));
        assert_eq!(entry.file_name().unwrap(), "uos-com-example-hello.desktop");
    }

    #[test]
    fn install_twice_is_refused() {
        let t = Tmp::new();
        let app = t.make_app("dl", "Hello", "com.example.hello", false);
        let host = FakeHost::new(Arch::X86_64, &[]);
        install(&app, &host, &t.dirs, &no_7z()).unwrap();
        let err = install(&app, &host, &t.dirs, &no_7z()).err().unwrap().to_string();
        assert!(err.contains("already exists"), "{err}");
    }

    #[test]
    fn install_keeps_symlinks_inside_bundles() {
        let t = Tmp::new();
        let app = t.make_app("dl", "Hello", "com.example.hello", false);
        std::os::unix::fs::symlink("MacOS/Hello", app.join("Contents/Current")).unwrap();
        install(&app, &FakeHost::new(Arch::X86_64, &[]), &t.dirs, &no_7z()).unwrap();
        let copied = t.dirs.applications().join("Hello.app/Contents/Current");
        assert_eq!(fs::read_link(copied).unwrap(), Path::new("MacOS/Hello"));
    }

    #[test]
    fn install_appimage() {
        let t = Tmp::new();
        let img = t.write("dl/Krita-5.2-x86_64.AppImage", &elf(0x3E, 2));
        let entry = entry_path(install(&img, &FakeHost::new(Arch::X86_64, &[]), &t.dirs, &no_7z()).unwrap());
        assert!(fs::read_to_string(entry).unwrap().contains("Name=Krita-5.2\n"));
        assert!(crate::host::is_executable(&t.dirs.applications().join("Krita-5.2-x86_64.AppImage")));
    }

    #[test]
    fn install_exe_returns_installer_plan() {
        let t = Tmp::new();
        let exe = t.write("setup.exe", &pe(0x14C, PeOpts::default()));
        match install(&exe, &FakeHost::new(Arch::X86_64, &["wine"]), &t.dirs, &no_7z()).unwrap() {
            Outcome::Installer(p) => assert_eq!(p.runner, "wine"),
            Outcome::MenuEntry(_) => panic!("expected an installer"),
        }
    }

    #[test]
    fn install_unknown_is_refused() {
        let t = Tmp::new();
        let f = t.write("notes.txt", b"hello");
        let err = install(&f, &FakeHost::new(Arch::X86_64, &[]), &t.dirs, &no_7z()).err().unwrap().to_string();
        assert!(err.contains("'unknown'"), "{err}");
    }

    #[test]
    fn list_and_uninstall() {
        let t = Tmp::new();
        let app = t.make_app("dl", "Hello", "com.example.hello", true);
        install(&app, &FakeHost::new(Arch::X86_64, &[]), &t.dirs, &no_7z()).unwrap();
        let list = installed(&t.dirs);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Hello");
        let removed = uninstall("Hello", &t.dirs).unwrap();
        assert_eq!(removed.len(), 3); // .app, icon, menu entry
        assert!(installed(&t.dirs).is_empty());
        assert!(!t.dirs.applications().join("Hello.app").exists());
    }

    #[test]
    fn uninstall_never_deletes_outside_applications() {
        let t = Tmp::new();
        let outside = t.write("important/data.bin", b"x");
        let icon = t.write("important/icon.png", b"x");
        write_desktop_entry(&t.dirs, "evil", "Evil", &outside, Some(&icon), "x").unwrap();
        uninstall("evil", &t.dirs).unwrap();
        assert!(outside.exists());
        assert!(icon.exists());
    }

    #[test]
    fn uninstall_unknown_and_ambiguous() {
        let t = Tmp::new();
        assert!(uninstall("nothing", &t.dirs).is_err());
        let a = t.write("a", b"x");
        write_desktop_entry(&t.dirs, "one", "Same", &a, None, "x").unwrap();
        write_desktop_entry(&t.dirs, "two", "Same", &a, None, "x").unwrap();
        let err = uninstall("Same", &t.dirs).unwrap_err().to_string();
        assert!(err.contains("ambiguous"), "{err}");
    }

    #[test]
    fn desktop_quoting() {
        assert_eq!(desktop_quote("/a/b.app"), "/a/b.app");
        assert_eq!(desktop_quote("/x/My \"App\" 100%"), "\"/x/My \\\"App\\\" 100%%\"");
        assert_eq!(desktop_quote("/x/$HOME"), "\"/x/\\$HOME\"");
    }

    #[test]
    fn appimage_names() {
        assert_eq!(linux_app_name("Krita-5.2-x86_64.AppImage"), "Krita-5.2");
        assert_eq!(linux_app_name("App.AppImage"), "App");
        assert_eq!(linux_app_name("tool_ARM64.appimage"), "tool");
        assert_eq!(linux_app_name("prog"), "prog");
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("com.example.Hello"), "com-example-hello");
        assert_eq!(slug("!!!"), "app");
    }
}
