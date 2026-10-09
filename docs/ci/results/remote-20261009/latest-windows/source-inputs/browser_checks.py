#!/usr/bin/env python3
"""Run required native HTTP suites, failing on missing browsers or empty suites."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
PORTABLE = ("http_compat", "http_actionability", "http_clipped_control", "http_bidi_navigation")


def targets(suite: str) -> list[str]:
    manifest = tomllib.loads((ROOT / "tests/Cargo.toml").read_text(encoding="utf-8"))
    registered = [item["name"] for item in manifest["test"] if item["name"].startswith("http_")]
    missing = set(PORTABLE) - set(registered)
    if missing:
        raise ValueError(f"Required portable targets are unregistered: {sorted(missing)}")
    if suite == "portable":
        return list(PORTABLE)
    return [name for name in registered if name not in PORTABLE]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--suite", choices=("portable", "linux-extra"), default="portable")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--list", action="store_true", help="List targets without running or validating browsers")
    args = parser.parse_args()
    selected = targets(args.suite)
    if args.list:
        print("\n".join(selected))
        return 0
    output = (args.output or ROOT / "target/ci" / args.suite).resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {"suite": args.suite, "platform": platform.platform(), "status": "running", "targets": selected, "cases": []}
    summary = output / "report.json"

    def save() -> None:
        summary.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    save()
    try:
        if args.suite == "linux-extra" and sys.platform != "linux":
            raise ValueError("linux-extra requires Linux; use portable on other operating systems")
        env = dict(os.environ)
        browsers = {}
        for key in ("RUSTWRIGHT_CHROME", "RUSTWRIGHT_FIREFOX"):
            value = env.get(key)
            if not value or not Path(value).is_file():
                raise ValueError(f"{key} must explicitly select an installed browser file")
            browsers[key] = str(Path(value).resolve())
            env[key] = browsers[key]
        report["browsers"] = browsers
        env.update(RUSTWRIGHT_RETRIES="0", RUSTWRIGHT_HEADLESS="1")
        env.pop("RUSTWRIGHT_SHARD", None)
        cargo = shutil.which("cargo")
        if cargo is None:
            raise ValueError("Cargo is required")
        report["cargo_version"] = subprocess.check_output([cargo, "--version"], text=True).strip()
        if not selected:
            raise ValueError("An empty target selection cannot pass")
        for target in selected:
            command = [cargo, "test", "--locked", "-p", "rustwright-integration-tests", "--test", target, "--", "--test-threads=1", "--nocapture"]
            log = output / f"{target}.log"
            record = {"target": target, "command": command, "log": str(log)}
            report["cases"].append(record)
            print(f"Running {target}; log: {log}", flush=True)
            save()
            with log.open("w", encoding="utf-8") as stream:
                result = subprocess.run(command, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT, timeout=1200)
            text = log.read_text(encoding="utf-8", errors="replace")
            record["exit_code"] = result.returncode
            counts = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; \d+ measured; (\d+) filtered out", text)
            if result.returncode != 0 or len(counts) != 1:
                raise RuntimeError(f"{target} failed or did not finish one native suite; see {log}")
            passed, failed, ignored, filtered = map(int, counts[0])
            record.update(passed=passed, failed=failed, ignored=ignored, filtered=filtered)
            if passed == 0 or failed or ignored or filtered:
                raise RuntimeError(f"{target} was empty, failed, ignored or filtered: {record}")
            if args.suite == "portable":
                for backend in ("chrome", "firefox"):
                    if not re.search(rf"test {backend}_\w+ \.\.\..*?\bok\b", text, re.S):
                        raise RuntimeError(f"{target} did not execute successful {backend} cases")
            save()
        report["status"] = "passed"
        report["passed"] = sum(case["passed"] for case in report["cases"])
        print(f"Native HTTP {args.suite}: {report['passed']} passed, no skips", flush=True)
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        report["status"] = "failed"
        report["error"] = str(error)
        print(str(error), file=sys.stderr)
        return 1
    finally:
        save()


if __name__ == "__main__":
    raise SystemExit(main())
