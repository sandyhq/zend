# A build and test machine for this repository: what CI installs, plus the
# local tools docs/reference/testing.md names. Built and run by infra/dev/run.sh
# (the build context is scripts/ci, for the pinned nextest installer).
#
# Toolchains: stable (what rust-toolchain.toml pins, with rustfmt and clippy)
# and nightly (for the parallel frontend, --jobs-frontend; not for releases).
# run.sh picks one with RUSTUP_TOOLCHAIN, so rustup never updates a toolchain
# inside the container.
FROM ubuntu:24.04
ENV DEBIAN_FRONTEND=noninteractive

# CI's system packages (zgx-build.yml, release.yml, voice-tests.yml,
# zgx-tests.yml), node and python for test fixtures and scripts, and
# iproute2, passt (pasta), netcat-openbsd and bubblewrap for the zend-harness sandbox tests.
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends \
      build-essential curl git ca-certificates pkg-config cmake clang libclang-dev \
      meson ninja-build \
      libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
      libx11-dev libxcb1-dev libx11-xcb-dev \
      libfontconfig1-dev libfreetype-dev libasound2-dev \
      libvulkan-dev mesa-vulkan-drivers libwebkit2gtk-4.1-dev libjson-glib-dev libssl-dev \
      nodejs python3 xz-utils \
      iproute2 passt bubblewrap netcat-openbsd \
    && rm -rf /var/lib/apt/lists/*

# Everything lives under /opt and belongs to the ubuntu user (uid 1000, the
# owner's uid on the host), so files written into the mounted checkout are theirs.
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo KACHE_CACHE_DIR=/opt/kache-store \
    CARGO_TARGET_DIR=/opt/target PATH=/opt/cargo/bin:/opt/tools/bin:$PATH
# The volume mount points must exist here: an empty volume takes their owner.
RUN mkdir -p /opt/rustup /opt/cargo/registry /opt/cargo/git /opt/kache-store /opt/target /opt/tools/bin \
    && chown -R ubuntu:ubuntu /opt
USER ubuntu

ARG NIGHTLY=nightly
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable \
      --component rustfmt,clippy \
    && rustup toolchain install "$NIGHTLY" --profile minimal \
    && rustc +stable -V && rustc +"$NIGHTLY" -V

# cargo-nextest, pinned and checked by its SHA-256 (the script CI uses).
COPY --chown=ubuntu:ubuntu install-nextest.sh /tmp/install-nextest.sh
RUN HOME=/tmp/nx PATH=/tmp/nx/.cargo/bin:$PATH bash /tmp/install-nextest.sh && mv /tmp/nx/.cargo/bin/cargo-nextest /opt/tools/bin/ \
    && rm -rf /tmp/nx /tmp/install-nextest.sh

# kache, the compile cache (docs/reference/testing.md): the release's binary
# pair, checked by its SHA-256. Used as RUSTC_WRAPPER; it starts its daemon itself.
ARG KACHE_VERSION=2.2.0
ARG KACHE_SHA256=eeb6e72b9f55d0e9d61cae0789ffa8f3ab8c33bef166b7af655cc0806447ebac
RUN cd /tmp && curl -fsSL -o kache.tgz \
      "https://github.com/kunobi-ninja/kache/releases/download/v$KACHE_VERSION/kache-x86_64-unknown-linux-musl.tar.gz" \
    && echo "$KACHE_SHA256  kache.tgz" | sha256sum -c - \
    && tar -xzf kache.tgz -C /opt/tools/bin kache kache-engine && rm kache.tgz \
    && kache --version

# No blaze: it fails on this workspace (docs/reference/testing.md).

WORKDIR /src
