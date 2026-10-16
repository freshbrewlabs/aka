ARG TARGETVERSION=v1.99.1

FROM registry.freshbrewlabs.cloud/base/rust-dev:$TARGETVERSION

ARG TARGETPLATFORM

RUN cargo install trunk --git https://github.com/trunk-rs/trunk --tag v0.22.0-beta.2
RUN cargo install stylance-cli
RUN rustup target add wasm32-unknown-unknown

EXPOSE 80