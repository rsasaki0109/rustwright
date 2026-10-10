#!/usr/bin/env python3
"""Required sequential headed, alternate-version, and public-site observations."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import threading
import time

ROOT = Path(__file__).resolve().parents[2]
VERSIONS = {"chrome_current": "155.0.8059.39", "firefox_current": "157.0.1",
            "chrome_previous": "151.0.7922.138", "firefox_esr": "153.4.0esr"}
# Gecko capabilities report the application version, which can omit the ESR
# channel suffix. The executable --version assertion above remains exact.
PROTOCOL_VERSIONS = {key: version.removesuffix("esr") for key, version in VERSIONS.items()}
COUNTS = {"http_compat": 24, "http_actionability": 12, "http_clipped_control": 10,
          "http_bidi_navigation": 7, "http_network_idle_parity": 26}
SITES = ["https://example.com/", "https://developer.mozilla.org/en-US/docs/Web", "https://docs.rs/"]


def stamp() -> str:
    return datetime.now(timezone.utc).isoformat()


def version_matches(text: str, version: str) -> bool:
    return re.search(r"(?<![\d.])" + re.escape(version) + r"(?![\w.])", text) is not None


def descendant(pid: int, ancestor: int) -> bool:
    """Require a live window PID owned by this command, rather than a title match."""
    seen = set()
    while pid > 1 and pid not in seen:
        if pid == ancestor:
            return True
        seen.add(pid)
        try:
            status = Path(f"/proc/{pid}/status").read_text()
            pid = int(re.search(r"^PPid:\s+(\d+)$", status, re.M)[1])
        except (OSError, TypeError, ValueError):
            return False
    return False


class Windows:
    """Retain raw X11 observations; only owned IsViewable application windows count."""

    def __init__(self, output: Path, pid: int, env: dict[str, str]):
        self.output, self.pid, self.env = output, pid, env
        self.stop = threading.Event()
        self.proofs: dict[str, dict] = {}
        self.error = None
        self.thread = threading.Thread(target=self.observe, daemon=True)

    def probe(self, command: list[str], stream) -> dict:
        result = subprocess.run(command, env=self.env, capture_output=True, text=True, timeout=2)
        record = {"utc": stamp(), "command": command, "exit_code": result.returncode,
                  "stdout": result.stdout, "stderr": result.stderr}
        stream.write(json.dumps(record) + "\n")
        stream.flush()
        return record

    def observe(self) -> None:
        try:
            with self.output.open("x", encoding="utf-8") as stream:
                while not self.stop.is_set():
                    tree = self.probe(["xwininfo", "-root", "-tree"], stream)
                    if tree["exit_code"]:
                        raise RuntimeError("xwininfo cannot inspect the active display")
                    ids = list(dict.fromkeys(re.findall(r"^\s+(0x[0-9a-fA-F]+)\s", tree["stdout"], re.M)))
                    for window in ids:
                        if self.stop.is_set():
                            break
                        props = self.probe(["xprop", "-id", window, "WM_CLASS", "_NET_WM_PID"], stream)
                        if props["exit_code"]:
                            continue  # A window may be destroyed between snapshots.
                        classes = re.search(r"WM_CLASS\(STRING\)\s*=\s*(.*)", props["stdout"])
                        pid = re.search(r"_NET_WM_PID\(CARDINAL\)\s*=\s*(\d+)", props["stdout"])
                        if classes is None or pid is None:
                            continue
                        name = classes[1].lower()
                        backend = "firefox" if "firefox" in name else "chrome" if ("chrome" in name or "chromium" in name) else None
                        if backend is None or not descendant(int(pid[1]), self.pid):
                            continue
                        info = self.probe(["xwininfo", "-id", window, "-stats"], stream)
                        if info["exit_code"] == 0 and re.search(r"Map State:\s+IsViewable\b", info["stdout"]):
                            self.proofs.setdefault(backend, {"utc": stamp(), "window": window,
                                "WM_CLASS": classes[1], "pid": int(pid[1]), "command_ancestor_pid": self.pid,
                                "map_state": "IsViewable", "raw_log": str(self.output)})
                    self.stop.wait(0.15)
        except Exception as error:
            # Observer failures fail the headed command even if an earlier
            # snapshot had already found a window; they cannot become proof.
            self.error = str(error)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for key in VERSIONS:
        parser.add_argument("--" + key.replace("_", "-"), required=True, type=Path)
    parser.add_argument("--sandbox-current", required=True, type=Path)
    parser.add_argument("--sandbox-previous", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    summary = output / "report.json"
    if summary.exists():
        parser.error(f"Refusing to overwrite existing evidence: {summary}")
    report = {"schema_version": 1, "started_utc": stamp(), "status": "running", "commands": [],
              "expected_versions": VERSIONS, "expected_protocol_versions": PROTOCOL_VERSIONS,
              "sites": SITES, "http_status_policy":
              "HTTP denial/error observations remain separate from mandatory successful browser operations",
              "window_proof_scope": "X11 IsViewable WM_CLASS and live PID descendant observed during each headed command",
              "proxy_policy": "compat_report has no proxy argument; browser default TLS verification", "browsers": {}}

    def save() -> None:
        summary.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    def cleanup(child: subprocess.Popen, record: dict) -> None:
        """Bound cleanup to the command's new session; retain cleanup failures."""
        failures = []
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except OSError as error:
            failures.append(f"owned process-group kill failed: {error}")
        try:
            child.wait(timeout=5)
        except (OSError, subprocess.TimeoutExpired) as error:
            failures.append(f"direct child reap failed: {error}")
        record["cleanup"] = {"process_group": child.pid, "direct_child_reaped": child.returncode is not None,
                             "deadline_seconds": 5, "errors": failures}

    def run(name: str, command: list[str], env: dict[str, str], timeout: int, headed=()) -> dict:
        record = {"name": name, "command": command, "started_utc": stamp(),
                  "environment": {key: env.get(key) for key in ("DISPLAY", "RUSTWRIGHT_CHROME", "RUSTWRIGHT_FIREFOX",
                      "CHROME_DEVEL_SANDBOX", "RUSTWRIGHT_HEADLESS", "RUSTWRIGHT_RETRIES")},
                  "log": str(output / f"{name}.log"), "status": "running"}
        report["commands"].append(record)
        save()
        observer = None
        started = time.monotonic()
        print(f"Running {name}: {command}", flush=True)
        with Path(record["log"]).open("x", encoding="utf-8") as stream:
            child = subprocess.Popen(command, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT, start_new_session=True)
            try:
                if headed:
                    observer = Windows(output / f"{name}-windows.jsonl", child.pid, env)
                    observer.thread.start()
                record["exit_code"] = child.wait(timeout=timeout)
            except subprocess.TimeoutExpired:
                cleanup(child, record)
                record.update(exit_code=child.returncode, timed_out=True)
            except BaseException as error:
                record["parent_error"] = f"{type(error).__name__}: {error}"
                cleanup(child, record)
                record["exit_code"] = child.returncode
                raise
            finally:
                if observer:
                    observer.stop.set()
                    if observer.thread.ident is not None:
                        observer.thread.join(timeout=5)
                    record["mapped_windows"] = observer.proofs
                    record["window_observer_error"] = observer.error
                    if observer.thread.is_alive():
                        record["window_observer_error"] = "Observer did not stop within five seconds"
                record.update(finished_utc=stamp(), duration_seconds=time.monotonic() - started)
                record["status"] = "passed" if record.get("exit_code") == 0 and not record.get("timed_out") and not record.get("parent_error") else "failed"
                save()
        if record["status"] != "passed":
            raise RuntimeError(f"{name} failed: {record}; see original log")
        if headed and (record["window_observer_error"] or not set(headed).issubset(record["mapped_windows"])):
            record["status"] = "failed"
            save()
            raise RuntimeError(f"{name} lacks required owned mapped-window proof: {record}")
        return record

    def environment(chrome: Path, firefox: Path, sandbox: Path, headed: bool) -> dict[str, str]:
        metadata = json.loads(sandbox.read_text())
        if metadata.get("status") != "passed" or Path(metadata["chrome"]["path"]).resolve() != chrome:
            raise ValueError("Sandbox report must successfully describe the exact selected Chrome executable")
        env = dict(os.environ)
        env.pop("CHROME_DEVEL_SANDBOX", None)
        helper = metadata.get("installed_helper")
        if helper:
            path = Path(helper["path"])
            info = path.stat()
            if info.st_uid != 0 or info.st_gid != 0 or info.st_mode & 0o7777 != 0o4755:
                raise ValueError(f"Configured sandbox helper lost its root-owned 4755 contract: {path}")
            if hashlib.sha256(path.read_bytes()).hexdigest() != helper["sha256"]:
                raise ValueError("Configured sandbox helper bytes changed")
            env["CHROME_DEVEL_SANDBOX"] = str(path)
        elif metadata.get("initial_CHROME_DEVEL_SANDBOX"):
            raise ValueError("Sandbox report relied on an inherited helper instead of its selected installation")
        env.update(RUSTWRIGHT_CHROME=str(chrome), RUSTWRIGHT_FIREFOX=str(firefox),
                   RUSTWRIGHT_RETRIES="0", RUSTWRIGHT_HEADLESS="0" if headed else "1", RUSTWRIGHT_REQUIRE_BROWSERS="1")
        env.pop("RUSTWRIGHT_SHARD", None)
        return env

    def portable(name: str, env: dict[str, str], headed: bool, versions: tuple[str, str]) -> None:
        directory = output / name
        if directory.exists():
            raise ValueError(f"Refusing to reuse native result directory: {directory}")
        command = [sys.executable, str(ROOT / "scripts/ci/browser_checks.py"), "--suite", "portable", "--output", str(directory)]
        if headed:
            command.append("--headed")
        record = run(name, command, env, 3600, ("chrome", "firefox") if headed else ())
        native = json.loads((directory / "report.json").read_text())
        if native.get("status") != "passed" or native.get("passed") != 79 or native.get("headless") != (not headed):
            raise RuntimeError(f"{name}: expected complete 79-case native suite in requested mode")
        if native.get("targets") != list(COUNTS) or len(native.get("cases", [])) != len(COUNTS):
            raise RuntimeError(f"{name}: portable target selection changed")
        if native.get("browsers") != {key: env[key] for key in ("RUSTWRIGHT_CHROME", "RUSTWRIGHT_FIREFOX")}:
            raise RuntimeError(f"{name}: native harness selected different executable paths")
        protocol_observations = {backend: [] for backend in ("Chrome", "Firefox")}
        for case in native["cases"]:
            if case.get("passed") != COUNTS[case["target"]] or any(case.get(key) != 0 for key in ("exit_code", "failed", "ignored", "filtered")):
                raise RuntimeError(f"{name}: native cases were failed, skipped, filtered or omitted")
            text = Path(case["log"]).read_text()
            for backend, version in zip(("Chrome", "Firefox"), versions):
                lines = [line for line in text.splitlines() if f"{backend} version:" in line]
                protocol_version = version.removesuffix("esr")
                if not all(version_matches(line, protocol_version) or version_matches(line, version) for line in lines):
                    raise RuntimeError(f"{name}: raw {backend} version observations differ from {version}")
                protocol_observations[backend].extend({"target": case["target"], "line": line} for line in lines)
        if not all(protocol_observations.values()):
            raise RuntimeError(f"{name}: native protocol version observations are missing")
        record["protocol_version_observations"] = protocol_observations
        record["verified_native_passed"] = 79
        save()

    try:
        save()
        if sys.platform != "linux" or not os.environ.get("DISPLAY"):
            raise ValueError("This job requires Linux with an active Xvfb DISPLAY")
        if os.environ.get("MOZ_HEADLESS"):
            raise ValueError("Unset MOZ_HEADLESS; headed Firefox must create native windows")
        for tool in ("xwininfo", "xprop", "cargo"):
            if not shutil.which(tool):
                raise ValueError(f"Required tool is missing: {tool}")
        paths = {key: getattr(args, key).resolve(strict=True) for key in VERSIONS}
        if paths["chrome_current"] == paths["chrome_previous"] or paths["firefox_current"] == paths["firefox_esr"]:
            raise ValueError("Pinned browser installations must use distinct executable paths")
        for key, path in paths.items():
            if not path.is_file() or not os.access(path, os.X_OK):
                raise ValueError(f"Browser is not an executable file: {path}")
            record = run(f"version-{key}", [str(path), "--version"], dict(os.environ), 15)
            text = Path(record["log"]).read_text()
            if not version_matches(text, VERSIONS[key]):
                raise ValueError(f"Installed {key} did not report exact version {VERSIONS[key]}: {text}")
            report["browsers"][key] = {"path": str(path), "version_output": text.strip()}
        current = environment(paths["chrome_current"], paths["firefox_current"], args.sandbox_current, True)
        previous = environment(paths["chrome_previous"], paths["firefox_esr"], args.sandbox_previous, False)
        portable("headed-current", current, True, (VERSIONS["chrome_current"], VERSIONS["firefox_current"]))
        portable("headless-alternate", previous, False, (VERSIONS["chrome_previous"], VERSIONS["firefox_esr"]))
        run("build-compat-report", ["cargo", "build", "--locked", "-p", "rustwright-examples", "--example", "compat_report"], current, 900)
        report["site_observations"] = []
        for backend in ("chrome", "firefox"):
            directory = output / f"sites-{backend}"
            command = [str(ROOT / "target/debug/examples/compat_report"), "--browser", backend, "--headed", "--output", str(directory), "--viewport", "1280x800", *SITES]
            record = run(f"sites-{backend}", command, current, 600, (backend,))
            sites = json.loads((directory / "report.json").read_text())
            if sites.get("status") != "completed" or sites.get("headed_requested") is not True or sites.get("proxy") is not None:
                raise RuntimeError(f"{backend} site report did not complete requested headed/native-TLS operations")
            if not version_matches(str(sites["browser"]["version"]), VERSIONS[f"{backend}_current"]):
                raise RuntimeError(f"{backend} site report used an unexpected browser version")
            if [site.get("requested_url") for site in sites.get("sites", [])] != SITES:
                raise RuntimeError(f"{backend} site report omitted or replaced a requested URL")
            if not sites.get("operations") or any(op["status"] != "passed" for op in sites["operations"]):
                raise RuntimeError(f"{backend} browser setup/teardown operations failed")
            screenshots = []
            for site in sites["sites"]:
                if site.get("operations_succeeded") is not True or not site.get("operations") or any(op["status"] != "passed" for op in site["operations"]):
                    raise RuntimeError(f"{backend} site operations failed: {site['requested_url']}")
                png = Path(site["screenshot"])
                data = png.read_bytes()
                if png.resolve().parent != directory or not data.startswith(b"\x89PNG\r\n\x1a\n") or len(data) < 100:
                    raise RuntimeError("Site screenshot was absent or did not contain a PNG artifact")
                screenshots.append({"path": str(png), "bytes": len(data), "sha256": hashlib.sha256(data).hexdigest()})
            record["screenshots"] = screenshots
            report["site_observations"].append({"backend": backend, "report": str(directory / "report.json"),
                "operations_succeeded": True, "observations": [{"url": s["requested_url"], "observation": s["observation"],
                "observed_document_statuses": s["observed_document_statuses"]} for s in sites["sites"]]})
            save()
        report["status"] = "passed"
        return 0
    except (OSError, ValueError, RuntimeError, KeyError, TypeError, subprocess.SubprocessError) as error:
        report.update(status="failed", error=str(error))
        print(str(error), file=sys.stderr)
        return 1
    finally:
        report["finished_utc"] = stamp()
        save()


if __name__ == "__main__":
    raise SystemExit(main())
