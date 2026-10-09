#!/usr/bin/env python3
"""Check real crate archives and version-only consumers without publishing.

Requires Python 3.11+, Cargo with multi-package `cargo package` support, cached
locked dependencies, and the stable/MSRV toolchains. Uses only a local registry.
"""
from __future__ import annotations

import argparse
import importlib.util
import hashlib
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
PACKAGES = (
    "rustwright", "rustwright-bidi", "rustwright-browser", "rustwright-cdp",
    "rustwright-common", "rustwright-core", "rustwright-test", "rustwright-test-macros",
)


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command: list[str], cwd: Path, log: Path, env: dict[str, str] | None = None) -> None:
    log.parent.mkdir(parents=True, exist_ok=True)
    print("Running", " ".join(command), flush=True)
    with log.open("w") as stream:
        process = subprocess.run(command, cwd=cwd, env=env, stdout=stream, stderr=subprocess.STDOUT)
    if process.returncode:
        raise RuntimeError(f"Command failed ({process.returncode}); see {log}")


def archive_manifest(archive: Path) -> dict:
    with tarfile.open(archive, "r:gz") as contents:
        root = archive.name.removesuffix(".crate")
        stream = contents.extractfile(f"{root}/Cargo.toml")
        if stream is None:
            raise ValueError(f"Missing normalized manifest: {archive}")
        return tomllib.loads(stream.read().decode())


def dependency_tables(manifest: dict):
    for field, kind in (("dependencies", "normal"), ("build-dependencies", "build"), ("dev-dependencies", "dev")):
        for name, dependency in manifest.get(field, {}).items():
            yield name, dependency, kind, None
    for target, config in manifest.get("target", {}).items():
        for field, kind in (("dependencies", "normal"), ("build-dependencies", "build"), ("dev-dependencies", "dev")):
            for name, dependency in config.get(field, {}).items():
                yield name, dependency, kind, target


def audit_archives(directory: Path, output: Path, version: str) -> list[dict]:
    report = []
    failures = []
    for name in PACKAGES:
        archive = directory / f"{name}-{version}.crate"
        manifest = archive_manifest(archive)
        package = manifest["package"]
        prefix = f"{name}-{version}/"
        with tarfile.open(archive, "r:gz") as contents:
            members = set(contents.getnames())
            def matches(relative: str, expected: Path) -> bool:
                stream = contents.extractfile(prefix + relative) if prefix + relative in members else None
                return stream is not None and stream.read() == expected.read_bytes()
            checks = {
                "package_name": package["name"] == name,
                "package_version": package["version"] == version,
                "msrv": package.get("rust-version") == "1.85",
                "license_expression": package.get("license") == "MIT OR Apache-2.0",
                "readme_metadata": package.get("readme") == "PACKAGE_README.md",
                "readme_content": matches("PACKAGE_README.md", ROOT / "docs/PACKAGE_README.md"),
                "license_mit": matches("LICENSE-MIT", ROOT / "LICENSE-MIT"),
                "license_apache": matches("LICENSE-APACHE", ROOT / "LICENSE-APACHE"),
                "no_path_dependencies": all(not isinstance(dep, dict) or not {"path", "git", "workspace"}.intersection(dep) for _, dep, _, _ in dependency_tables(manifest)),
            }
            source_members = [member.removeprefix(prefix) for member in members if member.startswith(prefix) and member.removeprefix(prefix) not in ("Cargo.toml", "Cargo.lock", ".cargo_vcs_info.json", "PACKAGE_README.md")]
            checks["source_content"] = all(matches(relative, ROOT / "crates" / name / ("Cargo.toml" if relative == "Cargo.toml.orig" else relative)) for relative in source_members)
            if name == "rustwright-common":
                checks["injected_js"] = matches("src/injected.js", ROOT / "crates/rustwright-common/src/injected.js")
            if name == "rustwright-test-macros":
                checks["proc_macro"] = manifest.get("lib", {}).get("proc-macro") is True
            report.append({"name": name, "version": version, "archive": str(archive), "sha256": sha256(archive), "file_count": len(members), "checks": checks})
            failures.extend(f"{name}: {check}" for check, success in checks.items() if not success)
    output.write_text(json.dumps({"packages": report, "failures": failures}, indent=2) + "\n")
    if failures:
        raise RuntimeError("Archive audit failed: " + ", ".join(failures))
    return report


def registry_index_path(name: str) -> Path:
    if len(name) == 1:
        return Path("1") / name
    if len(name) == 2:
        return Path("2") / name
    if len(name) == 3:
        return Path("3") / name[0] / name
    return Path(name[:2]) / name[2:4] / name


def index_record(archive: Path, manifest: dict) -> dict:
    package = manifest["package"]
    dependencies = []
    for name, raw, kind, target in dependency_tables(manifest):
        dependency = {"version": raw} if isinstance(raw, str) else raw
        if {"path", "git", "workspace"}.intersection(dependency):
            raise ValueError(f"Non-registry dependency in normalized {archive}: {name}")
        record = {
            "name": name, "req": dependency["version"], "features": dependency.get("features", []),
            "optional": dependency.get("optional", False), "default_features": dependency.get("default-features", True),
            "target": target, "kind": kind, "registry": None,
        }
        if "package" in dependency:
            record["package"] = dependency["package"]
        dependencies.append(record)
    return {
        "name": package["name"], "vers": package["version"], "deps": dependencies,
        "cksum": sha256(archive), "features": {}, "features2": manifest.get("features", {}),
        "yanked": False, "links": package.get("links"), "rust_version": package.get("rust-version"), "v": 2,
    }


def make_registry(directory: Path, rustwright_archives: Path, cargo_home: Path, lock: dict) -> list[dict]:
    if directory.exists():
        shutil.rmtree(directory)
    (directory / "index").mkdir(parents=True)
    verified = []
    archives = []
    for package in lock["package"]:
        if not package.get("source", "").startswith("registry+"):
            continue
        filename = f"{package['name']}-{package['version']}.crate"
        candidates = list((cargo_home / "registry/cache").glob("*/" + filename))
        valid = [path for path in candidates if sha256(path) == package["checksum"]]
        if not valid:
            raise RuntimeError(f"Missing cached archive matching Cargo.lock checksum: {filename}")
        source = valid[0]
        destination = directory / filename
        shutil.copyfile(source, destination)
        verified.append({"name": package["name"], "version": package["version"], "checksum": package["checksum"], "cached_archive": str(source)})
        archives.append(destination)
    version = lock_version(lock)
    for name in PACKAGES:
        destination = directory / f"{name}-{version}.crate"
        shutil.copyfile(rustwright_archives / destination.name, destination)
        archives.append(destination)
    entries: dict[Path, list[dict]] = {}
    for archive in archives:
        record = index_record(archive, archive_manifest(archive))
        entries.setdefault(registry_index_path(record["name"]), []).append(record)
    for relative, records in entries.items():
        path = directory / "index" / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("".join(json.dumps(record, separators=(",", ":")) + "\n" for record in records))
    return verified


def lock_version(lock: dict) -> str:
    versions = {package["version"] for package in lock["package"] if package["name"] in PACKAGES and "source" not in package}
    if len(versions) != 1:
        raise ValueError(f"Rustwright version mismatch: {versions}")
    return versions.pop()


def make_consumer(directory: Path, registry: Path, version: str) -> None:
    if directory.exists():
        shutil.rmtree(directory)
    (directory / "src").mkdir(parents=True)
    (directory / "tests").mkdir()
    (directory / ".cargo").mkdir()
    dependencies = "".join(f'{name} = "={version}"\n' for name in PACKAGES)
    (directory / "Cargo.toml").write_text('[package]\nname = "rustwright-release-consumer"\nversion = "0.0.0"\nedition = "2021"\nrust-version = "1.85"\n\n[workspace]\n\n[dependencies]\n' + dependencies + 'tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "io-util", "sync", "time"] }\nserde_json = "1"\n')
    (directory / ".cargo/config.toml").write_text('[source.crates-io]\nreplace-with = "release-check-local"\n\n[source.release-check-local]\nlocal-registry = ' + json.dumps(str(registry)) + '\n')
    (directory / "src/main.rs").write_text('''fn main() {
    assert!(rustwright_common::INJECTED_SCRIPT.contains("prepareClick"));
    let _ = std::mem::size_of::<rustwright::AnyPage>();
    let _ = std::mem::size_of::<rustwright_bidi::BidiPage>();
    let _ = std::mem::size_of::<rustwright_browser::Chrome>();
    let _ = std::mem::size_of::<rustwright_cdp::CdpConnection>();
    let _ = std::mem::size_of::<rustwright_core::Browser>();
    let _ = std::mem::size_of::<rustwright_test::TestContext>();
    println!("Packaged Rustwright consumer loaded both backends and injected.js");
}
''')
    (directory / "tests/macro_compile.rs").write_text('''use rustwright_test::{Result, TestContext};
#[rustwright_test::rustwright_test]
async fn reexported_macro(_context: TestContext) -> Result<()> { Ok(()) }
#[rustwright_test_macros::rustwright_test]
async fn direct_macro(_context: TestContext) -> Result<()> { Ok(()) }
''')
    template = ROOT / "scripts/release/consumer_tests.rs"
    if template.is_file():
        shutil.copyfile(template, directory / "tests/browser.rs")
    extractor=ROOT/"scripts/release/readme_snippets.py"
    spec=importlib.util.spec_from_file_location("rustwright_readme_snippets",extractor)
    if spec is None or spec.loader is None:
        raise RuntimeError("README snippet extractor unavailable")
    snippets=importlib.util.module_from_spec(spec)
    spec.loader.exec_module(snippets)
    snippets.prepare(ROOT/"README.md",directory)



def audit_resolution(consumer: Path, log: Path, env: dict[str, str], cargo: str, toolchain: str, packages: list[dict]) -> dict:
    command = [cargo, f"+{toolchain}", "metadata", "--offline", "--locked", "--format-version", "1"]
    print("Running", " ".join(command), flush=True)
    process=subprocess.run(command,cwd=consumer,env=env,capture_output=True,text=True)
    log.write_text(process.stdout)
    log.with_suffix(".stderr.log").write_text(process.stderr)
    if process.returncode:
        raise RuntimeError(f"Metadata failed ({process.returncode}); see {log.with_suffix('.stderr.log')}")
    metadata = json.loads(process.stdout)
    resolved = {}
    checksums = {entry["name"]: entry["sha256"] for entry in packages}
    lock = tomllib.loads((consumer / "Cargo.lock").read_text())
    for package in metadata["packages"]:
        if package["name"] not in PACKAGES:
            continue
        manifest = Path(package["manifest_path"])
        if not package.get("source", "").startswith("registry+") or not manifest.is_relative_to(Path(env["CARGO_HOME"])):
            raise RuntimeError(f"Consumer used a workspace/path package: {package['name']}: {manifest}")
        locked = next(entry for entry in lock["package"] if entry["name"] == package["name"])
        if locked["checksum"] != checksums[package["name"]]:
            raise RuntimeError(f"Consumer checksum mismatch: {package['name']}")
        resolved[package["name"]] = {"version": package["version"], "source": package["source"], "manifest_path": str(manifest), "checksum": locked["checksum"]}
    if set(resolved) != set(PACKAGES):
        raise RuntimeError("Consumer did not resolve all eight Rustwright archives")
    return resolved


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/release-check")
    parser.add_argument("--stable", default="stable")
    parser.add_argument("--msrv", default="1.85.0")
    parser.add_argument("--audit-only", type=Path, help="Audit existing archives without packaging/building")
    parser.add_argument("--run-browser-tests", action="store_true", help="Run only the separate native consumer fixture, if available")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    # A failed rerun must not leave an earlier successful summary behind.
    report_path = output / "report.json"
    report_path.write_text(json.dumps({"status": "running", "scope": "local release verification"}) + "\n", encoding="utf-8")
    (output / "readme-runtime.json").unlink(missing_ok=True)
    lock_path = ROOT / "Cargo.lock"
    original_lock = lock_path.read_bytes()
    lock = tomllib.loads(original_lock.decode())
    version = lock_version(lock)
    if args.audit_only:
        audit_archives(args.audit_only.resolve(), output / "archive-audit.json", version)
        report_path.write_text(json.dumps({"status": "audit-only", "archive_audit": str(output / "archive-audit.json")}) + "\n", encoding="utf-8")
        return
    cargo = shutil.which("cargo")
    if cargo is None:
        report_path.write_text(json.dumps({"status": "failed", "error": "Cargo is not available"}) + "\n", encoding="utf-8")
        raise RuntimeError("Cargo is not available; activate the Rust environment first")
    cargo_home = Path(os.environ.get("CARGO_HOME", str(Path.home() / ".cargo"))).resolve()
    # Cargo caches unpacked staging sources by registry location + name/version.
    # Give changed unpublished sources a new location, even at the same version.
    inputs = [ROOT / filename for filename in ("Cargo.toml", "Cargo.lock", "docs/PACKAGE_README.md", "LICENSE-MIT", "LICENSE-APACHE")]
    for name in PACKAGES:
        inputs.extend(path for path in (ROOT / "crates" / name).rglob("*") if path.is_file() and "target" not in path.relative_to(ROOT / "crates" / name).parts)
    source_fingerprint = hashlib.sha256("".join(str(path.relative_to(ROOT)) + ":" + sha256(path) + "\n" for path in sorted(inputs)).encode()).hexdigest()
    target = output / "package-target" / source_fingerprint[:16]
    command = [cargo, f"+{args.stable}", "package", "--allow-dirty", "--offline", "--locked", "--target-dir", str(target)]
    for name in PACKAGES:
        command.extend(["--package", name])
    try:
        run(command, ROOT, output / "package.log")
        archives = target / "package"
        packages = audit_archives(archives, output / "archive-audit.json", version)
        artifact_set=hashlib.sha256("".join(package["sha256"] for package in packages).encode()).hexdigest()[:16]
        registry = output / f"registry-{artifact_set}"
        third_party = make_registry(registry, archives, cargo_home, lock)
        (output / "registry-checksums.json").write_text(json.dumps(third_party, indent=2) + "\n")
        consumer = output / "consumer"
        make_consumer(consumer, registry, version)
        consumer_home=output / "consumer-cargo-home"
        if consumer_home.exists():
            shutil.rmtree(consumer_home)
        env = dict(os.environ, CARGO_HOME=str(consumer_home), CARGO_TARGET_DIR=str(output / "consumer-target-stable"))
        run([cargo, f"+{args.stable}", "generate-lockfile", "--offline"], consumer, output / "consumer-lock.log", env)
        resolution = {}
        for label, toolchain in (("stable", args.stable), ("msrv", args.msrv)):
            env["CARGO_TARGET_DIR"] = str(output / f"consumer-target-{label}")
            run([cargo, f"+{toolchain}", "test", "--offline", "--locked", "--all-targets", "--no-run"], consumer, output / f"consumer-{label}-compile.log", env)
            run([cargo, f"+{toolchain}", "run", "--offline", "--locked", "--bin", "rustwright-release-consumer"], consumer, output / f"consumer-{label}-run.log", env)
            resolution[label] = audit_resolution(consumer, output / f"consumer-{label}-metadata.json", env, cargo, toolchain, packages)
        native={}
        if args.run_browser_tests:
            if not (consumer / "tests/browser.rs").exists():
                raise RuntimeError("Native consumer template scripts/release/consumer_tests.rs is missing")
            for backend,key in (("chrome","RUSTWRIGHT_CHROME"),("firefox","RUSTWRIGHT_FIREFOX")):
                if not Path(env.get(key,"missing-browser-binding")).is_file():
                    raise RuntimeError(f"Native consumer requires an explicit installed {key} executable")
            env["CARGO_TARGET_DIR"] = str(output / "consumer-target-stable")
            env["RUSTWRIGHT_HEADLESS"]="1"
            env["RUSTWRIGHT_RETRIES"]="0"
            env.pop("RUSTWRIGHT_SHARD",None)
            for backend in ("chrome","firefox"):
                profile_parent=output / "native-profiles"
                profile_parent.mkdir(parents=True,exist_ok=True)
                with tempfile.TemporaryDirectory(prefix=f"{backend}-",dir=profile_parent) as profile:
                    env["RUSTWRIGHT_BROWSER"]=backend
                    env["RUSTWRIGHT_PROFILE"]=profile
                    env["RUSTWRIGHT_RELEASE_OUTPUT"]=str(output / "native-screenshots" / backend)
                    log=output / f"consumer-{backend}-native.log"
                    run([cargo, f"+{args.stable}", "test", "--offline", "--locked", "--test", "browser", "--", "--test-threads=1", "--nocapture"], consumer, log, env)
                    text = log.read_text(encoding="utf-8")
                    counts = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; (\d+) filtered out", text)
                    if counts != [("2", "0", "0", "0")] or '"macro_callback_executed":true' not in text or f'"backend":"{backend}"' not in text:
                        raise RuntimeError(f"Native {backend} consumer did not execute both required cases and its macro callback; see {log}")
                    native[backend]={"log":str(log),"explicit_browser":env[f"RUSTWRIGHT_{backend.upper()}"],"headless":True,"retries":0,"fresh_profile":True,"profile_removed_on_exit":True,"passed":2,"failed":0,"ignored":0,"filtered":0}
        report = {
            "status": "passed",
            "package_input_sha256": source_fingerprint,
            "scope": "local registry simulation from verified real .crate archives; no registry publication or upload",
            "rustwright_packages": packages, "third_party_verified_archives": len(third_party),
            "consumer_resolution": resolution, "macro_checks": "both re-exported and direct attributes compiled; " + ("native runner fixtures executed on Chrome and Firefox" if args.run_browser_tests else "browser runner tests not executed"),
            "native_consumer_executed": args.run_browser_tests, "native_backends":native, "registry_path":str(registry), "readme_snippets":json.loads((consumer/"readme-snippets.json").read_text()), "root_lock_sha256": sha256(lock_path),
        }
        report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(f"Release checks passed: {output / 'report.json'}", flush=True)
    except Exception as error:
        report_path.write_text(json.dumps({"status": "failed", "error": str(error)}) + "\n", encoding="utf-8")
        raise
    finally:
        if lock_path.read_bytes() != original_lock:
            report_path.write_text(json.dumps({"status": "failed", "error": "Root Cargo.lock changed during release checks"}) + "\n", encoding="utf-8")
            raise RuntimeError("Root Cargo.lock changed during release checks")


if __name__ == "__main__":
    main()
