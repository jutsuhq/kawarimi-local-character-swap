# The Linux toolchain for scripts/ubuntu.sh: built once, reused by CI and the release build.
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update -qq && apt-get install -y -qq curl ca-certificates build-essential pkg-config libssl-dev git \
        xz-utils file python3 libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev libdbus-1-dev patchelf \
    && rm -rf /var/lib/apt/lists/*
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
RUN curl -fsSL https://nodejs.org/dist/v24.19.0/node-v24.19.0-linux-x64.tar.xz | tar -xJ -C /opt
ENV PATH="/root/.cargo/bin:/opt/node-v24.19.0-linux-x64/bin:$PATH"
RUN npm install -g -s pnpm@11.22.0
