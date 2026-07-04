# Build stage
# Debian trixie (glibc 2.40) is required because ort-sys downloads a
# prebuilt ONNX Runtime 1.24 binary that references C23 symbols
# (__isoc23_strtoll) absent from glibc < 2.38. Bookworm (glibc 2.36)
# fails at link time with "undefined symbol: __isoc23_strtoll".
FROM rust:trixie AS builder

WORKDIR /

## Install native dependencies needed for linking
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl-dev \
    pkg-config \
    clang \
    && rm -rf /var/lib/apt/lists/*

## Use lld for faster, more memory-efficient linking (critical on small VMs)
RUN rustup component add llvm-tools-preview 2>/dev/null; \
    mkdir -p ~/.cargo && \
    printf '[target.x86_64-unknown-linux-gnu]\nrustflags = ["-C", "link-arg=-fuse-ld=lld"]\n' > ~/.cargo/config.toml

## Cache rust dependencies
## https://stackoverflow.com/questions/58473606/cache-rust-dependencies-with-docker-build
## --locked ensures the build uses Cargo.lock exactly as committed, with no registry resolution drift.
RUN mkdir ./src && echo 'fn main() { println!("Dummy!"); }' > ./src/main.rs
COPY ./Cargo.toml .
COPY ./Cargo.lock .
RUN cargo build --release --locked

## Actually build the app
RUN rm -rf ./src
COPY ./src ./src
RUN touch -a -m ./src/main.rs
RUN cargo build --release --locked

# Run stage
# Match builder's glibc (see note above re: __isoc23_strtoll).
FROM debian:trixie-slim AS runner

RUN apt update
RUN apt install -y git

# Use the compiled binary rather than cargo
COPY --from=builder /target/release/hq /hq

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
