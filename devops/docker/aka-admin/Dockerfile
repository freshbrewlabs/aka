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
# Tagged and served for the builder's own platform: unlike aka-proxy/aka-dns
# (config on top of a published base, shipped multi-arch), this is a Rust and
# wasm compile, which is not cross-targeted here.

ARG RUST_DEV_IMAGE_VERSION=v1.98.0
ARG DEBIAN_SLIM_BASE_IMAGE_VERSION=trixie-slim

FROM registry.vizerapp.cloud/pub/rust-dev:${RUST_DEV_IMAGE_VERSION} AS build

# The same trunk + wasm toolchain pair as devops/docker/stylance (the dev
# container for admin-web), so `trunk build --release` here and `trunk serve`
# there produce the same bundle.
RUN cargo install trunk --git https://github.com/trunk-rs/trunk --tag v0.22.0-beta.2 \
    && rustup target add wasm32-unknown-unknown

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Registry + target caches: a rebuild after a source edit recompiles what
# changed instead of the whole crate graph. Both mounts stay out of the image;
# the artifacts are copied to ordinary paths before the mount is left behind.
RUN --mount=type=cache,id=aka-admin-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=aka-admin-cargo-target,target=/build/target \
    cargo build --release -p admin-api \
    && cp target/release/admin-api /usr/local/bin/admin-api \
    && cd src/admin/admin-web \
    && trunk build --release \
    && test -f dist/index.html

FROM debian:${DEBIAN_SLIM_BASE_IMAGE_VERSION}

COPY --from=build /usr/local/bin/admin-api /usr/local/bin/admin-api
COPY --from=build /build/src/admin/admin-web/dist /usr/share/aka-admin/web

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
