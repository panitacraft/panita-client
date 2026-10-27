# Build environment only. BOX is reserved for installed-product testing.
FROM ubuntu:24.04@sha256:68ee22004a9109b44973313b02ff7b1de250f1de0f5ecb628d73f579aaff33ac

ARG DEBIAN_FRONTEND=noninteractive
RUN apt-get update && apt-get install --no-install-recommends -y \
    build-essential ca-certificates curl wget file git xz-utils pkg-config \
    libwebkit2gtk-4.1-dev libssl-dev libxdo-dev libayatana-appindicator3-dev \
    librsvg2-dev patchelf desktop-file-utils xdg-utils \
    && rm -rf /var/lib/apt/lists/*

ARG NODE_VERSION=24.16.0
ARG PNPM_VERSION=11.25.0
RUN cd /tmp \
    && curl -fsSLO "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
    && curl -fsSLO "https://nodejs.org/dist/v${NODE_VERSION}/SHASUMS256.txt" \
    && grep " node-v${NODE_VERSION}-linux-x64.tar.xz$" SHASUMS256.txt | sha256sum --check \
    && tar -xJf "node-v${NODE_VERSION}-linux-x64.tar.xz" --strip-components=1 -C /usr/local \
    && npm install --global "pnpm@${PNPM_VERSION}" \
    && rm "node-v${NODE_VERSION}-linux-x64.tar.xz" SHASUMS256.txt

RUN useradd --create-home --uid 10001 --shell /bin/bash panita-build \
    && mkdir -p /work /build-target /home/panita-build/.cargo/registry /home/panita-build/.cargo/git \
    && chown -R panita-build:panita-build /work /build-target /home/panita-build
USER panita-build
ENV CARGO_HOME=/home/panita-build/.cargo RUSTUP_HOME=/home/panita-build/.rustup
ENV PATH="/home/panita-build/.cargo/bin:${PATH}"
ARG RUST_VERSION=1.96.0
RUN curl -fsSL https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init -o /tmp/rustup-init \
    && curl -fsSL https://static.rust-lang.org/rustup/dist/x86_64-unknown-linux-gnu/rustup-init.sha256 -o /tmp/rustup-init.sha256 \
    && printf '%s  /tmp/rustup-init\n' "$(cut -d ' ' -f 1 /tmp/rustup-init.sha256)" | sha256sum --check \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --profile minimal --default-toolchain "${RUST_VERSION}" --component rustfmt --component clippy \
    && rm /tmp/rustup-init /tmp/rustup-init.sha256
WORKDIR /work
