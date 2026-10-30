#!/usr/bin/env bash
# Locks ComfyUI's Python environment for each platform, with hashes, from crates/kawarimi-comfy/data/requirements.in.
# The app installs exactly these files (uv pip install --require-hashes), so a rebuild picks up nothing new.
set -euo pipefail
data="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/crates/kawarimi-comfy/data"
for platform in x86_64-pc-windows-msvc x86_64-unknown-linux-gnu; do
    uv pip compile "$data/requirements.in" --python-version 3.12 --python-platform "$platform" --generate-hashes \
        --index https://download.pytorch.org/whl/cu130 --index-strategy unsafe-best-match --no-header \
        -o "$data/requirements-$platform.txt" -q
done
echo "locked: $(ls "$data"/requirements-*.txt | tr '\n' ' ')"
