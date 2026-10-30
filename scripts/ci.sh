#!/usr/bin/env bash
# The CI steps (.github/workflows/ci.yml), runnable on a developer machine:
#
#     scripts/ci.sh                 # here: Windows (Git Bash) or Linux
#     scripts/ci.sh --docker        # Ubuntu 24.04 in Docker, from a clean export of HEAD
#
# GitHub Actions is not available on the current repository's account, so this is how both platforms are checked.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ "${1:-}" == "--docker" ]]; then
    exec bash "$root/scripts/ubuntu.sh" 'bash scripts/ci.sh'
fi

cd "$root"
pnpm install --frozen-lockfile
pnpm build
# FFmpeg through the app's own pinned download, so the decoding test runs instead of skipping.
cargo run -q -p kawarimi-assets --bin kawarimi-fetch -- --dir .models ffmpeg
KAWARIMI_FFMPEG_DIR="$root/.models/ffmpeg" cargo test --workspace
cargo build -p kawarimi
echo "ci: all steps passed on $(uname -s)"
