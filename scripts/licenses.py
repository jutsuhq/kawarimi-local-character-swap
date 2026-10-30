"""Writes src-tauri/resources/THIRD_PARTY_LICENSES.md: every Rust crate and npm package the app ships, with its
license text, and every model and tool it downloads, with its license and source.

    python scripts/licenses.py
"""

import json
import pathlib
import shutil
import subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "src-tauri" / "resources" / "THIRD_PARTY_LICENSES.md"
LICENSE_NAMES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT")


def run(*cmd):
    # pnpm is pnpm.cmd on Windows, which CreateProcess does not find by its bare name.
    cmd = (shutil.which(cmd[0]) or cmd[0],) + cmd[1:]
    return subprocess.run(cmd, cwd=ROOT, check=True, capture_output=True, text=True, encoding="utf-8").stdout


def license_texts(folder):
    texts = []
    for path in sorted(pathlib.Path(folder).iterdir()):
        if path.is_file() and path.name.upper().startswith(LICENSE_NAMES):
            texts.append(path.read_text(encoding="utf-8", errors="replace").strip())
    return texts


def crates():
    meta = json.loads(run("cargo", "metadata", "--format-version", "1", "--locked"))
    dirs = {(p["name"], p["version"]): (pathlib.Path(p["manifest_path"]).parent, p) for p in meta["packages"]}
    members = {p["name"] for p in meta["packages"] if p["id"] in meta["workspace_members"]}
    shipped = set()
    tree = run("cargo", "tree", "-p", "kawarimi", "-e", "normal,build", "--target", "all", "--prefix", "none", "-f", "{p}")
    for line in tree.splitlines():
        parts = line.replace(" (*)", "").split()
        if len(parts) >= 2 and parts[0] not in members:
            shipped.add((parts[0], parts[1].lstrip("v")))
    for key in sorted(shipped):
        folder, package = dirs[key]
        yield key[0], key[1], package.get("license") or package.get("license_file") or "see text", package.get("repository") or "", license_texts(folder)


def npm():
    data = json.loads(run("pnpm", "licenses", "ls", "--json", "--prod"))
    for license, packages in sorted(data.items()):
        for package in packages:
            texts = license_texts(package["paths"][0]) if package["paths"] else []
            yield package["name"], ", ".join(package["versions"]), license, package.get("homepage") or "", texts


def main():
    assets = json.loads(run("cargo", "run", "-q", "-p", "kawarimi-assets", "--bin", "kawarimi-fetch", "--", "--licenses"))
    out = ["# Third-party licenses", "",
           "Kawarimi is MIT-licensed. It is built from the packages below and downloads the models and tools listed",
           "first only when you ask, each pinned to an exact revision and checked against its SHA-256.", "",
           "## Models and tools downloaded by the app", "",
           "| Asset | License | Source |", "|---|---|---|"]
    out += [f"| {a['title']} | {a['license']} | {a['source']} |" for a in assets]
    out += ["", "FFmpeg is a separate program, downloaded as an LGPL-2.1-or-later build (BtbN, `lgpl` variant) and run as",
            "a child process; its source is at https://github.com/FFmpeg/FFmpeg. NVIDIA's CUDA and cuDNN libraries are",
            "redistributed under the NVIDIA software license agreements linked from the asset's source. ComfyUI's Python",
            "packages (PyTorch and the rest, under their own licenses) are installed by uv from PyPI and the PyTorch index",
            "into the app's ComfyUI folder, pinned by hash in crates/kawarimi-comfy/data/requirements-*.txt.", "",
            "## Font", "", "Barlow Condensed ExtraBold, SIL Open Font License 1.1:", "", "```",
            (ROOT / "src" / "assets" / "fonts" / "OFL.txt").read_text(encoding="utf-8").strip(), "```", ""]
    seen = {}
    for title, items in (("Rust crates", crates()), ("npm packages", npm())):
        out += [f"## {title}", ""]
        for name, version, license, home, texts in items:
            out.append(f"### {name} {version}")
            out.append(f"{license}" + (f" · {home}" if home else ""))
            for text in texts:
                ref = seen.setdefault(text, f"{name} {version}")
                if ref == f"{name} {version}":
                    out += ["", "```", text, "```"]
                else:
                    out.append(f"\nSame text as {ref}.")
            out.append("")
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text("\n".join(out) + "\n", encoding="utf-8", newline="\n")
    print(f"{OUT.relative_to(ROOT)}: {OUT.stat().st_size // 1024} KB")


if __name__ == "__main__":
    main()
