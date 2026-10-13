import hashlib
import os
import plistlib
import struct
import subprocess
import unittest
from unittest import mock

from uos import cli, macos, profiles, runners
from uos import detect as d
from uos import install as inst

from .test_uos import Fixture, elf, fake_which, macho, pe


def read(path: str, mode: str = "r"):
    with open(path, mode) as f:
        return f.read()


def png(width: int) -> bytes:
    return b"\x89PNG\r\n\x1a\n" + struct.pack(">I4sII", 13, b"IHDR", width, width) + b"\0" * 8


def icns(*chunks: tuple[bytes, bytes]) -> bytes:
    body = b"".join(kind + struct.pack(">I", len(data) + 8) + data for kind, data in chunks)
    return b"icns" + struct.pack(">I", len(body) + 8) + body


class AppFixture(Fixture):
    def make_app(self, root: str, name="Hello", bundle_id="com.example.hello", icon=True) -> str:
        app = os.path.join(root, f"{name}.app")
        plist = {"CFBundleExecutable": name, "CFBundleIdentifier": bundle_id, "CFBundleName": name}
        if icon:
            plist["CFBundleIconFile"] = "AppIcon"
        rel = os.path.relpath(app, self.tmp.name)
        self.write(f"{rel}/Contents/MacOS/{name}", macho(0x01000007))
        self.write(f"{rel}/Contents/Info.plist", plistlib.dumps(plist))
        if icon:
            self.write(f"{rel}/Contents/Resources/AppIcon.icns",
                       icns((b"ic07", png(128)), (b"ic10", png(1024)), (b"is32", b"x" * 20)))
        return app


class ContainerDetectTests(Fixture):
    def test_dmg_by_koly_trailer(self):
        data = b"\0" * 2048 + b"koly" + b"\0" * 508
        self.assertEqual(d.detect(self.write("App.dmg", data)).family, d.MAC_DMG)

    def test_pkg_is_xar(self):
        self.assertEqual(d.detect(self.write("Setup.pkg", b"xar!" + b"\0" * 24)).family, d.MAC_PKG)

    def test_pkg_runs_darling_installer(self):
        info = d.detect(self.write("Setup.pkg", b"xar!" + b"\0" * 24))
        with mock.patch.object(runners.shutil, "which", side_effect=fake_which("darling")):
            p = runners.plan(info, [])
        self.assertEqual(p.runner, "darling-installer")
        self.assertEqual(p.argv[2:4], ["installer", "-pkg"])
        self.assertTrue(p.argv[4].startswith("/Volumes/SystemRoot/"))


class IcnsTests(AppFixture):
    def test_largest_png_is_chosen(self):
        data = icns((b"ic07", png(128)), (b"ic10", png(1024)), (b"ic08", png(256)))
        out = macos.icns_largest_png(data)
        self.assertEqual(struct.unpack(">I", out[16:20])[0], 1024)

    def test_legacy_only_icns(self):
        self.assertIsNone(macos.icns_largest_png(icns((b"is32", b"x" * 40))))

    def test_not_icns(self):
        self.assertIsNone(macos.icns_largest_png(b"garbage"))

    def test_bundle_icon(self):
        app = self.make_app(self.tmp.name)
        self.assertTrue(macos.bundle_icon(app).startswith(b"\x89PNG"))


class DmgTests(AppFixture):
    def run_extract(self, populate):
        dmg = self.write("Hello.dmg", b"\0" * 1024 + b"koly" + b"\0" * 508)
        calls = []

        def run(argv, **_kw):
            calls.append(argv)
            dest = next(a[2:] for a in argv if a.startswith("-o"))
            populate(dest, argv[-1])
            return subprocess.CompletedProcess(argv, 0, "", "")

        with mock.patch.object(macos.shutil, "which", side_effect=fake_which("7z")), \
             mock.patch.object(macos.subprocess, "run", side_effect=run):
            return dmg, calls, macos.resolve_dmg(dmg)

    def test_resolves_single_app_and_ignores_applications_symlink(self):
        def populate(dest, _archive):
            vol = os.path.join(dest, "Hello")
            os.makedirs(vol)
            self.make_app(vol)
            os.symlink("/Applications", os.path.join(vol, "Applications"))

        _dmg, calls, app = self.run_extract(populate)
        self.assertTrue(app.endswith("Hello/Hello.app"))
        self.assertEqual(len(calls), 1)
        self.assertTrue(app.startswith(macos.cache_dir()))

    def test_nested_hfs_partition_is_extracted_again(self):
        def populate(dest, archive):
            if archive.endswith(".dmg"):
                open(os.path.join(dest, "4.hfs"), "wb").close()
            else:
                self.make_app(dest)

        _dmg, calls, app = self.run_extract(populate)
        self.assertEqual(len(calls), 2)
        self.assertTrue(app.endswith("Hello.app"))
        self.assertFalse(os.path.exists(os.path.join(os.path.dirname(app), "4.hfs")))

    def test_helper_apps_inside_bundle_are_not_counted(self):
        def populate(dest, _archive):
            app = self.make_app(dest)
            self.make_app(os.path.join(app, "Contents", "Helpers"), name="Helper")

        _dmg, _calls, app = self.run_extract(populate)
        self.assertTrue(app.endswith("Hello.app"))

    def test_extraction_is_cached(self):
        def populate(dest, _archive):
            self.make_app(dest)

        dmg, calls, _ = self.run_extract(populate)
        with mock.patch.object(macos.shutil, "which", side_effect=fake_which("7z")), \
             mock.patch.object(macos.subprocess, "run") as run:
            macos.resolve_dmg(dmg)
        run.assert_not_called()

    def test_multiple_apps_is_an_error(self):
        def populate(dest, _archive):
            self.make_app(dest, name="A", bundle_id="a")
            self.make_app(dest, name="B", bundle_id="b")

        with self.assertRaisesRegex(macos.MacError, "più programmi"):
            self.run_extract(populate)

    def test_missing_7zip(self):
        dmg = self.write("Hello.dmg", b"\0" * 1024 + b"koly" + b"\0" * 508)
        with mock.patch.object(macos.shutil, "which", return_value=None):
            with self.assertRaisesRegex(macos.MacError, "7-Zip"):
                macos.resolve_dmg(dmg)


class ProfileTests(Fixture):
    def profile(self, name: str, text: str) -> str:
        path = os.path.join(profiles.user_dir(), name + ".toml")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(text)
        return path

    def test_match_by_name_and_apply(self):
        self.profile("npp", '''
[match]
name = "Notepad++*.EXE"
[run]
prefix = "editor"
args = ["-multiInst"]
env = { DXVK_HUD = "fps" }
dll_overrides = { d3d11 = "n,b", dxgi = "n,b" }
''')
        info = d.detect(self.write("notepad++_8.6.exe", pe(0x8664)))
        prof = profiles.find(info)
        self.assertEqual(prof.name, "npp")
        with mock.patch.object(runners, "host_arch", return_value=d.X86_64), \
             mock.patch.object(runners.shutil, "which", side_effect=fake_which("wine")):
            p = runners.plan(info, ["file.txt"], profile=prof)
        self.assertEqual(p.argv[-2:], ["-multiInst", "file.txt"])
        self.assertTrue(p.env["WINEPREFIX"].endswith("prefixes/editor"))
        self.assertEqual(p.env["WINEDLLOVERRIDES"], "d3d11=n,b;dxgi=n,b")
        self.assertEqual(p.env["DXVK_HUD"], "fps")
        self.assertEqual(p.profile, "npp")

    def test_match_by_sha256(self):
        path = self.write("game.exe", pe(0x8664))
        digest = hashlib.sha256(read(path, "rb")).hexdigest()
        self.profile("game", f'[match]\nsha256 = "{digest.upper()}"\n')
        self.assertEqual(profiles.find(d.detect(path)).name, "game")
        self.assertIsNone(profiles.find(d.detect(self.write("other.exe", pe(0x14C)))))

    def test_profile_without_match_never_applies(self):
        self.profile("empty", "[run]\nisolate = true\n")
        self.assertIsNone(profiles.find(d.detect(self.write("x.exe", pe(0x8664)))))

    def test_user_profile_wins_over_system(self):
        os.makedirs(profiles.SYSTEM_DIR)
        with open(os.path.join(profiles.SYSTEM_DIR, "a.toml"), "w") as f:
            f.write('[match]\nname = "*.exe"\n')
        self.profile("z", '[match]\nname = "*.exe"\n')
        self.assertEqual(profiles.find(d.detect(self.write("x.exe", pe(0x8664)))).name, "z")

    def test_invalid_key_is_reported(self):
        self.profile("bad", '[match]\nname = "*"\n[run]\nprefx = "oops"\n')
        with self.assertRaisesRegex(profiles.ProfileError, "prefx"):
            profiles.find(d.detect(self.write("x.exe", pe(0x8664))))

    def test_wrong_type_is_reported(self):
        self.profile("bad", '[match]\nname = "*"\n[run]\nargs = "-x"\n')
        with self.assertRaisesRegex(profiles.ProfileError, "list"):
            profiles.find(d.detect(self.write("x.exe", pe(0x8664))))

    def test_missing_custom_wine(self):
        self.profile("p", '[match]\nname = "*"\n[run]\nwine = "/nope/wine"\n')
        info = d.detect(self.write("x.exe", pe(0x8664)))
        with mock.patch.object(runners, "host_arch", return_value=d.X86_64):
            with self.assertRaisesRegex(runners.RunnerError, "/nope/wine"):
                runners.plan(info, [], profile=profiles.find(info))

    def test_cli_new_profile_template_is_loadable(self):
        exe = self.write("My Game.exe", pe(0x8664))
        with mock.patch("builtins.print"):
            self.assertEqual(cli.main(["profile", "--new", exe]), 0)
        self.assertEqual(profiles.find(d.detect(exe)).name, "my_game")

    def test_cli_forced_profile(self):
        self.profile("forced", '[run]\nargs = ["-safe"]\n')
        exe = self.write("x.exe", pe(0x8664))
        with mock.patch.object(runners.shutil, "which", side_effect=fake_which("wine")), \
             mock.patch("builtins.print") as out:
            cli.main(["run", "--dry-run", "--profile", "forced", exe])
        self.assertIn("profilo forced", out.call_args[0][0])
        self.assertIn("-safe", out.call_args[0][0])


class InstallTests(AppFixture):
    def test_install_app_bundle_with_icon(self):
        app = self.make_app(os.path.join(self.tmp.name, "dl"))
        entry = inst.install(app)
        dest = os.path.join(self.home, "Applications", "Hello.app")
        self.assertTrue(os.path.isdir(dest))
        text = read(entry)
        self.assertIn("Name=Hello", text)
        self.assertIn(f"Exec=uos run {dest} %U", text)
        icon = next(l.split("=", 1)[1] for l in text.splitlines() if l.startswith("Icon="))
        self.assertTrue(read(icon, "rb").startswith(b"\x89PNG"))
        self.assertEqual(os.path.basename(entry), "uos-com-example-hello.desktop")

    def test_install_twice_is_refused(self):
        app = self.make_app(os.path.join(self.tmp.name, "dl"))
        inst.install(app)
        with self.assertRaisesRegex(inst.InstallError, "esiste già"):
            inst.install(app)

    def test_install_appimage(self):
        img = self.write("dl/Krita-5.2-x86_64.AppImage", elf(0x3E))
        entry = inst.install(img)
        self.assertIn("Name=Krita-5.2", read(entry))
        self.assertTrue(os.access(os.path.join(self.home, "Applications", os.path.basename(img)), os.X_OK))

    def test_install_exe_returns_installer_plan(self):
        exe = self.write("setup.exe", pe(0x14C))
        with mock.patch.object(runners, "host_arch", return_value=d.X86_64), \
             mock.patch.object(runners.shutil, "which", side_effect=fake_which("wine")):
            p = inst.install(exe)
        self.assertEqual(p.runner, "wine")

    def test_list_and_uninstall(self):
        inst.install(self.make_app(os.path.join(self.tmp.name, "dl")))
        [entry] = inst.installed()
        self.assertEqual(entry["Name"], "Hello")
        removed = inst.uninstall("Hello")
        self.assertEqual(len(removed), 3)  # .app, icona, voce di menu
        self.assertEqual(inst.installed(), [])
        self.assertFalse(os.path.exists(os.path.join(self.home, "Applications", "Hello.app")))

    def test_uninstall_never_deletes_outside_applications(self):
        outside = self.write("important/data.bin", b"x")
        inst.write_desktop_entry("evil", "Evil", outside, "/etc/passwd", "x")
        inst.uninstall("evil")
        self.assertTrue(os.path.exists(outside))
        self.assertTrue(os.path.exists("/etc/passwd"))

    def test_desktop_quote(self):
        self.assertEqual(inst.desktop_quote("/a/b.app"), "/a/b.app")
        self.assertEqual(inst.desktop_quote('/x/My "App" 100%'), '"/x/My \\"App\\" 100%%"')
        self.assertEqual(inst.desktop_quote("/x/$HOME"), '"/x/\\$HOME"')


if __name__ == "__main__":
    unittest.main()
