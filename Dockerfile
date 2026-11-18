# Build stage
# Debian trixie (glibc 2.40) is required because ort-sys downloads a
# prebuilt ONNX Runtime 1.24 binary that references C23 symbols
# (__isoc23_strtoll) absent from glibc < 2.38. Bookworm (glibc 2.36)
# fails at link time with "undefined symbol: __isoc23_strtoll".
FROM rust:trixie AS builder

# `uv` provisions the pinned componentize-py toolchain that build.rs uses to
# compile the Python sandbox component (src/ai/tools/python/guest.py). The
# resulting wasm is architecture-independent, so it can be built on either the
# arm64 host or an amd64 builder.
COPY --from=ghcr.io/astral-sh/uv:0.13.5 /uv /uvx /bin/

# The committed bin/uv is a macOS arm64 binary, so the builder points build.rs
# at the Linux build instead. build.rs has no fallback to a uv on PATH.
ENV HQ_UV=/bin/uv

# The package must not live at the filesystem root. Cargo walks the package
# directory to fingerprint it whenever a build script is present (build.rs
# generates the migration registry), so `WORKDIR /` makes it stat the whole
# container — /proc, /sys, the toolchain, and the multi-GB target dir — which
# exhausts memory on the build VM.
WORKDIR /app

# Keep build artifacts outside the package root so that fingerprint walk never
# descends into them. The runner stage copies from this stable path.
ENV CARGO_TARGET_DIR=/target

## Install native dependencies + x86_64 cross-compilation toolchain
## (Dokku server is linux/amd64, but colima on Apple Silicon is arm64)
RUN dpkg --add-architecture amd64 && \
    apt-get update && apt-get install -y --no-install-recommends \
    libssl-dev \
    libssl-dev:amd64 \
    pkg-config \
    clang \
    gcc-x86-64-linux-gnu \
    g++-x86-64-linux-gnu \
    && rm -rf /var/lib/apt/lists/*

## Add x86_64 Rust target and configure cargo for cross-compilation
RUN rustup target add x86_64-unknown-linux-gnu && \
    mkdir -p ~/.cargo && \
    printf '[target.x86_64-unknown-linux-gnu]\nlinker = "x86_64-linux-gnu-gcc"\n' > ~/.cargo/config.toml

## Set env vars for cc-rs / C dependencies to use the cross-compiler
ENV CC_x86_64_unknown_linux_gnu=x86_64-linux-gnu-gcc
ENV CXX_x86_64_unknown_linux_gnu=x86_64-linux-gnu-g++
ENV AR_x86_64_unknown_linux_gnu=x86_64-linux-gnu-ar
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc
ENV PKG_CONFIG_ALLOW_CROSS=1
ENV PKG_CONFIG_PATH=/usr/lib/x86_64-linux-gnu/pkgconfig

## Cache rust dependencies
## https://stackoverflow.com/questions/58473606/cache-rust-dependencies-with-docker-build
## --locked ensures the build uses Cargo.lock exactly as committed, with no registry resolution drift.
## This is a Cargo workspace (root lib + crates/bash, crates/api, crates/cli,
## crates/search), so every manifest must exist and every package needs a
## placeholder source file before the dependency-only build can resolve.
RUN mkdir -p ./src ./crates/bash/src ./crates/api/src ./crates/cli/src ./crates/search/src
COPY ./Cargo.toml ./Cargo.lock ./
COPY ./crates/bash/Cargo.toml ./crates/bash/
COPY ./crates/api/Cargo.toml ./crates/api/
COPY ./crates/cli/Cargo.toml ./crates/cli/
COPY ./crates/search/Cargo.toml ./crates/search/
# build.rs generates the migration registry into OUT_DIR, which src/core/db/migrations
# includes via `env!("OUT_DIR")`. Without it Cargo sets no OUT_DIR and the build fails.
COPY ./build.rs ./build.rs
COPY ./src/core/db/migrations ./src/core/db/migrations
# build.rs also compiles the Python sandbox component into OUT_DIR, so the
# guest source and WIT must be present for the dependency-only build too.
COPY ./src/ai/tools/python/guest.py ./src/ai/tools/python/guest.py
COPY ./src/ai/tools/python/sandbox.wit ./src/ai/tools/python/sandbox.wit
RUN echo '' > ./src/lib.rs && \
    echo '' > ./crates/bash/src/lib.rs && \
    echo '' > ./crates/api/src/lib.rs && \
    echo '' > ./crates/cli/src/lib.rs && \
    echo '' > ./crates/search/src/lib.rs && \
    echo 'fn main() {}' > ./crates/cli/src/main.rs
# The stubbed CLI source references none of its dependencies, so Cargo's
# workspace-wide unused_dependencies lint flags the ones used only by the CLI
# (hq-api, rustyline). They are used by the real sources, so allow the lint for
# this dependency-only build and restore the manifest for the real build.
RUN cp ./crates/cli/Cargo.toml /tmp/cli-Cargo.toml && \
    printf '\n[lints.cargo]\nunused_dependencies = "allow"\n' >> ./crates/cli/Cargo.toml && \
    cargo build --release --target x86_64-unknown-linux-gnu --locked --features embed-assets -p hq-cli && \
    mv /tmp/cli-Cargo.toml ./crates/cli/Cargo.toml

## Actually build the app
COPY ./src ./src
COPY ./crates ./crates
COPY ./build.rs ./build.rs
# Assets are embedded into the binary at compile time; copying them here lets
# the include_dir! macro read them and busts the layer cache when assets change.
COPY ./web-ui ./web-ui
RUN touch -a -m ./src/lib.rs ./crates/*/src/lib.rs
RUN cargo build --release --target x86_64-unknown-linux-gnu --locked --features embed-assets -p hq-cli

# Run stage
# Match builder's glibc (see note above re: __isoc23_strtoll).
FROM --platform=linux/amd64 debian:trixie-slim AS runner

RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    openssh-client \
    ca-certificates \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Use the compiled binary rather than cargo
# Web assets are embedded in the binary (`--features embed-assets`), so no
# web-ui directory is needed at runtime.
COPY --from=builder /target/x86_64-unknown-linux-gnu/release/hq /hq

# Applies pending migrations before starting the server. Dokku passes its start
# command (`serve --host 0.0.0.0 --port 2222`) to the entrypoint as arguments,
# so schema changes ship with each deploy. Other commands pass through
# unchanged, which keeps `dokku run` usable.
COPY ./bin/docker-entrypoint.sh /docker-entrypoint.sh
RUN chmod +x /docker-entrypoint.sh

EXPOSE 2222

ENTRYPOINT ["/docker-entrypoint.sh"]
