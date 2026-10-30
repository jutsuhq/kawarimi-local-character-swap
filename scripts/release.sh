#!/usr/bin/env bash
# Builds the installers and their SHA-256 sums into release/<version>/, and prints the command that publishes them.
# Nothing is uploaded. Windows (NSIS) is built here, Linux (AppImage, deb) in Docker (scripts/ubuntu.sh).
#
#     scripts/release.sh [windows] [linux]          # both by default, each only on a host that can build it
#
# Signing is optional: KAWARIMI_SIGN_COMMAND (Tauri's bundle.windows.signCommand, e.g. a signtool or
# relic command with %1 for the file) signs the Windows binaries; without it they are unsigned.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
version="$(node -p "require('./src-tauri/tauri.conf.json').version")"
out="$root/release/$version"
platforms=("${@:-windows linux}")
platforms=(${platforms[*]})
mkdir -p "$out"

# ONNX Runtime is linked in, but its CUDA provider is a library it loads from the executable's folder.
stage_ort() {
    rm -rf src-tauri/resources/ort && mkdir -p src-tauri/resources/ort
    for lib in "$@"; do cp "$lib" src-tauri/resources/ort/; done
}

if [[ " ${platforms[*]} " == *" windows "* && "$(uname -s)" == MINGW* ]]; then
    target="${CARGO_TARGET_DIR:-$root/target}"
    python3 scripts/licenses.py
    pnpm install --frozen-lockfile
    cargo build --release -p kawarimi
    stage_ort "$target/release/onnxruntime_providers_cuda.dll" "$target/release/onnxruntime_providers_shared.dll"
    extra=()
    if [[ -n "${KAWARIMI_SIGN_COMMAND:-}" ]]; then
        extra=(--config "$(node -e 'console.log(JSON.stringify({bundle:{windows:{signCommand:process.argv[1]}}}))' "$KAWARIMI_SIGN_COMMAND")")
    fi
    pnpm tauri build --bundles nsis --config src-tauri/tauri.release.windows.json "${extra[@]}"
    cp "$target"/release/bundle/nsis/*.exe "$out/"
fi

if [[ " ${platforms[*]} " == *" linux "* ]]; then
    bash scripts/ubuntu.sh '
        pnpm install --frozen-lockfile
        python3 scripts/licenses.py
        cargo build --release -p kawarimi
        mkdir -p src-tauri/resources/ort
        cp /target/release/libonnxruntime_providers_cuda.so /target/release/libonnxruntime_providers_shared.so src-tauri/resources/ort/
        # The strip inside linuxdeploy fails on sections current toolchains write; the binary is already stripped.
        # CUDA comes from the cuda-runtime asset at run time and libcuda from the driver, so the AppImage leaves them out.
        export LINUXDEPLOY_EXCLUDED_LIBRARIES="libcuda.so*;libcudart.so*;libcublas.so*;libcublasLt.so*;libcurand.so*"
        NO_STRIP=true pnpm tauri build --verbose --bundles deb,appimage --config src-tauri/tauri.release.linux.json
        cp /target/release/bundle/appimage/*.AppImage /target/release/bundle/deb/*.deb /out/' "$out"
fi

(cd "$out" && sha256sum -- *.exe *.AppImage *.deb 2>/dev/null > SHA256SUMS || true)
echo "Built into $out:"
cat "$out/SHA256SUMS"
echo
echo "To publish (after checking them on clean machines, docs/RELEASE.md):"
echo "  gh release create v$version $out/* --title \"Kawarimi $version\" --notes-file docs/RELEASE.md"
