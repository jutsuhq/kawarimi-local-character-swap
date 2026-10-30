#!/usr/bin/env bash
# Runs a command in Ubuntu 24.04 with the Linux toolchain (image from scripts/docker/Dockerfile, built once), on an
# export of HEAD (no host node_modules or target), with caches in Docker volumes. Uncommitted changes are not in it.
#
#     scripts/ubuntu.sh 'bash scripts/ci.sh' [<host folder mounted at /out>]
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
docker build -q -t kawarimi-ubuntu "$root/scripts/docker" > /dev/null
out=()
if [[ -n "${2:-}" ]]; then
    mkdir -p "$2"
    out=(-v "$(cd "$2" && { pwd -W 2>/dev/null || pwd; }):/out")
fi
# Git Bash would rewrite the container paths below into Windows ones (/target -> C:/Program Files/Git/target).
# AppImage tools run without FUSE inside a container only when they extract themselves.
git -C "$root" archive --format=tar HEAD | MSYS_NO_PATHCONV=1 docker run --rm -i "${out[@]}" \
    -v kawarimi-cargo:/root/.cargo/registry -v kawarimi-target:/target -v kawarimi-pnpm:/root/.local/share/pnpm \
    -e CARGO_TARGET_DIR=/target -e APPIMAGE_EXTRACT_AND_RUN=1 kawarimi-ubuntu bash -c '
        set -euo pipefail
        mkdir /src && tar -x -C /src && cd /src
        '"$1"
