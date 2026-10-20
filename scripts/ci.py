"""Plan CI, package the two binaries, check real execution, and publish GitHub assets.

Python >= 3.11, standard library only. The runtime wrapper does not use this script.
"""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import zipfile
from pathlib import Path
from typing import Any, TypedDict
from urllib.parse import quote

ROOT = Path(__file__).resolve().parent.parent
NAME = "uv_python_warpper"
RUST = "1.96.0"


class Target(TypedDict):
    target: str
    host: str
    runner: str
    musl: bool


TARGETS: list[Target] = [
    {
        "target": "x86_64-unknown-linux-gnu",
        "host": "x86_64-unknown-linux-gnu",
        "runner": "ubuntu-22.04",
        "musl": False,
    },
    {
        "target": "aarch64-unknown-linux-gnu",
        "host": "aarch64-unknown-linux-gnu",
        "runner": "ubuntu-22.04-arm",
        "musl": False,
    },
    {
        "target": "x86_64-pc-windows-msvc",
        "host": "x86_64-pc-windows-msvc",
        "runner": "windows-2025",
        "musl": False,
    },
    {
        "target": "aarch64-pc-windows-msvc",
        "host": "aarch64-pc-windows-msvc",
        "runner": "windows-11-arm",
        "musl": False,
    },
    {
        "target": "x86_64-apple-darwin",
        "host": "x86_64-apple-darwin",
        "runner": "macos-15-intel",
        "musl": False,
    },
    {
        "target": "aarch64-apple-darwin",
        "host": "aarch64-apple-darwin",
        "runner": "macos-14",
        "musl": False,
    },
    {
        "target": "x86_64-unknown-linux-musl",
        "host": "x86_64-unknown-linux-gnu",
        "runner": "ubuntu-22.04",
        "musl": True,
    },
    {
        "target": "aarch64-unknown-linux-musl",
        "host": "aarch64-unknown-linux-gnu",
        "runner": "ubuntu-22.04-arm",
        "musl": True,
    },
]
SEMVER = re.compile(
    r"(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?"
)


def run(args: list[str], **kwargs: Any) -> subprocess.CompletedProcess:
    return subprocess.run(args, check=True, timeout=300, **kwargs)


def git(root: Path, *args: str) -> str:
    return run(
        ["git", "-C", str(root), *args], capture_output=True, text=True
    ).stdout.strip()


def version_at(root: Path) -> str:
    package = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))[
        "package"
    ]
    version = package["version"]
    if package["name"] != NAME or not SEMVER.fullmatch(version):
        raise ValueError(
            "Cargo.toml must name this package and contain a SemVer version"
        )
    locked = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))["package"]
    if not any(p["name"] == NAME and p["version"] == version for p in locked):
        raise ValueError("Cargo.toml and Cargo.lock package versions differ")
    return version


def plan(root: Path, event: str, ref: str, before: str, mode: str) -> dict[str, Any]:
    if mode not in {"auto", "check", "build", "release"}:
        raise ValueError(f"Unknown CI mode: {mode}")
    version = version_at(root)
    publish = mode == "release"
    if publish and (event != "workflow_dispatch" or ref != "refs/heads/main"):
        raise ValueError("Manual releases must run on main using workflow_dispatch")
    if (
        event == "push"
        and ref == "refs/heads/main"
        and mode == "auto"
        and before
        and set(before) != {"0"}
    ):
        if not re.fullmatch(r"[0-9a-f]{40,64}", before):
            raise ValueError("Invalid push before SHA")
        # A missing baseline is an error, rather than permission to publish or skip
        # checks. checkout's full history normally contains the before commit.
        git(root, "cat-file", "-e", f"{before}^{{commit}}")
        files = git(root, "ls-tree", "--name-only", before, "Cargo.toml")
        previous = (
            tomllib.loads(git(root, "show", f"{before}:Cargo.toml"))["package"][
                "version"
            ]
            if files
            else None
        )
        publish = previous != version
    return {
        "version": version,
        "tag": f"v{version}",
        "sha": git(root, "rev-parse", "HEAD"),
        "rust": RUST,
        "build": mode != "check",
        "publish": publish,
        "checks_matrix": {"include": [t for t in TARGETS if not t["musl"]]},
        "build_matrix": {"include": TARGETS},
    }


def write_outputs(values: dict[str, Any]) -> None:
    print(json.dumps(values, indent=2))
    if output := os.environ.get("GITHUB_OUTPUT"):
        with open(output, "a", encoding="utf-8") as stream:
            for key, value in values.items():
                text = (
                    value
                    if isinstance(value, str)
                    else json.dumps(value, separators=(",", ":"))
                )
                if "\n" in text or "\r" in text:
                    raise ValueError("Workflow outputs must be single lines")
                stream.write(f"{key}={text}\n")


def asset_name(target: str) -> str:
    if target not in {t["target"] for t in TARGETS}:
        raise ValueError(f"Unsupported release target: {target}")
    return f"{NAME}-{target}" + (".zip" if "windows" in target else ".tar.gz")


def binary_names(target: str) -> tuple[str, str]:
    return ("python.exe", "pip.exe") if "windows" in target else ("python", "pip")


def package(root: Path, target: str, dist: Path) -> Path:
    name = asset_name(target)
    files = {
        binary: (root / "target" / target / "release" / binary).read_bytes()
        for binary in binary_names(target)
    }
    if not all(files.values()):
        raise ValueError("Cannot package empty binaries")
    files["README.md"] = (root / "README.md").read_bytes()
    files["BUILD.json"] = (
        json.dumps(
            {
                "version": version_at(root),
                "target": target,
                "commit": git(root, "rev-parse", "HEAD"),
            },
            indent=2,
        ).encode()
        + b"\n"
    )
    dist.mkdir(parents=True, exist_ok=True)
    archive = dist / name
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as zipped:
            for filename, data in files.items():
                info = zipfile.ZipInfo(filename)
                info.external_attr = (
                    stat.S_IFREG
                    | (0o755 if filename in binary_names(target) else 0o644)
                ) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                zipped.writestr(info, data)
    else:
        with tarfile.open(archive, "w:gz") as tar:
            for filename, data in files.items():
                info = tarfile.TarInfo(filename)
                info.size = len(data)
                info.mode = 0o755 if filename in binary_names(target) else 0o644
                tar.addfile(info, io.BytesIO(data))
    print(f"Packaged {archive}")
    return archive


def read_archive(
    archive: Path, target: str, version: str, sha: str
) -> dict[str, bytes]:
    expected = {*binary_names(target), "README.md", "BUILD.json"}
    files: dict[str, bytes] = {}
    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as zipped:
            if sorted(zipped.namelist()) != sorted(expected):
                raise ValueError(f"Wrong archive members in {archive.name}")
            for info in zipped.infolist():
                if stat.S_ISLNK(info.external_attr >> 16):
                    raise ValueError("Release archives cannot contain symlinks")
                files[info.filename] = zipped.read(info)
    else:
        with tarfile.open(archive) as tar:
            members = tar.getmembers()
            if sorted(m.name for m in members) != sorted(expected) or not all(
                m.isfile() for m in members
            ):
                raise ValueError(f"Wrong archive members in {archive.name}")
            for member in members:
                if member.name in binary_names(target) and member.mode & 0o111 != 0o111:
                    raise ValueError("Release binaries must be executable")
                stream = tar.extractfile(member)
                assert stream is not None
                files[member.name] = stream.read()
    metadata = json.loads(files["BUILD.json"])
    if metadata != {"version": version, "target": target, "commit": sha}:
        raise ValueError(f"Version, target or source commit mismatch in {archive.name}")
    if not all(files[name] for name in binary_names(target)):
        raise ValueError("Release contains an empty binary")
    return files


def checksums(dist: Path, version: str, sha: str) -> str:
    expected = {asset_name(t["target"]) for t in TARGETS}
    actual = {p.name for p in dist.iterdir()}
    if actual - {"SHA256SUMS"} != expected:
        raise ValueError(
            f"Incomplete or unexpected release assets: missing={expected - actual}, extra={actual - expected - {'SHA256SUMS'}}"
        )
    lines = []
    for target in sorted(t["target"] for t in TARGETS):
        name = asset_name(target)
        read_archive(dist / name, target, version, sha)
        with (dist / name).open("rb") as stream:
            lines.append(
                f"{hashlib.file_digest(stream, 'sha256').hexdigest()}  {name}\n"
            )
    return "".join(lines)


def manifest(dist: Path, version: str, sha: str, *, verify: bool = False) -> None:
    text = checksums(dist, version, sha)
    checksum_file = dist / "SHA256SUMS"
    if verify:
        if checksum_file.read_text(encoding="utf-8") != text:
            raise ValueError("SHA256SUMS does not match release files")
    else:
        checksum_file.write_text(text, encoding="utf-8")
    print("Verified all eight platform packages and SHA256SUMS")


def smoke(root: Path, dist: Path, target: str) -> None:
    files = read_archive(
        dist / asset_name(target),
        target,
        version_at(root),
        git(root, "rev-parse", "HEAD"),
    )
    env = dict(os.environ)
    for key in (
        "UV_PYTHON",
        "UV_PYTHON_WRAPPER_ACTIVE",
        "PYTHONHOME",
        "PYTHONPATH",
        "PYTHONOPTIMIZE",
        "VIRTUAL_ENV",
    ):
        env.pop(key, None)
    env.update(UV_NO_CONFIG="1", UV_PYTHON_DOWNLOADS="never")
    uv = shutil.which("uv")
    if uv is None:
        raise ValueError("Real uv is required for the release smoke check")
    with tempfile.TemporaryDirectory(prefix="uv-wrapper-smoke-") as temporary:
        directory = Path(temporary)
        for name, data in files.items():
            path = directory / name
            path.write_bytes(data)
            if name in binary_names(target):
                path.chmod(0o755)
        # Resolve the reference interpreter before placing the packaged wrapper first.
        python = (
            run(
                [uv, "python", "find", "--system"],
                env=env,
                cwd=root,
                capture_output=True,
            )
            .stdout.decode()
            .strip()
        )
        expected_version = run(
            [python, "--version"], env=env, capture_output=True
        ).stdout
        expected_pip = run(
            [python, "-m", "pip", "--version"], env=env, capture_output=True
        ).stdout
        print(f"Reference interpreter: {expected_version.decode().strip()} ({python})")
        env["PATH"] = str(directory) + os.pathsep + env.get("PATH", "")
        wrapper, pip = (str(directory / name) for name in binary_names(target))
        if (
            run(
                [wrapper, "--version"], env=env, cwd=directory, capture_output=True
            ).stdout
            != expected_version
        ):
            raise ValueError("Packaged wrapper resolved an unexpected Python version")
        if (
            run([pip, "--version"], env=env, cwd=directory, capture_output=True).stdout
            != expected_pip
        ):
            raise ValueError("Packaged pip resolved an unexpected Python environment")
        data = b"pipe\0\xff\r\nUTF-8: \xe4\xb8\xad\xe6\x96\x87\n"
        script = (
            "import sys\n"
            "if sys.argv[1:] != ['with spaces', '中文', '']: raise SystemExit('argument mismatch')\n"
            "sys.stdout.buffer.write(sys.stdin.buffer.read())\n"
            "sys.stderr.buffer.write(b'stderr marker')\n"
            "sys.exit(23)\n"
        )
        result = subprocess.run(
            [wrapper, "-c", script, "with spaces", "中文", ""],
            check=False,
            env=env,
            cwd=directory,
            input=data,
            capture_output=True,
            timeout=30,
        )
        if (result.returncode, result.stdout, result.stderr) != (
            23,
            data,
            b"stderr marker",
        ):
            raise ValueError(
                f"Packaged Python pipe/arguments/exit status check failed: {result!r}"
            )
    print(f"Real uv/Python/pip smoke check passed: {target}")


def gh_json(
    endpoint: str, data: dict[str, Any] | None = None, method: str = "GET"
) -> Any:
    args = ["gh", "api", endpoint, "--method", method]
    kwargs: dict[str, Any] = {"capture_output": True, "text": True}
    if data is not None:
        args += ["--input", "-"]
        kwargs["input"] = json.dumps(data)
    result = subprocess.run(args, check=False, timeout=300, **kwargs)
    if result.returncode:
        if method == "GET" and "(HTTP 404)" in result.stderr:
            return None
        raise RuntimeError(f"GitHub API failed: {result.stderr}")
    return json.loads(result.stdout)


def ensure_tag(repository: str, tag: str, sha: str) -> None:
    reference = gh_json(f"repos/{repository}/git/ref/tags/{quote(tag, safe='')}")
    if reference is None:
        gh_json(
            f"repos/{repository}/git/refs",
            {"ref": f"refs/tags/{tag}", "sha": sha},
            "POST",
        )
        return
    obj = reference["object"]
    for _ in range(10):
        if obj["type"] != "tag":
            break
        obj = gh_json(f"repos/{repository}/git/tags/{obj['sha']}")["object"]
    if obj["type"] != "commit" or obj["sha"] != sha:
        raise ValueError(
            f"Existing tag {tag} points to another commit; refusing to move it"
        )


def verify_remote(repository: str, tag: str, version: str, sha: str) -> None:
    with tempfile.TemporaryDirectory(prefix="uv-wrapper-release-") as temporary:
        run(
            ["gh", "release", "download", tag, "--repo", repository, "--dir", temporary]
        )
        manifest(Path(temporary), version, sha, verify=True)


def publish(
    root: Path, dist: Path, repository: str, version: str, tag: str, sha: str
) -> None:
    if (
        os.environ.get("GITHUB_EVENT_NAME") not in {"push", "workflow_dispatch"}
        or os.environ.get("GITHUB_REF") != "refs/heads/main"
    ):
        raise ValueError(
            "GitHub releases are permitted only from a main push or manual main run"
        )
    if (
        version != version_at(root)
        or tag != f"v{version}"
        or sha != git(root, "rev-parse", "HEAD")
    ):
        raise ValueError("Release inputs must match the checked-out version and commit")
    manifest(dist, version, sha, verify=True)
    ensure_tag(repository, tag, sha)
    release = gh_json(f"repos/{repository}/releases/tags/{quote(tag, safe='')}")
    if release and not release["draft"]:
        verify_remote(repository, tag, version, sha)
        print(f"{tag} is already published with verified assets; leaving it unchanged")
        return
    if not release:
        run(
            [
                "gh",
                "release",
                "create",
                tag,
                "--repo",
                repository,
                "--verify-tag",
                "--draft",
                "--title",
                tag,
                "--generate-notes",
            ]
        )
    run(
        [
            "gh",
            "release",
            "upload",
            tag,
            "--repo",
            repository,
            "--clobber",
            *map(str, sorted(dist.iterdir())),
        ]
    )
    verify_remote(repository, tag, version, sha)
    release = gh_json(f"repos/{repository}/releases/tags/{quote(tag, safe='')}")
    prerelease = "-" in version.split("+", 1)[0]
    # Let GitHub choose the latest stable version atomically, even if releases of
    # different versions finish out of order. Prereleases never replace latest.
    gh_json(
        f"repos/{repository}/releases/{release['id']}",
        {
            "draft": False,
            "prerelease": prerelease,
            "make_latest": "false" if prerelease else "legacy",
        },
        "PATCH",
    )
    verify_remote(repository, tag, version, sha)
    print(f"Published https://github.com/{repository}/releases/tag/{tag}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "command", choices=["plan", "package", "smoke", "manifest", "verify", "publish"]
    )
    parser.add_argument("--target")
    parser.add_argument("--dist", type=Path, default=ROOT / "dist")
    args = parser.parse_args()
    if args.command == "plan":
        write_outputs(
            plan(
                ROOT,
                os.environ.get("GITHUB_EVENT_NAME", "workflow_dispatch"),
                os.environ.get("GITHUB_REF", ""),
                os.environ.get("BEFORE", ""),
                os.environ.get("CI_MODE", "check"),
            )
        )
    elif args.command in {"package", "smoke"}:
        if not args.target:
            parser.error("--target is required for package and smoke")
        if args.command == "package":
            package(ROOT, args.target, args.dist)
        else:
            smoke(ROOT, args.dist, args.target)
    elif args.command == "publish":
        publish(
            ROOT,
            args.dist,
            os.environ["GITHUB_REPOSITORY"],
            os.environ["RELEASE_VERSION"],
            os.environ["RELEASE_TAG"],
            os.environ["RELEASE_SHA"],
        )
    else:
        manifest(
            args.dist,
            version_at(ROOT),
            git(ROOT, "rev-parse", "HEAD"),
            verify=args.command == "verify",
        )


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        sys.exit(str(error))
