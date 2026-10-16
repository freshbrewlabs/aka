# aka's status dashboard as a single container: the axum API plus the release
# build of the dashboard it serves. One origin, one port — the browser talks to
# the API at the same host that served the page, and the API talks to the docker
# engine plus aka's state directory (both mounted at runtime, neither written).
#
# Build context is the repo root, because admin-api reads aka's state through
# aka's own crates:
#
#   docker build -f devops/docker/aka-admin/Dockerfile -t aka-admin .
#   src/admin/build.sh wraps this (and bin/build.sh runs that)
#
# Ships linux/amd64 + linux/arm64 like aka-proxy/aka-dns, but without ever
# emulating a compile: every Rust stage is pinned to the builder's own platform
# and the API is *cross-linked* for the target (rust-dev carries both
# unknown-linux-gnu rustup targets and both cross gcc's, with a cross glibc at
# trixie's 2.41 — the runtime base's own), and the dashboard is wasm, which is
# architecture-independent. The runtime stage has no RUN, so assembling the
# foreign image never executes foreign code. Multi-arch needs a buildx
# invocation with both platforms (src/admin/build.sh); a plain `docker build`
# still yields the builder's own arch.

ARG RUST_DEV_IMAGE_VERSION=v1.99.1
ARG DEBIAN_SLIM_BASE_IMAGE_VERSION=trixie-slim

# The dashboard bundle. Builder-pinned, and its instructions are identical for
# every target platform, so BuildKit resolves this stage once per multi-arch
# invocation instead of once per arch: neither the `cargo install trunk` below
# nor `trunk build` is ever repeated for a second architecture.
FROM --platform=$BUILDPLATFORM registry.freshbrewlabs.cloud/base/rust-dev:${RUST_DEV_IMAGE_VERSION} AS web

# The same trunk + wasm toolchain pair as devops/docker/stylance (the dev
# container for admin-web), so `trunk build --release` here and `trunk serve`
# there produce the same bundle.
RUN cargo install trunk --git https://github.com/trunk-rs/trunk --tag v0.22.0-beta.2 \
    && rustup target add wasm32-unknown-unknown

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Registry + target caches, so a rebuild after a source edit recompiles what
# changed instead of the whole crate graph; the mount stays out of the image
# (trunk's `dist/` lands under /build/src, off the mount).
RUN --mount=type=cache,id=aka-admin-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=aka-admin-web-target,target=/build/target \
    cd src/admin/admin-web \
    && trunk build --release \
    && test -f dist/index.html

# The API: compiled for the builder, linked for the target. The target dir is
# per-arch so the two platform variants share one cache mount without
# serialising on cargo's build-directory lock.
FROM --platform=$BUILDPLATFORM registry.freshbrewlabs.cloud/base/rust-dev:${RUST_DEV_IMAGE_VERSION} AS build

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src

ARG BUILDARCH
ARG TARGETARCH

RUN --mount=type=cache,id=aka-admin-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=aka-admin-cargo-target,target=/build/cargo-target \
    set -eux; \
    case "${TARGETARCH}" in \
      amd64) RUST_TARGET=x86_64-unknown-linux-gnu ;; \
      arm64) RUST_TARGET=aarch64-unknown-linux-gnu ;; \
      *) echo "aka-admin: no Rust target for linux/${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    if [ "${TARGETARCH}" = arm64 ] && [ "${BUILDARCH}" != arm64 ]; then \
      export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc; \
    fi; \
    if [ "${TARGETARCH}" = amd64 ] && [ "${BUILDARCH}" != amd64 ]; then \
      export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc; \
    fi; \
    CARGO_TARGET_DIR="/build/cargo-target/${TARGETARCH}" \
      cargo build --release -p admin-api --target "${RUST_TARGET}"; \
    cp "/build/cargo-target/${TARGETARCH}/${RUST_TARGET}/release/admin-api" \
      /usr/local/bin/admin-api

FROM debian:${DEBIAN_SLIM_BASE_IMAGE_VERSION}

COPY --from=build /usr/local/bin/admin-api /usr/local/bin/admin-api
COPY --from=web /build/src/admin/admin-web/dist /usr/share/aka-admin/web

# One origin, aka's container port; the dashboard build it serves and aka's
# runtime home (mount ${HOME}/.aka there) are the other two defaults.
ENV PORT=80 \
    ADMIN_WEB_DIR=/usr/share/aka-admin/web \
    AKA_HOME=/aka/home

# Runs as root: it needs the docker socket, and docker-group access is already
# root-equivalent on the host, so a non-root user here would only break reads of
# the host-owned state directory.
EXPOSE 80

ENTRYPOINT ["/usr/local/bin/admin-api"]
