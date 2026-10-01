"""CPU-only packaging regressions using temporary repositories and fake tools."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


class PackagingTests(unittest.TestCase):
    def test_unsigned_flatpak_writes_both_install_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            repo, source, binaries = (base / name for name in ("repo", "source", "bin"))
            (repo / "flatpak" / "objects").mkdir(parents=True)
            (source / "objects").mkdir(parents=True)
            binaries.mkdir()
            commands = {
                "ostree": "printf '%s\\n' app/ai.localrouter.app/x86_64/stable",
                "flatpak": "exit 0",
            }
            for name, body in commands.items():
                executable = binaries / name
                executable.write_text(f"#!/bin/sh\n{body}\n")
                executable.chmod(0o755)
            environment = os.environ.copy()
            environment.pop("APT_GPG_KEY_ID", None)
            environment["PATH"] = f"{binaries}{os.pathsep}{environment['PATH']}"
            result = subprocess.run(
                ["bash", str(ROOT / "packaging/linux-repo/build-flatpak-repo.sh"),
                 "--repo-dir", str(repo), "--src-repo", str(source)],
                env=environment, capture_output=True, text=True, timeout=10,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            for suffix in ("flatpakrepo", "flatpakref"):
                contents = (repo / "flatpak" / f"localrouter.{suffix}").read_text()
                self.assertIn("Url=https://packages.localrouter.ai/flatpak", contents)
                self.assertNotIn("GPGKey=", contents)
            self.assertTrue((repo / ".nojekyll").is_file())

    def test_invalid_retention_cannot_modify_repository(self):
        for keep in ("0", "-1", "all", "1.5"):
            with self.subTest(keep=keep), tempfile.TemporaryDirectory() as temporary:
                base = Path(temporary)
                assets, repo = base / "assets", base / "repo"
                assets.mkdir()
                repo.mkdir()
                (assets / "LocalRouter_1.2.3_amd64.deb").write_bytes(b"fixture")
                marker = repo / "keep.txt"
                marker.write_text("existing repository")
                result = subprocess.run(
                    ["bash", str(ROOT / "packaging/linux-repo/build-linux-repo.sh"),
                     "--version", "1.2.3", "--assets-dir", str(assets),
                     "--repo-dir", str(repo), "--keep", keep],
                    capture_output=True, text=True, timeout=10,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("--keep must be a positive integer", result.stderr)
                self.assertEqual(list(repo.iterdir()), [marker])


if __name__ == "__main__":
    unittest.main()
