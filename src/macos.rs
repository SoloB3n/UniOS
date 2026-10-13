//! macOS containers: .dmg disk images, .app bundles and .icns icons.
//!
//! .dmg images are not mounted (that would need root and APFS/HFS+ support
//! in the kernel): they are extracted with 7-Zip into a per-user cache, from
//! which the contained .app is run or installed.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::host::Host;
use crate::paths::Dirs;

/// Something that can unpack an archive into a directory.
pub trait Extractor {
    fn extract(&self, archive: &Path, dest: &Path) -> Result<()>;
}

/// 7-Zip (`7zz` or `7z`: both read DMG, HFS+ and APFS; `7za`/`7zr` do not).
pub struct SevenZip {
    bin: Option<PathBuf>,
}

impl SevenZip {
    pub fn from_host(host: &dyn Host) -> SevenZip {
        SevenZip { bin: host.which_any(&["7zz", "7z"]) }
    }
}

impl Extractor for SevenZip {
    fn extract(&self, archive: &Path, dest: &Path) -> Result<()> {
        let bin =
            self.bin.as_ref().ok_or_else(|| Error::Mac("7-Zip is not installed: cannot open .dmg images".into()))?;
        let mut out_flag = std::ffi::OsString::from("-o");
        out_flag.push(dest);
        let output = Command::new(bin)
            .args(["x".as_ref(), "-y".as_ref(), out_flag.as_os_str(), archive.as_os_str()])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr = stderr.trim();
            let tail = &stderr[stderr.char_indices().rev().nth(199).map_or(0, |(i, _)| i)..];
            let name = archive.file_name().unwrap_or_default().to_string_lossy();
            return Err(Error::Mac(format!("extracting {name} failed: {tail}")));
        }
        Ok(())
    }
}

fn cache_key(path: &Path) -> Result<String> {
    let meta = fs::metadata(path)?;
    let abs = std::path::absolute(path)?;
    let mtime_ns = meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128;
    let raw = format!("{}:{}:{mtime_ns}", abs.display(), meta.len());
    let digest: String = Sha256::digest(raw.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
    let name = path.file_stem().unwrap_or_default().to_string_lossy();
    Ok(format!("{name}-{}", &digest[..12]))
}

const VOLUME_IMAGES: [&str; 4] = [".hfs", ".hfsx", ".apfs", ".img"];
const COMPLETE_MARKER: &str = ".uos-complete";

/// Extract `path` into the cache (only once) and return the directory.
pub fn extract_dmg(path: &Path, dirs: &Dirs, extractor: &dyn Extractor) -> Result<PathBuf> {
    let dest = dirs.dmg_cache().join(cache_key(path)?);
    if dest.join(COMPLETE_MARKER).exists() {
        return Ok(dest);
    }
    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    fs::create_dir_all(&dest)?;
    extractor.extract(path, &dest)?;

    // Some 7-Zip versions extract the partition as an .hfs/.apfs file instead
    // of its contents: in that case it has to be extracted a second time.
    let entries: Vec<PathBuf> = fs::read_dir(&dest)?
        .filter_map(|e| e.ok())
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    if let [inner] = entries.as_slice() {
        let lower = inner.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        if VOLUME_IMAGES.iter().any(|ext| lower.ends_with(ext)) {
            extractor.extract(inner, &dest)?;
            fs::remove_file(inner)?;
        }
    }

    fs::write(dest.join(COMPLETE_MARKER), b"")?;
    Ok(dest)
}

fn sorted_entries(dir: &Path) -> Vec<fs::DirEntry> {
    let mut entries: Vec<fs::DirEntry> =
        fs::read_dir(dir).map(|r| r.filter_map(|e| e.ok()).collect()).unwrap_or_default();
    entries.sort_by_key(|e| e.file_name());
    entries
}

/// The .app bundles and .pkg packages found under `root`.
///
/// Does not descend into .app bundles (which contain helper .apps of their
/// own) and ignores symbolic links to directories, such as the link to
/// /Applications found in most DMGs.
pub fn find_contents(root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let (mut apps, mut pkgs) = (Vec::new(), Vec::new());
    walk(root, &mut apps, &mut pkgs);
    (apps, pkgs)
}

fn walk(dir: &Path, apps: &mut Vec<PathBuf>, pkgs: &mut Vec<PathBuf>) {
    let mut subdirs = Vec::new();
    let mut file_pkgs = Vec::new();
    for entry in sorted_entries(dir) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_link = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        if path.is_dir() {
            if is_link || name.starts_with('.') {
                continue;
            }
            if name.ends_with(".app") {
                apps.push(path);
            } else if name.ends_with(".pkg") {
                // "Bundle" packages are directories
                pkgs.push(path);
            } else {
                subdirs.push(path);
            }
        } else if name.ends_with(".pkg") || name.ends_with(".mpkg") {
            file_pkgs.push(path);
        }
    }
    pkgs.extend(file_pkgs);
    for sub in subdirs {
        walk(&sub, apps, pkgs);
    }
}

/// Extract the DMG and return the only .app or .pkg it contains.
pub fn resolve_dmg(path: &Path, dirs: &Dirs, extractor: &dyn Extractor) -> Result<PathBuf> {
    let (apps, pkgs) = find_contents(&extract_dmg(path, dirs, extractor)?);
    let candidates = if apps.is_empty() { pkgs } else { apps };
    match candidates.as_slice() {
        [] => Err(Error::Mac("the .dmg image contains no applications or .pkg packages".into())),
        [one] => Ok(one.clone()),
        many => {
            let names: Vec<String> =
                many.iter().map(|c| c.file_name().unwrap_or_default().to_string_lossy().into_owned()).collect();
            Err(Error::Mac(format!(
                "the image contains several programs ({}): run one with uos run <path>",
                names.join(", ")
            )))
        }
    }
}

// --- Icons -------------------------------------------------------------------

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

/// The largest PNG in an .icns file (modern icons: ic07..ic14).
pub fn icns_largest_png(data: &[u8]) -> Option<&[u8]> {
    if data.get(..4)? != b"icns" {
        return None;
    }
    let declared = u32::from_be_bytes(data.get(4..8)?.try_into().ok()?) as usize;
    let end = data.len().min(declared);
    let mut best: Option<(u32, &[u8])> = None;
    let mut off = 8;
    while off + 8 <= end {
        let size = u32::from_be_bytes(data[off + 4..off + 8].try_into().ok()?) as usize;
        if size < 8 {
            break;
        }
        let chunk = &data[off + 8..end.min(off + size)];
        if chunk.starts_with(PNG_MAGIC) && chunk.len() >= 24 {
            let width = u32::from_be_bytes(chunk[16..20].try_into().ok()?);
            if best.is_none_or(|(w, _)| width > w) {
                best = Some((width, chunk));
            }
        }
        off += size;
    }
    best.map(|(_, png)| png)
}

/// PNG of an .app's icon, if it can be found in Contents/Resources.
pub fn bundle_icon(app: &Path) -> Option<Vec<u8>> {
    let contents = app.join("Contents");
    let plist = plist::Value::from_file(contents.join("Info.plist")).ok()?;
    let mut name = plist.as_dictionary()?.get("CFBundleIconFile")?.as_string()?.to_string();
    if name.is_empty() {
        return None;
    }
    if !name.ends_with(".icns") {
        name.push_str(".icns");
    }
    let data = fs::read(contents.join("Resources").join(name)).ok()?;
    icns_largest_png(&data).map(<[u8]>::to_vec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use std::cell::RefCell;

    /// Fake 7-Zip: "extracts" by calling `populate(dest, archive)`.
    struct FakeExtractor<F: Fn(&Path, &Path)> {
        populate: F,
        calls: RefCell<Vec<PathBuf>>,
    }

    impl<F: Fn(&Path, &Path)> Extractor for FakeExtractor<F> {
        fn extract(&self, archive: &Path, dest: &Path) -> Result<()> {
            self.calls.borrow_mut().push(archive.to_path_buf());
            (self.populate)(dest, archive);
            Ok(())
        }
    }

    fn fake<F: Fn(&Path, &Path)>(populate: F) -> FakeExtractor<F> {
        FakeExtractor { populate, calls: RefCell::new(Vec::new()) }
    }

    #[test]
    fn resolves_single_app_and_ignores_applications_symlink() {
        let t = Tmp::new();
        let dmg_path = t.write("Hello.dmg", &dmg());
        let ex = fake(|dest, _| {
            let vol = dest.join("Hello");
            fs::create_dir_all(&vol).unwrap();
            make_app_in(&vol, "Hello", "com.example.hello", false);
            std::os::unix::fs::symlink("/Applications", vol.join("Applications")).unwrap();
        });
        let app = resolve_dmg(&dmg_path, &t.dirs, &ex).unwrap();
        assert!(app.ends_with("Hello/Hello.app"));
        assert!(app.starts_with(t.dirs.dmg_cache()));
        assert_eq!(ex.calls.borrow().len(), 1);
    }

    #[test]
    fn nested_hfs_partition_is_extracted_again() {
        let t = Tmp::new();
        let dmg_path = t.write("Hello.dmg", &dmg());
        let ex = fake(|dest, archive| {
            if archive.extension().is_some_and(|e| e == "dmg") {
                fs::write(dest.join("4.hfs"), b"").unwrap();
            } else {
                make_app_in(dest, "Hello", "com.example.hello", false);
            }
        });
        let app = resolve_dmg(&dmg_path, &t.dirs, &ex).unwrap();
        assert_eq!(ex.calls.borrow().len(), 2);
        assert!(app.ends_with("Hello.app"));
        assert!(!app.parent().unwrap().join("4.hfs").exists());
    }

    #[test]
    fn helper_apps_inside_bundle_are_not_counted() {
        let t = Tmp::new();
        let dmg_path = t.write("Hello.dmg", &dmg());
        let ex = fake(|dest, _| {
            let app = make_app_in(dest, "Hello", "com.example.hello", false);
            make_app_in(&app.join("Contents/Helpers"), "Helper", "com.example.helper", false);
        });
        assert!(resolve_dmg(&dmg_path, &t.dirs, &ex).unwrap().ends_with("Hello.app"));
    }

    #[test]
    fn extraction_is_cached() {
        let t = Tmp::new();
        let dmg_path = t.write("Hello.dmg", &dmg());
        let ex = fake(|dest, _| {
            make_app_in(dest, "Hello", "com.example.hello", false);
        });
        resolve_dmg(&dmg_path, &t.dirs, &ex).unwrap();
        resolve_dmg(&dmg_path, &t.dirs, &ex).unwrap();
        assert_eq!(ex.calls.borrow().len(), 1);
    }

    #[test]
    fn multiple_apps_is_an_error() {
        let t = Tmp::new();
        let dmg_path = t.write("Hello.dmg", &dmg());
        let ex = fake(|dest, _| {
            make_app_in(dest, "A", "a", false);
            make_app_in(dest, "B", "b", false);
        });
        let err = resolve_dmg(&dmg_path, &t.dirs, &ex).unwrap_err().to_string();
        assert!(err.contains("several programs") && err.contains("A.app, B.app"), "{err}");
    }

    #[test]
    fn pkg_inside_dmg() {
        let t = Tmp::new();
        let dmg_path = t.write("Tool.dmg", &dmg());
        let ex = fake(|dest, _| {
            fs::write(dest.join("Tool.pkg"), pkg()).unwrap();
        });
        assert!(resolve_dmg(&dmg_path, &t.dirs, &ex).unwrap().ends_with("Tool.pkg"));
    }

    #[test]
    fn missing_7zip() {
        let t = Tmp::new();
        let dmg_path = t.write("Hello.dmg", &dmg());
        let seven = SevenZip::from_host(&FakeHost::new(crate::detect::Arch::X86_64, &[]));
        let err = resolve_dmg(&dmg_path, &t.dirs, &seven).unwrap_err().to_string();
        assert!(err.contains("7-Zip"), "{err}");
    }

    #[test]
    fn largest_png_is_chosen() {
        let data = icns(&[(b"ic07", png(128)), (b"ic10", png(1024)), (b"ic08", png(256))]);
        let out = icns_largest_png(&data).unwrap();
        assert_eq!(u32::from_be_bytes(out[16..20].try_into().unwrap()), 1024);
    }

    #[test]
    fn legacy_only_icns() {
        assert!(icns_largest_png(&icns(&[(b"is32", vec![b'x'; 40])])).is_none());
    }

    #[test]
    fn not_icns() {
        assert!(icns_largest_png(b"garbage").is_none());
        assert!(icns_largest_png(b"icns").is_none());
    }

    #[test]
    fn bundle_icon_from_resources() {
        let t = Tmp::new();
        let app = t.make_app("", "Hello", "com.example.hello", true);
        assert!(bundle_icon(&app).unwrap().starts_with(PNG_MAGIC));
    }
}
