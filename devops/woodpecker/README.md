# Woodpecker CI/CD

CI = test + push images. Nothing here deploys: `after-build.yml` is terminal
and deploys stay manual (`aka up` / compose). Design and rationale: `PLAN.md`.

Server: https://ci.freshbrewlabs.cloud (deployed from `fbl-lab-devops/helm_charts/woodpecker`).
Registries: every release tag lands in Harbor `registry.freshbrewlabs.cloud`
(project `aka`, private) AND public Docker Hub `dewey4iv/aka-*` — buildx
pushes Harbor; a `hub-mirror` crane step copies the pushed index (same
digests, all platforms).

## State (server verified read-only 2026-10-15)

- Repo `freshbrewlabs/aka` is **active**, `trusted` (network/volumes/security —
  privileged build steps allowed), Pipeline path `devops/woodpecker/workflows/`,
  PR builds enabled (approval: forks). Nothing to activate.
- Org secrets on `freshbrewlabs`: `docker_username` / `docker_password` /
  `slack_webhook` (pre-existing) plus `hub_username` / `hub_password`
  (Docker Hub PAT, added 2026-10-15). Registry creds and the PAT exclude
  `pull_request`; the Slack webhook includes it. Verified live in the server
  DB (see One-time setup); re-land via `bin/upload_secrets` after rotation.
- Harbor project `aka` exists (private); `robot$cicd-bot` can push (and create
  projects). The mirror path was smoke-verified end-to-end 2026-10-15:
  harbor push → `crane copy` → hub `imagetools inspect` showed amd64+arm64
  with identical digests; smoke tags/repo deleted from both registries.
- Agents already allow step-level `nodeSelector` (server value set for rolodex).

## Workflows

| File | Trigger | What |
|---|---|---|
| lint-check.yml | PR (src paths), push main, tag `v*` | stable `cargo fmt --check` + clippy `-D warnings` |
| unit-specs.yml | same | `bin/unit_test.sh` |
| after-specs.yml | same | Slack notification gate; builds depend on it |
| aka-{proxy,dns}-build.yml | tag `v*` **only** | buildx → Harbor amd64+arm64 `:<aka-cli ver>,latest` → hub-mirror |
| aka-admin-build.yml | tag `v*` **only** | buildx (cross-linked) on an amd64 node → Harbor amd64+arm64 `:<admin-api ver>,latest` → hub-mirror |
| after-build.yml | tag `v*` | Slack notification; terminal marker |

A tag is the only thing that produces an image. Tags come from the crate
versions (`bin/version.sh` moves them all); `bin/build.sh`'s Dockerfile ARG
pins (`RUST_DEV_IMAGE_VERSION`, `DEBIAN_SLIM_BASE_IMAGE_VERSION`) are
duplicated in `aka-admin-build.yml` — bump in lockstep, same for `rust:1.99`
vs `rust-dev:v1.99.1`.

## One-time setup — DONE (verified 2026-10-15)

Server-side state was confirmed (and `hub_username`/`hub_password` inserted)
via a short guarded transaction on `woodpecker.sqlite` (`repos` row 7;
`secrets` rows 6–10) — the machine had no Woodpecker API token, so the CLI
path wasn't available. From now on the blessed paths are the UI and
`devops/woodpecker/bin/upload_secrets` (needs `woodpecker-cli setup
--server https://ci.freshbrewlabs.cloud --token <your token>` — the token is
per-user, from the Woodpecker UI settings).

## Releasing / the Docker Hub mirror

Nothing manual: every `v*` tag pushes to Harbor, then the `hub-mirror` step
copies each tag (`<version>` + `latest`) to `dewey4iv/aka-*` on Docker Hub —
which is what aka's default config (`DEFAULT_TOML` in
`src/aka/aka-services/src/config.rs`) names for login-free `aka pull`.
`build.sh` keeps Hub pushes opt-in (`HUB_NAMESPACE=dewey4iv bin/build.sh`)
for unreleased local experiments; CI does not run `build.sh`.

The mirror step downloads `crane` **pinned to v0.22.1** (asset
`go-containerregistry_Linux_x86_64.tar.gz`) — bump all three workflows
together when updating.

## Rotating/updating secrets

1. `devops/woodpecker/bin/decrypt` (keybase logged in, team freshbrewlabs)
2. edit `secrets/secrets.json`
3. `devops/woodpecker/bin/encrypt` → commit `secrets/secrets.json.enc`
4. after merge, on `main`: `devops/woodpecker/bin/upload_secrets`
   (`brew install woodpecker-cli jq`, one-time `woodpecker-cli setup`).

## Verifying

- PR touching `src/**/*.rs` → lint + unit run, after-specs aggregates + Slack.
- `git tag v0.3.4 && git push --tags` → specs first, then the three builds;
  check `aka/aka-{proxy,dns,admin}:0.3.4` + `:latest` in the Harbor UI **and**
  `dewey4iv/aka-*` on Docker Hub. `docker buildx imagetools inspect` both: all
  three images list amd64 **and** arm64 in each registry.
- Grep of `workflows/` for `kubectl|helm|argocd` must stay empty.

## Gotchas

- k8s steps have no docker socket: build workflows run
  `plugin-docker-buildx` with `privileged: true` (the plugin starts its own
  dockerd). This repo must stay **trusted** in Woodpecker, like rolodex's.
- proxy/dns are multi-arch via QEMU binfmt (config on top of angie/alpine —
  no compile). The admin image is multi-arch too, but by cross-linking: its
  Rust stages are `$BUILDPLATFORM`-pinned (`devops/docker/aka-admin/Dockerfile`)
  so nothing is emulated, and the amd64 node pin only keeps the compile itself
  off the Pis.
- Harbor project `aka` is **private**: anything pulling `aka-admin` etc. from
  the registry (compose files name it) needs `docker login` — that's why the
  Rust default config keeps pointing at the public hub mirror.
- The org `slack_webhook` is shared with rolodex (same channel); to give aka
  its own channel, add a **repo** secret `slack_webhook` — repo beats org.
- A push to a nonexistent Harbor project fails cryptically; `aka` exists.
- The hub mirror uses the PAT in `hub_password`; rotating it means updating
  `secrets/secrets.json` + `upload_secrets` (the keychain copy on workstations
  is separate). The hub repos must stay public (or `aka pull` breaks).
