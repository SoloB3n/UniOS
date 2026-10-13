//! Test helpers: temporary trees, synthetic binary headers and a fake host.

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use crate::detect::Arch;
use crate::host::Host;
use crate::paths::Dirs;

pub struct Tmp {
    dir: TempDir,
    pub dirs: Dirs,
}

impl Tmp {
    pub fn new() -> Tmp {
        let dir = tempfile::tempdir().unwrap();
        let dirs = Dirs::under(dir.path());
        Tmp { dir, dirs }
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    pub fn write(&self, rel: &str, data: &[u8]) -> PathBuf {
        let p = self.path(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, data).unwrap();
        p
    }

    pub fn write_executable(&self, rel: &str, data: &[u8]) -> PathBuf {
        let p = self.write(rel, data);
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    /// Create `<root>/<name>.app` with an x86_64 Mach-O executable and,
    /// optionally, an .icns icon with PNG entries.
    pub fn make_app(&self, root: &str, name: &str, bundle_id: &str, icon: bool) -> PathBuf {
        make_app_in(&self.path(root), name, bundle_id, icon)
    }
}

pub fn make_app_in(root: &Path, name: &str, bundle_id: &str, icon: bool) -> PathBuf {
    let app = root.join(format!("{name}.app"));
    let contents = app.join("Contents");
    fs::create_dir_all(contents.join("MacOS")).unwrap();
    fs::write(contents.join("MacOS").join(name), macho(0x0100_0007)).unwrap();
    let icon_entry = if icon { "<key>CFBundleIconFile</key><string>AppIcon</string>" } else { "" };
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>{name}</string>
<key>CFBundleIdentifier</key><string>{bundle_id}</string>
<key>CFBundleName</key><string>{name}</string>
{icon_entry}
</dict></plist>"#
    );
    fs::write(contents.join("Info.plist"), plist).unwrap();
    if icon {
        fs::create_dir_all(contents.join("Resources")).unwrap();
        let data = icns(&[(b"ic07", png(128)), (b"ic10", png(1024)), (b"is32", vec![b'x'; 20])]);
        fs::write(contents.join("Resources").join("AppIcon.icns"), data).unwrap();
    }
    app
}

pub fn elf(machine: u16, class: u8) -> Vec<u8> {
    let mut d = b"\x7fELF".to_vec();
    d.extend_from_slice(&[class, 1, 1]);
    d.resize(16, 0);
    d.extend_from_slice(&2u16.to_le_bytes());
    d.extend_from_slice(&machine.to_le_bytes());
    d.resize(64, 0);
    d
}

#[derive(Clone, Copy)]
pub struct PeOpts {
    pub dll: bool,
    pub pe32plus: bool,
    pub dotnet: bool,
    pub subsystem: u16,
}

impl Default for PeOpts {
    fn default() -> Self {
        PeOpts { dll: false, pe32plus: true, dotnet: false, subsystem: 2 }
    }
}

/// A minimal PE image: MZ stub, PE header at 0x80, optional header.
pub fn pe(machine: u16, o: PeOpts) -> Vec<u8> {
    let pe_off = 0x80usize;
    let mut d = vec![0u8; pe_off];
    d[..2].copy_from_slice(b"MZ");
    d[0x3C..0x40].copy_from_slice(&(pe_off as u32).to_le_bytes());
    d.extend_from_slice(b"PE\0\0");
    let characteristics: u16 = if o.dll { 0x2000 } else { 0x0102 };
    d.extend_from_slice(&machine.to_le_bytes());
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&[0; 12]);
    d.extend_from_slice(&0xF0u16.to_le_bytes());
    d.extend_from_slice(&characteristics.to_le_bytes());
    let mut opt = vec![0u8; 0xF0];
    opt[0..2].copy_from_slice(&(if o.pe32plus { 0x20Bu16 } else { 0x10B }).to_le_bytes());
    opt[68..70].copy_from_slice(&o.subsystem.to_le_bytes());
    if o.dotnet {
        let clr = if o.pe32plus { 112 } else { 96 } + 14 * 8;
        opt[clr..clr + 4].copy_from_slice(&0x2008u32.to_le_bytes());
        opt[clr + 4..clr + 8].copy_from_slice(&0x48u32.to_le_bytes());
    }
    d.extend_from_slice(&opt);
    d
}

pub fn macho(cpu: u32) -> Vec<u8> {
    let mut d = Vec::new();
    for v in [0xFEED_FACFu32, cpu, 3, 2, 0, 0, 0, 0] {
        d.extend_from_slice(&v.to_le_bytes());
    }
    d
}

pub fn fat(cpus: &[u32]) -> Vec<u8> {
    let mut d = 0xCAFE_BABEu32.to_be_bytes().to_vec();
    d.extend_from_slice(&(cpus.len() as u32).to_be_bytes());
    for &cpu in cpus {
        for v in [cpu, 3, 0x1000, 0x1000, 12] {
            d.extend_from_slice(&v.to_be_bytes());
        }
    }
    d
}

pub fn dmg() -> Vec<u8> {
    let mut d = vec![0u8; 2048];
    d.extend_from_slice(b"koly");
    d.resize(2048 + 512, 0);
    d
}

pub fn pkg() -> Vec<u8> {
    let mut d = b"xar!".to_vec();
    d.resize(28, 0);
    d
}

pub fn png(width: u32) -> Vec<u8> {
    let mut d = b"\x89PNG\r\n\x1a\n".to_vec();
    d.extend_from_slice(&13u32.to_be_bytes());
    d.extend_from_slice(b"IHDR");
    d.extend_from_slice(&width.to_be_bytes());
    d.extend_from_slice(&width.to_be_bytes());
    d.extend_from_slice(&[0; 8]);
    d
}

pub fn icns(chunks: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (kind, data) in chunks {
        body.extend_from_slice(*kind);
        body.extend_from_slice(&(data.len() as u32 + 8).to_be_bytes());
        body.extend_from_slice(data);
    }
    let mut d = b"icns".to_vec();
    d.extend_from_slice(&(body.len() as u32 + 8).to_be_bytes());
    d.extend_from_slice(&body);
    d
}

/// A host with a chosen architecture and set of installed tools
/// (each reported at `/usr/bin/<name>`).
pub struct FakeHost {
    pub arch: Arch,
    pub tools: Vec<&'static str>,
    pub vars: HashMap<String, String>,
}

impl FakeHost {
    pub fn new(arch: Arch, tools: &[&'static str]) -> FakeHost {
        FakeHost { arch, tools: tools.to_vec(), vars: HashMap::new() }
    }

    pub fn with_var(mut self, key: &str, value: &str) -> FakeHost {
        self.vars.insert(key.to_string(), value.to_string());
        self
    }
}

impl Host for FakeHost {
    fn arch(&self) -> Arch {
        self.arch.clone()
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        self.tools.contains(&name).then(|| PathBuf::from("/usr/bin").join(name))
    }

    fn var(&self, key: &str) -> Option<String> {
        self.vars.get(key).cloned()
    }
}
