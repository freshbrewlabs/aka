### [6](https://git.freshbrewlabs.cloud/freshbrewlabs/aka/issues/6) - update base images

#### Changed

- bump `RUST_DEV_IMAGE_VERSION` from `v1.99.1` to `v1.100.0`
- replace the floating `trixie-slim` prod base with pinned `13.7-2026.46.0` and switch the aka-admin runtime stage FROM from Docker Hub `debian:` to `registry.freshbrewlabs.cloud/base/debian:`
