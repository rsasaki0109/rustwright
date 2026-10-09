"""Run verbatim README entry points from the packaged consumer over loopback HTTP."""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading

ROOT = Path(__file__).resolve().parents[2]
TITLE = "README 配布チェック"
BODY = f"<!DOCTYPE html><meta charset=utf-8><title>{TITLE}</title><h1>Local Rustwright</h1>".encode()


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(BODY)))
        self.end_headers()
        self.wfile.write(BODY)

    def log_message(self, *_):
        pass


def run(output: Path):
    consumer = output / "consumer"
    snippets = json.loads((consumer / "readme-snippets.json").read_text(encoding="utf-8"))
    current = hashlib.sha256((ROOT / "README.md").read_bytes()).hexdigest()
    if snippets["readme_sha256"] != current:
        raise ValueError("README changed after extraction; rerun release_check.py first")
    for relative, expected in snippets["generated_sha256"].items():
        if hashlib.sha256((consumer / relative).read_bytes()).hexdigest() != expected:
            raise ValueError(f"Extracted README source changed: {relative}")
    for key in ["RUSTWRIGHT_CHROME", "RUSTWRIGHT_FIREFOX"]:
        if not Path(os.environ.get(key, "missing-browser")).is_file():
            raise ValueError(f"An explicit installed {key} is required; no skip")
    cargo = shutil.which("cargo")
    if cargo is None:
        raise ValueError("Activate Cargo before running this check")
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    records = []
    try:
        for backend, binary in [("chrome", "readme_quickstart"), ("firefox", "readme_firefox")]:
            with tempfile.TemporaryDirectory(prefix=f"readme-{backend}-", dir=output) as profile:
                env = dict(os.environ, CARGO_HOME=str(output / "consumer-cargo-home"), CARGO_TARGET_DIR=str(output / "consumer-target-stable"), RUSTWRIGHT_PROFILE=profile)
                command = [cargo, "+stable", "run", "--offline", "--locked", "--bin", binary, "--", f"http://127.0.0.1:{server.server_port}/"]
                log = output / f"{binary}-native.log"
                screenshot = consumer / "firefox.png"
                if backend == "firefox":
                    screenshot.unlink(missing_ok=True)
                with log.open("w", encoding="utf-8") as stream:
                    result = subprocess.run(command, cwd=consumer, env=env, stdout=stream, stderr=subprocess.STDOUT, timeout=45)
                text = log.read_text(encoding="utf-8")
                if result.returncode != 0 or TITLE not in text:
                    raise RuntimeError(f"README {binary} failed ({result.returncode}); see {log}")
                if backend == "firefox":
                    if "Local Rustwright" not in text:
                        raise RuntimeError("Firefox README locator did not read the fixture heading")
                    png = screenshot.read_bytes()
                    if not png.startswith(b"\x89PNG\r\n\x1a\n") or len(png) < 24:
                        raise RuntimeError("Firefox README screenshot is not PNG")
                    if min(int.from_bytes(png[16:20], "big"), int.from_bytes(png[20:24], "big")) < 200:
                        raise RuntimeError("Firefox README screenshot dimensions are too small")
                record = {"backend": backend, "binary": binary, "exit_code": result.returncode, "title": TITLE, "headless": True, "snippet_verbatim": True, "fixture": "loopback HTTP", "profile_mode": "explicit disposable" if backend == "firefox" else "default ephemeral", "log": str(log)}
                if backend == "firefox":
                    record["screenshot_sha256"] = hashlib.sha256(png).hexdigest()
                records.append(record)
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)
    (output / "readme-runtime.json").write_text(json.dumps({"status": "passed", "readme_sha256": current, "cases": records}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(f"README runtime checks passed: {len(records)}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "target/release-check")
    output = parser.parse_args().output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    summary = output / "readme-runtime.json"
    summary.write_text(json.dumps({"status": "running"}) + "\n", encoding="utf-8")
    try:
        run(output)
    except Exception as error:
        summary.write_text(json.dumps({"status": "failed", "error": str(error)}) + "\n", encoding="utf-8")
        raise


if __name__ == "__main__":
    main()
