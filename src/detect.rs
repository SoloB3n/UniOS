//! Executable format detection from magic bytes.
//!
//! The file extension is never trusted: the header decides the family
//! (ELF / PE / Mach-O / ...), the CPU architecture and the details the
//! dispatcher needs to pick the right compatibility layer.

use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Serialize, Serializer};

const HEADER_SIZE: u64 = 4096;
const MSI_MAGIC: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Family {
    /// Linux / Unix
    Elf,
    /// Windows (PE32 / PE32+)
    Pe,
    /// MS-DOS (MZ without a PE header)
    Dos,
    /// Windows Installer (OLE Compound File)
    Msi,
    /// macOS, single architecture
    Macho,
    /// macOS universal binary
    MachoFat,
    /// macOS .app bundle
    MacApp,
    /// macOS .dmg disk image (UDIF)
    MacDmg,
    /// macOS .pkg installer package (xar archive)
    MacPkg,
    JavaClass,
    /// `#!` script
    Script,
    /// Windows .bat / .cmd
    WinBatch,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Arch {
    I386,
    X86_64,
    Arm,
    Aarch64,
    Riscv64,
    Ppc,
    Ppc64,
    Other(String),
}

impl Arch {
    pub fn from_name(name: &str) -> Arch {
        match name.to_ascii_lowercase().as_str() {
            "x86_64" | "amd64" => Arch::X86_64,
            "aarch64" | "arm64" => Arch::Aarch64,
            "x86" | "i386" | "i686" => Arch::I386,
            "arm" => Arch::Arm,
            "riscv64" => Arch::Riscv64,
            "powerpc" | "ppc" => Arch::Ppc,
            "powerpc64" | "ppc64" => Arch::Ppc64,
            other => Arch::Other(other.to_string()),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Arch::I386 => "i386",
            Arch::X86_64 => "x86_64",
            Arch::Arm => "arm",
            Arch::Aarch64 => "aarch64",
            Arch::Riscv64 => "riscv64",
            Arch::Ppc => "ppc",
            Arch::Ppc64 => "ppc64",
            Arch::Other(s) => s,
        }
    }

    fn from_elf(machine: u16) -> Arch {
        match machine {
            0x03 => Arch::I386,
            0x3E => Arch::X86_64,
            0x28 => Arch::Arm,
            0xB7 => Arch::Aarch64,
            0xF3 => Arch::Riscv64,
            0x14 => Arch::Ppc,
            0x15 => Arch::Ppc64,
            m => Arch::Other(format!("elf-machine-{m:#x}")),
        }
    }

    fn from_pe(machine: u16) -> Arch {
        match machine {
            0x014C => Arch::I386,
            0x8664 => Arch::X86_64,
            0x01C4 => Arch::Arm,
            0xAA64 => Arch::Aarch64,
            m => Arch::Other(format!("pe-machine-{m:#x}")),
        }
    }

    fn from_macho(cpu: u32) -> Arch {
        match cpu {
            7 => Arch::I386,
            0x0100_0007 => Arch::X86_64,
            12 => Arch::Arm,
            0x0100_000C => Arch::Aarch64,
            18 => Arch::Ppc,
            0x0100_0012 => Arch::Ppc64,
            c => Arch::Other(format!("macho-cpu-{c:#x}")),
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl Serialize for Arch {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

/// Format-specific details; only the fields that apply are set.
#[derive(Debug, Default, Clone, Serialize)]
pub struct Details {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bits: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endian: Option<&'static str>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub elf_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dll: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subsystem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dotnet: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filetype: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slices: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interpreter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct BinaryInfo {
    pub path: PathBuf,
    pub family: Family,
    pub archs: Vec<Arch>,
    pub details: Details,
    /// For .app bundles: the real executable inside Contents/MacOS.
    pub executable: Option<PathBuf>,
}

impl BinaryInfo {
    fn new(path: &Path, family: Family) -> BinaryInfo {
        BinaryInfo {
            path: path.to_path_buf(),
            family,
            archs: Vec::new(),
            details: Details::default(),
            executable: None,
        }
    }

    fn with_error(path: &Path, family: Family, error: &str) -> BinaryInfo {
        let mut info = BinaryInfo::new(path, family);
        info.details.error = Some(error.to_string());
        info
    }

    pub fn arch(&self) -> Option<&Arch> {
        self.archs.first()
    }
}

fn u16_at(b: &[u8], off: usize, little: bool) -> Option<u16> {
    let bytes: [u8; 2] = b.get(off..off + 2)?.try_into().ok()?;
    Some(if little { u16::from_le_bytes(bytes) } else { u16::from_be_bytes(bytes) })
}

fn u32_at(b: &[u8], off: usize, little: bool) -> Option<u32> {
    let bytes: [u8; 4] = b.get(off..off + 4)?.try_into().ok()?;
    Some(if little { u32::from_le_bytes(bytes) } else { u32::from_be_bytes(bytes) })
}

/// Analyse `path` (a file or an .app directory).
pub fn detect(path: &Path) -> io::Result<BinaryInfo> {
    if path.is_dir() {
        return Ok(detect_app_bundle(path));
    }

    let mut file = File::open(path)?;
    let mut head = Vec::new();
    (&mut file).take(HEADER_SIZE).read_to_end(&mut head)?;
    let tail = read_tail(&mut file, 512)?;

    if head.starts_with(b"\x7fELF") {
        return Ok(detect_elf(path, &head));
    }
    if head.starts_with(b"MZ") {
        return detect_mz(path, &head, &mut file);
    }
    if head.starts_with(&MSI_MAGIC) {
        return Ok(BinaryInfo::new(path, Family::Msi));
    }
    if let Some(magic) = u32_at(&head, 0, false) {
        if matches!(magic, 0xFEED_FACE | 0xFEED_FACF | 0xCEFA_EDFE | 0xCFFA_EDFE) {
            return Ok(detect_macho(path, &head, magic));
        }
        if matches!(magic, 0xCAFE_BABE | 0xCAFE_BABF) {
            return Ok(detect_cafebabe(path, &head, magic));
        }
    }
    if head.starts_with(b"xar!") {
        return Ok(BinaryInfo::new(path, Family::MacPkg));
    }
    // UDIF images end with a 512-byte "koly" trailer
    if tail.starts_with(b"koly") {
        return Ok(BinaryInfo::new(path, Family::MacDmg));
    }
    if head.starts_with(b"#!") {
        let line = head[2..].split(|&b| b == b'\n').next().unwrap_or_default();
        let mut info = BinaryInfo::new(path, Family::Script);
        info.details.interpreter = Some(String::from_utf8_lossy(line).trim().to_string());
        return Ok(info);
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
    if matches!(ext.as_deref(), Some("bat" | "cmd")) {
        return Ok(BinaryInfo::new(path, Family::WinBatch));
    }
    Ok(BinaryInfo::new(path, Family::Unknown))
}

fn read_tail(file: &mut File, size: u64) -> io::Result<Vec<u8>> {
    let len = file.seek(SeekFrom::End(0))?;
    if len < size {
        return Ok(Vec::new());
    }
    file.seek(SeekFrom::End(-(size as i64)))?;
    let mut tail = vec![0; size as usize];
    file.read_exact(&mut tail)?;
    Ok(tail)
}

fn detect_elf(path: &Path, head: &[u8]) -> BinaryInfo {
    if head.len() < 20 {
        return BinaryInfo::with_error(path, Family::Unknown, "truncated ELF");
    }
    let little = head[5] == 1;
    let machine = u16_at(head, 18, little).unwrap_or(0);
    let etype = u16_at(head, 16, little).unwrap_or(0);
    let mut info = BinaryInfo::new(path, Family::Elf);
    info.archs.push(Arch::from_elf(machine));
    info.details.bits = match head[4] {
        1 => Some(32),
        2 => Some(64),
        _ => None,
    };
    info.details.endian = Some(if little { "little" } else { "big" });
    info.details.elf_type = Some(match etype {
        2 => "exec".to_string(),
        3 => "dyn".to_string(),
        t => t.to_string(),
    });
    info
}

fn detect_mz(path: &Path, head: &[u8], file: &mut File) -> io::Result<BinaryInfo> {
    let Some(pe_off) = u32_at(head, 0x3C, true).map(|o| o as usize) else {
        return Ok(BinaryInfo::new(path, Family::Dos));
    };
    let mut pe_head =
        if pe_off + 24 <= head.len() { head[pe_off..head.len().min(pe_off + 0x100)].to_vec() } else { Vec::new() };
    if !pe_head.starts_with(b"PE\0\0") {
        // e_lfanew beyond the first block: read again at that offset
        file.seek(SeekFrom::Start(pe_off as u64))?;
        pe_head.clear();
        file.take(0x100).read_to_end(&mut pe_head)?;
    }
    if !pe_head.starts_with(b"PE\0\0") || pe_head.len() < 24 {
        return Ok(BinaryInfo::new(path, Family::Dos));
    }

    let machine = u16_at(&pe_head, 4, true).unwrap_or(0);
    let characteristics = u16_at(&pe_head, 22, true).unwrap_or(0);
    let opt = &pe_head[24..];
    let mut info = BinaryInfo::new(path, Family::Pe);
    info.archs.push(Arch::from_pe(machine));
    info.details.dll = Some(characteristics & 0x2000 != 0);
    if let Some(opt_magic) = u16_at(opt, 0, true) {
        info.details.format = Some(match opt_magic {
            0x10B => "PE32".to_string(),
            0x20B => "PE32+".to_string(),
            m => format!("{m:#x}"),
        });
        if let Some(sub) = u16_at(opt, 68, true) {
            info.details.subsystem = Some(match sub {
                1 => "native".to_string(),
                2 => "gui".to_string(),
                3 => "console".to_string(),
                9 => "wince-gui".to_string(),
                10 => "efi".to_string(),
                s => s.to_string(),
            });
        }
        // Data directory 14 = CLR runtime header (.NET)
        let clr = if opt_magic == 0x10B { 96 } else { 112 } + 14 * 8;
        if let (Some(rva), Some(size)) = (u32_at(opt, clr, true), u32_at(opt, clr + 4, true)) {
            info.details.dotnet = Some(rva != 0 && size != 0);
        }
    }
    Ok(info)
}

fn detect_macho(path: &Path, head: &[u8], magic: u32) -> BinaryInfo {
    let little = !matches!(magic, 0xFEED_FACE | 0xFEED_FACF);
    let (Some(cpu), Some(filetype)) = (u32_at(head, 4, little), u32_at(head, 12, little)) else {
        return BinaryInfo::with_error(path, Family::Unknown, "truncated Mach-O");
    };
    let mut info = BinaryInfo::new(path, Family::Macho);
    info.archs.push(Arch::from_macho(cpu));
    info.details.bits = Some(if matches!(magic, 0xFEED_FACF | 0xCFFA_EDFE) { 64 } else { 32 });
    info.details.filetype = Some(match filetype {
        2 => "exec".to_string(),
        6 => "dylib".to_string(),
        8 => "bundle".to_string(),
        t => t.to_string(),
    });
    info
}

fn detect_cafebabe(path: &Path, head: &[u8], magic: u32) -> BinaryInfo {
    // 0xCAFEBABE is shared by Mach-O universal binaries and Java classes.
    // Universal binaries continue with nfat_arch (a small number); Java
    // classes with minor/major version, where major >= 45, so that 32-bit
    // field is always >= 45.
    let n = u32_at(head, 4, false).unwrap_or(0);
    if magic == 0xCAFE_BABE && n >= 30 {
        return BinaryInfo::new(path, Family::JavaClass);
    }
    let entry = if magic == 0xCAFE_BABF { 32 } else { 20 };
    let mut info = BinaryInfo::new(path, Family::MachoFat);
    for i in 0..n.min(16) as usize {
        match u32_at(head, 8 + i * entry, false) {
            Some(cpu) => info.archs.push(Arch::from_macho(cpu)),
            None => break,
        }
    }
    info.details.slices = Some(n);
    info
}

fn detect_app_bundle(path: &Path) -> BinaryInfo {
    let plist_path = path.join("Contents").join("Info.plist");
    if !plist_path.is_file() {
        return BinaryInfo::with_error(path, Family::Unknown, "directory without Contents/Info.plist");
    }
    let plist = match plist::Value::from_file(&plist_path) {
        Ok(v) => v,
        Err(e) => return BinaryInfo::with_error(path, Family::Unknown, &format!("unreadable Info.plist: {e}")),
    };
    let dict = plist.as_dictionary();
    let string = |key: &str| dict.and_then(|d| d.get(key)).and_then(|v| v.as_string()).map(str::to_string);

    let exe_name = string("CFBundleExecutable")
        .unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    let exe = path.join("Contents").join("MacOS").join(exe_name);
    let mut info = BinaryInfo::new(path, Family::MacApp);
    if exe.is_file() {
        if let Ok(inner) = detect(&exe) {
            info.archs = inner.archs;
        }
    }
    info.details.bundle_id = string("CFBundleIdentifier");
    info.details.name = string("CFBundleName");
    info.executable = Some(exe);
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn elf_x86_64() {
        let t = Tmp::new();
        let info = detect(&t.write("prog", &elf(0x3E, 2))).unwrap();
        assert_eq!((info.family, info.arch(), info.details.bits), (Family::Elf, Some(&Arch::X86_64), Some(64)));
    }

    #[test]
    fn elf_aarch64() {
        let t = Tmp::new();
        assert_eq!(detect(&t.write("prog", &elf(0xB7, 2))).unwrap().arch(), Some(&Arch::Aarch64));
    }

    #[test]
    fn pe64_gui() {
        let t = Tmp::new();
        let info = detect(&t.write("app.exe", &pe(0x8664, PeOpts::default()))).unwrap();
        assert_eq!((info.family, info.arch()), (Family::Pe, Some(&Arch::X86_64)));
        assert_eq!(info.details.format.as_deref(), Some("PE32+"));
        assert_eq!(info.details.subsystem.as_deref(), Some("gui"));
        assert_eq!(info.details.dotnet, Some(false));
    }

    #[test]
    fn pe32_console_dotnet() {
        let t = Tmp::new();
        let opts = PeOpts { pe32plus: false, dotnet: true, subsystem: 3, ..PeOpts::default() };
        let info = detect(&t.write("tool.exe", &pe(0x14C, opts))).unwrap();
        assert_eq!(info.arch(), Some(&Arch::I386));
        assert_eq!(info.details.format.as_deref(), Some("PE32"));
        assert_eq!(info.details.subsystem.as_deref(), Some("console"));
        assert_eq!(info.details.dotnet, Some(true));
    }

    #[test]
    fn pe_dll() {
        let t = Tmp::new();
        let info = detect(&t.write("lib.dll", &pe(0x8664, PeOpts { dll: true, ..PeOpts::default() }))).unwrap();
        assert_eq!(info.details.dll, Some(true));
    }

    #[test]
    fn extension_is_ignored() {
        // A renamed PE is still a PE
        let t = Tmp::new();
        assert_eq!(detect(&t.write("photo.jpg", &pe(0x8664, PeOpts::default()))).unwrap().family, Family::Pe);
    }

    #[test]
    fn pe_header_beyond_first_block() {
        let t = Tmp::new();
        let mut data = vec![0u8; 8192];
        data[..2].copy_from_slice(b"MZ");
        data[0x3C..0x40].copy_from_slice(&5000u32.to_le_bytes());
        let image = pe(0x8664, PeOpts::default());
        let pe_part = &image[0x80..];
        data[5000..5000 + pe_part.len()].copy_from_slice(pe_part);
        let info = detect(&t.write("big.exe", &data)).unwrap();
        assert_eq!((info.family, info.arch()), (Family::Pe, Some(&Arch::X86_64)));
    }

    #[test]
    fn dos() {
        let t = Tmp::new();
        let mut data = b"MZ".to_vec();
        data.resize(102, 0);
        assert_eq!(detect(&t.write("game.exe", &data)).unwrap().family, Family::Dos);
    }

    #[test]
    fn msi() {
        let t = Tmp::new();
        let mut data = MSI_MAGIC.to_vec();
        data.resize(16, 0);
        assert_eq!(detect(&t.write("setup.msi", &data)).unwrap().family, Family::Msi);
    }

    #[test]
    fn macho_arm64() {
        let t = Tmp::new();
        let info = detect(&t.write("prog", &macho(0x0100_000C))).unwrap();
        assert_eq!((info.family, info.arch()), (Family::Macho, Some(&Arch::Aarch64)));
    }

    #[test]
    fn fat_macho() {
        let t = Tmp::new();
        let info = detect(&t.write("prog", &fat(&[0x0100_0007, 0x0100_000C]))).unwrap();
        assert_eq!(info.family, Family::MachoFat);
        assert_eq!(info.archs, vec![Arch::X86_64, Arch::Aarch64]);
    }

    #[test]
    fn java_class_not_confused_with_fat_macho() {
        let t = Tmp::new();
        let mut data = 0xCAFE_BABEu32.to_be_bytes().to_vec();
        data.extend_from_slice(&0u16.to_be_bytes());
        data.extend_from_slice(&61u16.to_be_bytes());
        data.resize(16, 0);
        assert_eq!(detect(&t.write("Main.class", &data)).unwrap().family, Family::JavaClass);
    }

    #[test]
    fn script() {
        let t = Tmp::new();
        let info = detect(&t.write("run.sh", b"#!/bin/sh\necho hi\n")).unwrap();
        assert_eq!(info.family, Family::Script);
        assert_eq!(info.details.interpreter.as_deref(), Some("/bin/sh"));
    }

    #[test]
    fn batch() {
        let t = Tmp::new();
        assert_eq!(detect(&t.write("go.BAT", b"@echo off\r\n")).unwrap().family, Family::WinBatch);
    }

    #[test]
    fn dmg_by_koly_trailer() {
        let t = Tmp::new();
        assert_eq!(detect(&t.write("App.dmg", &dmg())).unwrap().family, Family::MacDmg);
    }

    #[test]
    fn pkg_is_xar() {
        let t = Tmp::new();
        assert_eq!(detect(&t.write("Setup.pkg", &pkg())).unwrap().family, Family::MacPkg);
    }

    #[test]
    fn app_bundle() {
        let t = Tmp::new();
        let app = t.make_app("", "Hello", "com.example.hello", false);
        let info = detect(&app).unwrap();
        assert_eq!((info.family, info.arch()), (Family::MacApp, Some(&Arch::X86_64)));
        assert!(info.executable.unwrap().ends_with("Contents/MacOS/Hello"));
        assert_eq!(info.details.bundle_id.as_deref(), Some("com.example.hello"));
    }

    #[test]
    fn directory_without_plist() {
        let t = Tmp::new();
        std::fs::create_dir_all(t.path("Empty.app")).unwrap();
        assert_eq!(detect(&t.path("Empty.app")).unwrap().family, Family::Unknown);
    }

    #[test]
    fn unknown() {
        let t = Tmp::new();
        assert_eq!(detect(&t.write("x", b"hello")).unwrap().family, Family::Unknown);
    }
}
