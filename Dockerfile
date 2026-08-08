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
RUN cargo build --release --target x86_64-unknown-linux-gnu --locked

## Actually build the app
RUN rm -rf ./src
COPY ./src ./src
RUN touch -a -m ./src/main.rs
RUN cargo build --release --target x86_64-unknown-linux-gnu --locked

# Run stage
# Match builder's glibc (see note above re: __isoc23_strtoll).
FROM --platform=linux/amd64 debian:trixie-slim AS runner

RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    ca-certificates \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/*

# Use the compiled binary rather than cargo
COPY --from=builder /target/x86_64-unknown-linux-gnu/release/hq /hq

# Copy over static files for the web UI, currently these are built and
# checked into the repo but that might change later
COPY ./web-ui/src/index.html ./web-ui/src/index.html
COPY ./web-ui/src/search/index.html ./web-ui/src/search/index.html
COPY ./web-ui/src/search/index.js ./web-ui/src/search/index.js
COPY ./web-ui/src/metrics/index.html ./web-ui/src/metrics/index.html
COPY ./web-ui/src/metrics/index.js ./web-ui/src/metrics/index.js
COPY ./web-ui/src/chat/index.html ./web-ui/src/chat/index.html
COPY ./web-ui/src/chat/index.js ./web-ui/src/chat/index.js
COPY ./web-ui/src/chat/message-bubble.js ./web-ui/src/chat/message-bubble.js
COPY ./web-ui/src/chat/img/ ./web-ui/src/chat/img/
COPY ./web-ui/src/chat/sessions/index.html ./web-ui/src/chat/sessions/index.html
COPY ./web-ui/src/chat/sessions/index.js ./web-ui/src/chat/sessions/index.js
COPY ./web-ui/src/output.css ./web-ui/src/output.css
COPY ./web-ui/src/skills/index.html ./web-ui/src/skills/index.html
COPY ./web-ui/src/skills/index.js ./web-ui/src/skills/index.js
COPY ./web-ui/src/favicon.ico ./web-ui/src/favicon.ico
COPY ./web-ui/src/icon512_maskable.png ./web-ui/src/icon512_maskable.png
COPY ./web-ui/src/manifest.json ./web-ui/src/manifest.json
COPY ./web-ui/src/service-worker.js ./web-ui/src/service-worker.js
COPY ./web-ui/src/vendor/marked.min.js ./web-ui/src/vendor/marked.min.js
COPY ./web-ui/src/vendor/highlight.min.js ./web-ui/src/vendor/highlight.min.js
COPY ./web-ui/src/vendor/echarts.simple.min.js ./web-ui/src/vendor/echarts.simple.min.js
COPY ./web-ui/src/vendor/codemirror/codemirror.min.js ./web-ui/src/vendor/codemirror/codemirror.min.js
COPY ./web-ui/src/vendor/codemirror/codemirror.min.css ./web-ui/src/vendor/codemirror/codemirror.min.css
COPY ./web-ui/src/vendor/codemirror/theme/dracula.min.css ./web-ui/src/vendor/codemirror/theme/dracula.min.css
COPY ./web-ui/src/vendor/codemirror/mode/javascript.min.js ./web-ui/src/vendor/codemirror/mode/javascript.min.js
COPY ./web-ui/src/vendor/codemirror/mode/python.min.js ./web-ui/src/vendor/codemirror/mode/python.min.js
COPY ./web-ui/src/vendor/codemirror/mode/markdown.min.js ./web-ui/src/vendor/codemirror/mode/markdown.min.js
COPY ./web-ui/src/vendor/codemirror/mode/shell.min.js ./web-ui/src/vendor/codemirror/mode/shell.min.js
COPY ./web-ui/src/vendor/codemirror/mode/xml.min.js ./web-ui/src/vendor/codemirror/mode/xml.min.js
COPY ./web-ui/src/vendor/codemirror/mode/yaml.min.js ./web-ui/src/vendor/codemirror/mode/yaml.min.js

EXPOSE 2222

# Default command with run in docker so we can use `dokku run`
# Need to update $DOKKU_DOCKERFILE_START_CMD so that the server starts
#  with `./hq serve --host 0.0.0.0 --port 2222`
ENTRYPOINT ["./hq"]
