"""Extract every Rust README fence for compilation by a packaged consumer."""
import hashlib
import json
import re
from pathlib import Path


def prepare(readme: Path, consumer: Path) -> dict:
    blocks = re.findall(r"^```rust\n(.*?)^```", readme.read_text(encoding="utf-8"), re.M | re.S)
    if len(blocks) != 9:
        raise ValueError(f"README Rust fence count changed: expected 9, found {len(blocks)}; classify every new block")
    expectations = ["#[tokio::main]", "// Launch,", "pub async fn fill_and_read", "let mut pages", "BidiBrowser::launch", "#[rustwright_test]", "to_have_text", ".with_timeout", "Chrome::at"]
    for index, expected in enumerate(expectations):
        if expected not in blocks[index]:
            raise ValueError(f"README block {index} needs classification: missing {expected}")
    src = consumer / "src"
    (src / "bin").mkdir(parents=True, exist_ok=True)
    outputs = {
        "src/bin/readme_quickstart.rs": blocks[0],
        "src/readme_generic.rs": blocks[2],
        "src/bin/readme_firefox.rs": blocks[4],
    }
    header = "#![allow(dead_code, unused_variables)]\nuse rustwright::prelude::*;\nuse std::time::Duration;\nuse rustwright_test::expect;\n"
    checks = header + "pub async fn api_sketch() -> rustwright::Result<()> {\n" + blocks[1] + "\nOk(())\n}\n"
    checks += "pub async fn dynamic_pages(browser: &Browser, bidi_browser: &BidiBrowser) -> std::result::Result<(), AnyError> {\n" + blocks[3] + "\nOk(())\n}\n"
    checks += "pub async fn assertions(page: &AnyPage) -> rustwright_test::Result<()> {\n" + blocks[6] + "\n" + blocks[7] + "\nOk(())\n}\n"
    checks += "pub fn browser_configuration() {\n" + blocks[8] + "\n}\n"
    checks += "mod runner {\n" + blocks[5] + "\n}\n"
    outputs["src/readme_checks.rs"] = checks
    outputs["tests/readme_compile.rs"] = '#[path = "../src/readme_checks.rs"]\nmod readme_checks;\n#[path = "../src/readme_generic.rs"]\nmod readme_generic;\n'
    for relative, text in outputs.items():
        destination = consumer / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text, encoding="utf-8")
    record = {
        "readme_sha256": hashlib.sha256(readme.read_bytes()).hexdigest(),
        "rust_blocks": len(blocks),
        "all_blocks_classified": True,
        "wrapping": "Standalone entry points and generic helper copied verbatim; illustrative fragments only gain imports/function context and scoped unused/dead-code allowances. Runner compiled but not executed by browser smoke selection.",
        "generated_sha256": {name: hashlib.sha256((consumer / name).read_bytes()).hexdigest() for name in outputs},
    }
    (consumer / "readme-snippets.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    return record


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("readme", type=Path)
    parser.add_argument("consumer", type=Path)
    args = parser.parse_args()
    print(json.dumps(prepare(args.readme, args.consumer)))
