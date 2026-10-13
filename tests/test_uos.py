import os
import plistlib
import struct
import tempfile
import unittest
from unittest import mock

from uos import cli
from uos import detect as d
from uos import runners


def elf(machine: int, bits: int = 2) -> bytes:
    return b"\x7fELF" + bytes([bits, 1, 1]) + b"\0" * 9 + struct.pack("<HH", 2, machine) + b"\0" * 44


def pe(machine: int, *, dll: bool = False, pe32plus: bool = True, dotnet: bool = False,
       subsystem: int = 2) -> bytes:
    pe_off = 0x80
    mz = bytearray(b"MZ" + b"\0" * (pe_off - 2))
    mz[0x3C:0x40] = struct.pack("<I", pe_off)
    coff = struct.pack("<HHIIIHH", machine, 1, 0, 0, 0, 0xF0, 0x2000 if dll else 0x0102)
    opt = bytearray(0xF0)
    opt[0:2] = struct.pack("<H", 0x20B if pe32plus else 0x10B)
    opt[68:70] = struct.pack("<H", subsystem)
    if dotnet:
        clr = (112 if pe32plus else 96) + 14 * 8
        opt[clr:clr + 8] = struct.pack("<II", 0x2008, 0x48)
    return bytes(mz) + b"PE\0\0" + coff + bytes(opt)


def macho(cpu: int) -> bytes:
    return struct.pack("<IiiIIII", 0xFEEDFACF, cpu, 3, 2, 0, 0, 0) + b"\0" * 4


def fat(*cpus: int) -> bytes:
    data = struct.pack(">II", 0xCAFEBABE, len(cpus))
    for cpu in cpus:
        data += struct.pack(">iiIII", cpu, 3, 0x1000, 0x1000, 12)
    return data


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def write(self, name: str, data: bytes) -> str:
        path = os.path.join(self.tmp.name, name)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data)
        return path


class DetectTests(Fixture):
    def test_elf_x86_64(self):
        info = d.detect(self.write("prog", elf(0x3E)))
        self.assertEqual((info.family, info.arch, info.details["bits"]), (d.ELF, d.X86_64, 64))

    def test_elf_aarch64(self):
        self.assertEqual(d.detect(self.write("prog", elf(0xB7))).arch, d.ARM64)

    def test_pe64_gui(self):
        info = d.detect(self.write("app.exe", pe(0x8664)))
        self.assertEqual((info.family, info.arch), (d.PE, d.X86_64))
        self.assertEqual(info.details["format"], "PE32+")
        self.assertEqual(info.details["subsystem"], "gui")
        self.assertFalse(info.details["dotnet"])

    def test_pe32_console_dotnet(self):
        info = d.detect(self.write("tool.exe", pe(0x14C, pe32plus=False, dotnet=True, subsystem=3)))
        self.assertEqual((info.arch, info.details["format"]), (d.X86, "PE32"))
        self.assertEqual(info.details["subsystem"], "console")
        self.assertTrue(info.details["dotnet"])

    def test_pe_dll(self):
        self.assertTrue(d.detect(self.write("lib.dll", pe(0x8664, dll=True))).details["dll"])

    def test_extension_is_ignored(self):
        # Un PE rinominato resta un PE
        self.assertEqual(d.detect(self.write("photo.jpg", pe(0x8664))).family, d.PE)

    def test_dos(self):
        self.assertEqual(d.detect(self.write("game.exe", b"MZ" + b"\0" * 100)).family, d.DOS)

    def test_msi(self):
        self.assertEqual(d.detect(self.write("setup.msi", bytes.fromhex("d0cf11e0a1b11ae1") + b"\0" * 8)).family, d.MSI)

    def test_macho_arm64(self):
        info = d.detect(self.write("prog", macho(0x0100000C)))
        self.assertEqual((info.family, info.arch), (d.MACHO, d.ARM64))

    def test_fat_macho(self):
        info = d.detect(self.write("prog", fat(0x01000007, 0x0100000C)))
        self.assertEqual(info.family, d.MACHO_FAT)
        self.assertEqual(info.archs, [d.X86_64, d.ARM64])

    def test_java_class_not_confused_with_fat_macho(self):
        data = struct.pack(">IHH", 0xCAFEBABE, 0, 61) + b"\0" * 8
        self.assertEqual(d.detect(self.write("Main.class", data)).family, d.JAVA_CLASS)

    def test_script(self):
        info = d.detect(self.write("run.sh", b"#!/bin/sh\necho hi\n"))
        self.assertEqual((info.family, info.details["interpreter"]), (d.SCRIPT, "/bin/sh"))

    def test_batch(self):
        self.assertEqual(d.detect(self.write("go.bat", b"@echo off\r\n")).family, d.WIN_BATCH)

    def test_app_bundle(self):
        app = os.path.join(self.tmp.name, "Hello.app")
        self.write("Hello.app/Contents/MacOS/HelloBin", macho(0x01000007))
        self.write("Hello.app/Contents/Info.plist", plistlib.dumps(
            {"CFBundleExecutable": "HelloBin", "CFBundleIdentifier": "com.example.hello"},
            fmt=plistlib.FMT_BINARY))
        info = d.detect(app)
        self.assertEqual((info.family, info.arch), (d.MAC_APP, d.X86_64))
        self.assertTrue(info.executable.endswith("Contents/MacOS/HelloBin"))
        self.assertEqual(info.details["bundle_id"], "com.example.hello")

    def test_unknown(self):
        self.assertEqual(d.detect(self.write("x", b"hello")).family, d.UNKNOWN)


def fake_which(*available):
    return lambda name: f"/usr/bin/{name}" if name in available else None


class PlanTests(Fixture):
    def plan(self, data, name="prog", host=d.X86_64, available=(), **kw):
        info = d.detect(self.write(name, data))
        with mock.patch.object(runners, "host_arch", return_value=host), \
             mock.patch.object(runners.shutil, "which", side_effect=fake_which(*available)):
            return runners.plan(info, ["--flag"], **kw)

    def test_native_elf(self):
        p = self.plan(elf(0x3E))
        self.assertEqual(p.runner, "native")
        self.assertEqual(p.argv[1:], ["--flag"])

    def test_i386_elf_runs_natively_on_x86_64(self):
        self.assertEqual(self.plan(elf(0x03, bits=1)).runner, "native")

    def test_x86_elf_on_arm_prefers_box64(self):
        p = self.plan(elf(0x3E), host=d.ARM64, available=("box64", "qemu-x86_64"))
        self.assertEqual((p.runner, p.argv[0]), ("native+emu", "/usr/bin/box64"))

    def test_foreign_elf_without_emulator(self):
        with self.assertRaises(runners.RunnerError):
            self.plan(elf(0xF3))

    def test_windows_exe_uses_wine_with_prefix(self):
        p = self.plan(pe(0x8664), "app.exe", available=("wine",))
        self.assertEqual(p.runner, "wine")
        self.assertEqual(p.argv[0], "/usr/bin/wine")
        self.assertTrue(p.env["WINEPREFIX"].endswith("prefixes/default"))

    def test_isolated_prefix(self):
        p = self.plan(pe(0x8664), "app.exe", available=("wine",), isolate=True)
        self.assertTrue(p.env["WINEPREFIX"].endswith("prefixes/app"))

    def test_windows_exe_on_arm_goes_through_box64(self):
        p = self.plan(pe(0x8664), "app.exe", host=d.ARM64, available=("wine", "box64"))
        self.assertEqual(p.argv[:2], ["/usr/bin/box64", "/usr/bin/wine"])

    def test_msi(self):
        p = self.plan(bytes.fromhex("d0cf11e0a1b11ae1"), "setup.msi", available=("wine",))
        self.assertEqual(p.argv[1:3], ["msiexec", "/i"])

    def test_dll_is_rejected(self):
        with self.assertRaisesRegex(runners.RunnerError, "DLL"):
            self.plan(pe(0x8664, dll=True), "x.dll", available=("wine",))

    def test_missing_wine(self):
        with self.assertRaisesRegex(runners.RunnerError, "Wine"):
            self.plan(pe(0x8664), "app.exe")

    def test_macho_uses_darling_with_translated_path(self):
        p = self.plan(macho(0x01000007), available=("darling",))
        self.assertEqual(p.runner, "darling")
        self.assertEqual(p.argv[1], "shell")
        self.assertTrue(p.argv[2].startswith("/Volumes/SystemRoot/"))

    def test_arm64_only_macho_rejected_on_x86_host(self):
        with self.assertRaisesRegex(runners.RunnerError, "x86_64"):
            self.plan(macho(0x0100000C), available=("darling",))

    def test_universal_macho_accepted_on_x86_host(self):
        self.assertEqual(self.plan(fat(0x0100000C, 0x01000007), available=("darling",)).runner, "darling")

    def test_dos(self):
        p = self.plan(b"MZ" + b"\0" * 100, "game.exe", available=("dosbox",))
        self.assertEqual(p.runner, "dosbox")


class CliTests(Fixture):
    def test_binfmt_style_invocation_keeps_program_args(self):
        path = self.write("app.exe", pe(0x8664))
        ns_seen = {}
        with mock.patch.object(cli, "cmd_run", side_effect=lambda ns: ns_seen.update(vars(ns)) or 0):
            cli.main(["run", "--", path, "-x", "--help"])
        self.assertEqual(ns_seen["program"], path)
        self.assertEqual(ns_seen["args"], ["-x", "--help"])

    def test_dry_run(self):
        path = self.write("prog", elf(0x3E))
        with mock.patch("builtins.print") as out:
            self.assertEqual(cli.main(["run", "--dry-run", path, "a"]), 0)
        self.assertIn("[native]", out.call_args[0][0])

    def test_missing_file(self):
        self.assertEqual(cli.main(["run", "/nonexistent/x.exe"]), 127)


if __name__ == "__main__":
    unittest.main()
