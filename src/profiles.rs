//! Per-program profiles: dedicated settings in TOML files.
//!
//! Profiles are looked up in `~/.config/uos/profiles/` and then in
//! `/etc/uos/profiles/`; the first one that matches wins (within a directory,
//! files are tried in alphabetical order). Example:
//!
//! ```toml
//! # ~/.config/uos/profiles/notepadpp.toml
//! [match]
//! name = "notepad++*.exe"      # glob on the file name (case-insensitive)
//! # sha256 = "…"               # or the exact hash
//! # bundle_id = "com.foo.bar"  # or the identifier of an .app
//!
//! [run]
//! prefix = "editor"            # named Wine prefix
//! wine = "/opt/proton/files/bin/wine"
//! args = ["-multiInst"]
//! env = { DXVK_HUD = "fps" }
//! dll_overrides = { d3d11 = "n,b", dxgi = "n,b" }
//! emulator = "FEXInterpreter"  # preferred CPU translator
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use glob::{MatchOptions, Pattern};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::detect::BinaryInfo;
use crate::error::{Error, Result};
use crate::paths::Dirs;

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MatchSpec {
    pub name: Option<String>,
    pub sha256: Option<String>,
    pub bundle_id: Option<String>,
}

impl MatchSpec {
    fn is_empty(&self) -> bool {
        self.name.is_none() && self.sha256.is_none() && self.bundle_id.is_none()
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    pub prefix: Option<String>,
    #[serde(default)]
    pub isolate: bool,
    pub wine: Option<PathBuf>,
    pub emulator: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub dll_overrides: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileFile {
    #[serde(default, rename = "match")]
    matcher: MatchSpec,
    #[serde(default)]
    run: RunSpec,
}

#[derive(Debug, Clone, Default)]
pub struct Profile {
    /// File name without `.toml`.
    pub name: String,
    pub source: PathBuf,
    pub matcher: MatchSpec,
    pub run: RunSpec,
}

impl Profile {
    /// Value for `WINEDLLOVERRIDES`, if the profile overrides any DLL.
    pub fn wine_dll_overrides(&self) -> Option<String> {
        if self.run.dll_overrides.is_empty() {
            return None;
        }
        let parts: Vec<String> = self.run.dll_overrides.iter().map(|(dll, mode)| format!("{dll}={mode}")).collect();
        Some(parts.join(";"))
    }
}

pub fn load(path: &Path) -> Result<Profile> {
    let text = fs::read_to_string(path).map_err(|e| Error::Profile(format!("{}: {e}", path.display())))?;
    let file: ProfileFile =
        toml::from_str(&text).map_err(|e| Error::Profile(format!("{}: {}", path.display(), e.message())))?;
    Ok(Profile {
        name: path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        source: path.to_path_buf(),
        matcher: file.matcher,
        run: file.run,
    })
}

fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn glob_matches(pattern: &str, name: &str) -> bool {
    let options =
        MatchOptions { case_sensitive: false, require_literal_separator: false, require_literal_leading_dot: false };
    match Pattern::new(pattern) {
        Ok(p) => p.matches_with(name, options),
        // Not a valid glob: compare literally
        Err(_) => pattern.eq_ignore_ascii_case(name),
    }
}

/// Every criterion in `[match]` must hold; an empty `[match]` never matches.
pub fn matches(profile: &Profile, info: &BinaryInfo) -> bool {
    let m = &profile.matcher;
    if m.is_empty() {
        return false;
    }
    if let Some(pattern) = &m.name {
        let base = info.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !glob_matches(pattern, &base) {
            return false;
        }
    }
    if let Some(id) = &m.bundle_id {
        if info.details.bundle_id.as_ref() != Some(id) {
            return false;
        }
    }
    if let Some(hash) = &m.sha256 {
        if info.path.is_dir() {
            return false;
        }
        match sha256_file(&info.path) {
            Ok(h) if h == hash.to_ascii_lowercase() => {}
            _ => return false,
        }
    }
    true
}

fn all_profile_files(dirs: &Dirs) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in dirs.profile_dirs() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "toml"))
            .collect();
        files.sort();
        out.extend(files);
    }
    out
}

/// Number of profile files available.
pub fn count(dirs: &Dirs) -> usize {
    all_profile_files(dirs).len()
}

/// The first profile that matches `info`.
pub fn find(info: &BinaryInfo, dirs: &Dirs) -> Result<Option<Profile>> {
    for path in all_profile_files(dirs) {
        let profile = load(&path)?;
        if matches(&profile, info) {
            return Ok(Some(profile));
        }
    }
    Ok(None)
}

pub fn by_name(name: &str, dirs: &Dirs) -> Result<Profile> {
    for dir in dirs.profile_dirs() {
        let path = dir.join(format!("{name}.toml"));
        if path.is_file() {
            return load(&path);
        }
    }
    let searched: Vec<String> = dirs.profile_dirs().iter().map(|d| d.display().to_string()).collect();
    Err(Error::Profile(format!("profile '{name}' not found in {}", searched.join(", "))))
}

/// Text of a new profile that matches `info`.
pub fn template(info: &BinaryInfo) -> String {
    let base = info.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = info.path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let matcher = match &info.details.bundle_id {
        Some(id) => format!("bundle_id = {}", toml_string(id)),
        None => format!("name = {}", toml_string(&base)),
    };
    let prefix = if stem.is_empty() { "app".to_string() } else { stem };
    format!(
        "[match]\n{matcher}\n\n[run]\n# prefix = {}\n# args = []\n# env = {{ }}\n# dll_overrides = {{ }}\n\
         # wine = \"/path/to/wine\"\n# emulator = \"box64\"\n",
        toml_string(&prefix)
    )
}

fn toml_string(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{detect, Arch};
    use crate::runners::plan;
    use crate::testutil::*;

    fn write_profile(t: &Tmp, name: &str, text: &str) -> PathBuf {
        let path = t.dirs.user_profiles().join(format!("{name}.toml"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    fn exe(t: &Tmp, name: &str) -> BinaryInfo {
        detect(&t.write(name, &pe(0x8664, PeOpts::default()))).unwrap()
    }

    #[test]
    fn match_by_name_and_apply() {
        let t = Tmp::new();
        write_profile(
            &t,
            "npp",
            r#"
[match]
name = "Notepad++*.EXE"
[run]
prefix = "editor"
args = ["-multiInst"]
env = { DXVK_HUD = "fps" }
dll_overrides = { d3d11 = "n,b", dxgi = "n,b" }
"#,
        );
        let info = exe(&t, "notepad++_8.6.exe");
        let profile = find(&info, &t.dirs).unwrap().unwrap();
        assert_eq!(profile.name, "npp");
        let host = FakeHost::new(Arch::X86_64, &["wine"]);
        let p = plan(&info, &["file.txt".into()], false, Some(&profile), &host, &t.dirs).unwrap();
        assert_eq!(&p.argv[p.argv.len() - 2..], &["-multiInst".into(), "file.txt".into()] as &[std::ffi::OsString]);
        assert!(p.env["WINEPREFIX"].ends_with("prefixes/editor"));
        assert_eq!(p.env["WINEDLLOVERRIDES"], "d3d11=n,b;dxgi=n,b");
        assert_eq!(p.env["DXVK_HUD"], "fps");
        assert_eq!(p.profile.as_deref(), Some("npp"));
    }

    #[test]
    fn match_by_sha256() {
        let t = Tmp::new();
        let info = exe(&t, "game.exe");
        let digest = sha256_file(&info.path).unwrap();
        write_profile(&t, "game", &format!("[match]\nsha256 = \"{}\"\n", digest.to_uppercase()));
        assert_eq!(find(&info, &t.dirs).unwrap().unwrap().name, "game");
        let other = detect(&t.write("other.exe", &pe(0x14C, PeOpts::default()))).unwrap();
        assert!(find(&other, &t.dirs).unwrap().is_none());
    }

    #[test]
    fn match_by_bundle_id() {
        let t = Tmp::new();
        write_profile(&t, "hello", "[match]\nbundle_id = \"com.example.hello\"\n");
        let info = detect(&t.make_app("", "Hello", "com.example.hello", false)).unwrap();
        assert_eq!(find(&info, &t.dirs).unwrap().unwrap().name, "hello");
    }

    #[test]
    fn profile_without_match_never_applies() {
        let t = Tmp::new();
        write_profile(&t, "empty", "[run]\nisolate = true\n");
        assert!(find(&exe(&t, "x.exe"), &t.dirs).unwrap().is_none());
    }

    #[test]
    fn user_profile_wins_over_system() {
        let t = Tmp::new();
        fs::create_dir_all(&t.dirs.system_profiles).unwrap();
        fs::write(t.dirs.system_profiles.join("a.toml"), "[match]\nname = \"*.exe\"\n").unwrap();
        write_profile(&t, "z", "[match]\nname = \"*.exe\"\n");
        assert_eq!(find(&exe(&t, "x.exe"), &t.dirs).unwrap().unwrap().name, "z");
    }

    #[test]
    fn unknown_key_is_reported() {
        let t = Tmp::new();
        write_profile(&t, "bad", "[match]\nname = \"*\"\n[run]\nprefx = \"oops\"\n");
        let err = find(&exe(&t, "x.exe"), &t.dirs).unwrap_err().to_string();
        assert!(err.contains("prefx"), "{err}");
    }

    #[test]
    fn wrong_type_is_reported() {
        let t = Tmp::new();
        write_profile(&t, "bad", "[match]\nname = \"*\"\n[run]\nargs = \"-x\"\n");
        let err = find(&exe(&t, "x.exe"), &t.dirs).unwrap_err().to_string();
        assert!(err.contains("bad.toml") && err.contains("sequence"), "{err}");
    }

    #[test]
    fn missing_custom_wine() {
        let t = Tmp::new();
        write_profile(&t, "p", "[match]\nname = \"*\"\n[run]\nwine = \"/nope/wine\"\n");
        let info = exe(&t, "x.exe");
        let profile = find(&info, &t.dirs).unwrap();
        let host = FakeHost::new(Arch::X86_64, &["wine"]);
        let err = plan(&info, &[], false, profile.as_ref(), &host, &t.dirs).unwrap_err().to_string();
        assert!(err.contains("/nope/wine"), "{err}");
    }

    #[test]
    fn by_name_and_missing() {
        let t = Tmp::new();
        write_profile(&t, "forced", "[run]\nargs = [\"-safe\"]\n");
        assert_eq!(by_name("forced", &t.dirs).unwrap().run.args, vec!["-safe"]);
        assert!(by_name("nope", &t.dirs).is_err());
    }

    #[test]
    fn template_is_loadable_and_matches() {
        let t = Tmp::new();
        let info = exe(&t, "My \"Game\".exe");
        let path = write_profile(&t, "game", &template(&info));
        let profile = load(&path).unwrap();
        assert!(matches(&profile, &info));
    }

    #[test]
    fn example_profile_in_docs_is_valid() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/examples/game-dxvk.toml");
        let profile = load(&path).unwrap();
        assert_eq!(profile.run.prefix.as_deref(), Some("games"));
        assert_eq!(profile.wine_dll_overrides().as_deref(), Some("d3d11=n,b;dxgi=n,b"));
    }
}
