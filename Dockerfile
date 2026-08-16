# Build stage
# Debian trixie (glibc 2.40) is required because ort-sys downloads a
# prebuilt ONNX Runtime 1.24 binary that references C23 symbols
# (__isoc23_strtoll) absent from glibc < 2.38. Bookworm (glibc 2.36)
# fails at link time with "undefined symbol: __isoc23_strtoll".
FROM rust:trixie AS builder

WORKDIR /

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
RUN mkdir ./src && echo 'fn main() { println!("Dummy!"); }' > ./src/main.rs
COPY ./Cargo.toml .
COPY ./Cargo.lock .
RUN cargo build --release --target x86_64-unknown-linux-gnu --locked --features embed-assets

## Actually build the app
RUN rm -rf ./src
COPY ./src ./src
# Assets are embedded into the binary at compile time; copying them here lets
# the include_dir! macro read them and busts the layer cache when assets change.
COPY ./web-ui ./web-ui
RUN touch -a -m ./src/main.rs
RUN cargo build --release --target x86_64-unknown-linux-gnu --locked --features embed-assets

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

EXPOSE 2222

# Default command with run in docker so we can use `dokku run`
# Need to update $DOKKU_DOCKERFILE_START_CMD so that the server starts
#  with `./hq serve --host 0.0.0.0 --port 2222`
ENTRYPOINT ["./hq"]
