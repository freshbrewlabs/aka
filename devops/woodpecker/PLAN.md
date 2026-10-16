# Woodpecker CI/CD Plan (devops/woodpecker)

Modeled on `~/dev/freshbrewlabs/rolodex/devops/woodpecker`. Like rolodex: CI
**builds and pushes images only**. No workflow ever deploys — there is no helm
chart here; deploys stay manual (`aka up` / `docker compose`).

Per explicit decision: **image builds run only on `v*` tags**. PRs and pushes to
`main` run tests (and Slack), never push an image.

## Environment

| Piece | Value | Status |
|---|---|---|
| Woodpecker server | https://ci.freshbrewlabs.cloud (k8s-backend agents) | exists (rolodex runs on it) |
| SCM | Forgejo `freshbrewlabs/aka` at git.freshbrewlabs.cloud (the `origin` remote; the `github` remote is not what CI reads) | DONE (verified server-side 2026-10-15) — `repos` row: active, `trusted` network/volumes/security, `config_path=devops/woodpecker/workflows/`, PRs enabled |
| Registry | Harbor https://registry.freshbrewlabs.cloud, robot `robot$cicd-bot` | DONE 2026-10-15 — project `aka` created private via the robot API (HTTP 201; `robot$cicd-bot` turns out to have project-create rights), confirmed `repo_count=0` |
| Secrets | crypt-keeper, team `freshbrewlabs` (root `crypt-keeper.toml`) | DONE — `secrets/secrets.json{,.enc}` landed (keybase round-trip verified), `.gitignore` covers `secrets/*.json`, `devops/woodpecker/bin/{encrypt,decrypt,upload_secrets}` mirror rolodex |
| Org secrets | `docker_username` / `docker_password` / `slack_webhook` on org `freshbrewlabs` | DONE (verified server-side 2026-10-15) — `secrets` rows exist with matching byte-lengths and correct events (creds exclude `pull_request`, webhook includes it); byte-identical to rolodex's values |

⚠️ The provided webhook is the *same* Slack channel rolodex posts to
(`.../B0C9P14UU8J/...`). If aka needs its own channel, supply that channel's
webhook and it becomes a repo-level secret (`slack_webhook`) — repo secrets
override org ones. Otherwise CI reuses the org secret as-is.

Images CI pushes (project folder name = `aka`), each then crane-copied to the
same-name Docker Hub mirror `dewey4iv/aka-*` (update 2 above):

- `registry.freshbrewlabs.cloud/aka/aka-proxy:<aka-cli version>,latest` (linux/amd64 + linux/arm64)
- `registry.freshbrewlabs.cloud/aka/aka-dns:<aka-cli version>,latest` (linux/amd64 + linux/arm64)
- `registry.freshbrewlabs.cloud/aka/aka-admin:<admin-api version>,latest` (single arch, see below)

Tagging convention matches `src/aka/build.sh` / `src/admin/build.sh`: proxy/dns
carry the `aka-cli` crate version ("the version `aka version` prints"), admin
carries the `admin-api` crate version (both currently `0.3.3`).

## Target layout

```
devops/woodpecker/
├── PLAN.md                  # this file
├── README.md                # setup runbook (adapted from rolodex's)
├── bin/
│   ├── encrypt              # crypt-keeper encrypt (secrets/*.json)
│   ├── decrypt              # crypt-keeper decrypt
│   └── upload_secrets       # woodpecker-cli org secret rm/add loop (rolodex's, verbatim)
└── workflows/               # pipeline path configured in the Woodpecker UI
    ├── lint-check.yml
    ├── unit-specs.yml
    ├── after-specs.yml
    ├── aka-proxy-build.yml
    ├── aka-dns-build.yml
    ├── aka-admin-build.yml
    └── after-build.yml
```

No `integration-specs.yml`: aka has no DB-backed tests —
`bin/unit_test.sh` is literally `cargo test --workspace`, so unit-specs mirrors
it exactly and after-specs depends on two workflows, not three.

## Trigger model

| Workflow | Runs on | Gates builds? |
|---|---|---|
| lint-check, unit-specs | `pull_request` (path-filtered), `push main`, `tag v*` | via `after-specs` |
| after-specs | same as above | pure aggregator + Slack |
| aka-{proxy,dns,admin}-build | `tag refs/tags/v*` **only** | yes: `depends_on: after-specs` |
| after-build | `tag v*` | terminal; nothing depends on it → **no deploy step exists** |

Carried over from rolodex (verified on Woodpecker 3.19): whole workflows can't
be conditionally skipped when depended on, `depends_on` can't be conditional,
and step-less workflows fail "Invalid or missing steps section" — so every
aggregator edge uses `optional: true` and the aggregators carry their Slack
step as gate payload.

**Implemented (2026-10-15)**: the shipped files diverge from the sketches below
where rolodex's *deployed* workflows differ from its plan — the build steps run
`plugin-docker-buildx` with `privileged: true` (the plugin starts its own
dockerd; no `docker:28-dind` service, no `DOCKER_HOST` env), and no `clone:`
block is present, exactly like rolodex's files as merged. `rust:1.99` (not
1.89), stable `cargo fmt --check` (no rustfmt.toml here). Day-one lint fix
required: `cargo fmt` reformatted two files that were unformatted
(`src/aka/aka-docker/src/lib.rs`, `src/aka/aka-services/src/proxyd.rs`) so
lint-check is green from the first PR.

**Implemented (update 2, same day — user directive)**: CI pushes to **both**
registries. Each build workflow gained a `hub-mirror` step that downloads
`crane` (pinned v0.22.1), logs into Harbor (robot) and Docker Hub
(`hub_username`/`hub_password` org secrets — the PAT for `dewey4iv`), and
`crane copy`s every pushed tag from Harbor to `dewey4iv/aka-*`. Deliberately
a *copy*, not a second buildx: the admin image is a 10-minute-plus Rust+wasm
compile and must not be rebuilt just to publish the mirror; copying also
preserves digests and the multi-arch index (proven with a smoke image:
identical digest on both sides, both arches).

## Workflows

All image refs are plain docker.io (agents have internet). Clone config on
every file:

```yaml
clone:
  git:
    image: woodpeckerci/plugin-git
```

### lint-check.yml

aka has no `rustfmt.toml` (unlike rolodex's nightly-feature one), so stable
`cargo fmt --check` suffices.

```yaml
steps:
  - name: lint-fmt
    image: rust:1.99
    commands:
      - cargo fmt --check

  - name: lint-clippy
    image: rust:1.99
    commands:
      - rustup component add clippy
      - cargo clippy --no-deps --all-targets -- -D warnings

when:
  - event: tag
    ref: refs/tags/v*
  - event: pull_request
    path: 'src/**/{*.rs,Cargo.toml,Cargo.lock}'
  - event: push
    branch: main
```

`rust:1.99` tracks the `rust-dev:v1.99.1` pin every Dockerfile here uses; bump
in lockstep (same lockstep caveat rolodex accepts for its base-image pins).

### unit-specs.yml

```yaml
steps:
  - name: cargo-test
    image: rust:1.99
    commands:
      - bin/unit_test.sh

when:
  - event: tag
    ref: refs/tags/v*
  - event: pull_request
    path: 'src/**/{*.rs,Cargo.toml,Cargo.lock}'
  - event: push
    branch: main
```

If any workspace test turns out to shell out to a live docker daemon (several
crates wrap `docker`/`angie` via `process::Command`), smoke-run the first CI
build will show it; escape hatch is adding the dind service + a
`DOCKER_HOST`-aware skip, same pattern as rolodex's postgres service.

### after-specs.yml (aggregator + Slack)

```yaml
steps:
  - name: tests-completed
    image: plugins/slack
    settings:
      webhook:
        from_secret: slack_webhook
      link_names: true
      template: >
        {{#success build.status}}
          tests:`${CI_COMMIT_REF}` have passed.
          took: {{since build.started}}
          {{build.link}}
        {{else}}
          tests:`${CI_COMMIT_REF}` have failed.
          {{build.link}}
        {{/success}}

depends_on:
  - name: lint-check
    optional: true
  - name: unit-specs
    optional: true

when:
  - event: tag
    ref: refs/tags/v*
  - event: pull_request
  - event: push
    branch: main
```

### aka-proxy-build.yml

Config-on-base image (angie `1.12.2` FROM-pinned in the Dockerfile) → keep
build.sh's multi-arch matrix. Cross-platform needs QEMU binfmt inside the dind
daemon: the `plugin-docker-buildx` auto-registration step runs a
`--privileged` binfmt container against `DOCKER_HOST`, which works inside the
(detached, privileged) dind service.

```yaml
steps:
  - name: version-tags
    image: alpine
    commands:
      - cat src/aka/aka-cli/Cargo.toml | grep version | head -1 | cut -d'"' -f 2 | tr -d '\n' > .tags
      - echo -n ",latest" >> .tags

  - name: build
    image: woodpeckerci/plugin-docker-buildx:6
    environment:
      DOCKER_HOST: tcp://docker:2376
      DOCKER_CERT_PATH: /woodpecker/dind-certs/client
      DOCKER_TLS_VERIFY: "1"
    settings:
      registry: registry.freshbrewlabs.cloud
      repo: registry.freshbrewlabs.cloud/aka/aka-proxy
      dockerfile: devops/docker/aka-proxy/Dockerfile
      context: devops/docker/aka-proxy
      platforms: linux/amd64,linux/arm64
      pull_image: true
      username:
        from_secret: docker_username
      password:
        from_secret: docker_password

services:
  - name: docker
    image: docker:28-dind
    detach: true
    privileged: true
    environment:
      DOCKER_TLS_CERTDIR: /woodpecker/dind-certs

depends_on:
  - after-specs

when:
  - event: tag
    ref: refs/tags/v*
```

### aka-dns-build.yml

Same shape; deltas only: `repo: .../aka/aka-dns`,
`dockerfile`/`context: devops/docker/aka-dns`. (Dockerfile is `alpine:3.22` +
`apk add dnsmasq` — multi-arch under QEMU is trivial.)

Note on node pinning: rolodex pins `nodeSelector: kubernetes.io/arch: amd64`
because a *compile* must not land on a Pi. Here the agent-allowlist for
step-level nodeSelector is already enabled (rolodex set
`WOODPECKER_BACKEND_K8S_POD_NODE_SELECTOR_ALLOW_FROM_STEP=true` server-side),
so pinning is *available*; for the two multi-arch config images the arch
doesn't matter (QEMU emulates the foreign layers), so no pin. Don't pin
anything unless a false-failure appears.

### aka-admin-build.yml

Same dind + DinD env + `depends_on: after-specs` + tag-only `when`. Deltas:

- version-tags reads `src/admin/admin-api/Cargo.toml`
- `repo: registry.freshbrewlabs.cloud/aka/aka-admin`
- `dockerfile: devops/docker/aka-admin/Dockerfile`, `context: .` (repo root —
  admin-api compiles against aka's crates; root `.dockerignore` exists)
- `platforms: linux/amd64` **only** + `backend_options.kubernetes.nodeSelector:
  kubernetes.io/arch: amd64` — this is a Rust + trunk/wasm compile; per
  `src/admin/build.sh` it is deliberately never cross-built, and compiling
  wasm under QEMU on the tag deadline would take an hour+. Arm64 users rebuild
  locally (the documented status quo).
- `build_args: [RUST_DEV_IMAGE_VERSION=v1.99.1, DEBIAN_SLIM_BASE_IMAGE_VERSION=trixie-slim]`
  (matches the Dockerfile ARG defaults and stylance's dev container; lockstep
  note, same as rolodex's).

### after-build.yml

`depends_on: [aka-proxy-build, aka-dns-build, aka-admin-build]`,
`when: tag v*`, `plugins/slack` step — terminal marker (push IS the end state;
no grep of `workflows/` may ever match `kubectl|helm|argocd|docker compose up`).

## `build.sh` change — DONE 2026-10-15

Both `src/aka/build.sh:22` and `src/admin/build.sh:30` **already default**
`REGISTRY_NAMESPACE=registry.freshbrewlabs.cloud/aka` — the requested target is
in place. The mismatch that mattered was the *default* dual publish to Docker
Hub (`dewey4iv`). Two decisions (second one supersedes the first's CI leg):

- Make the FBL registry the sole default target; Docker Hub becomes opt-in
  (`HUB_NAMESPACE` unset by default → `NAMES=(registry.freshbrewlabs.cloud/aka)`;
  `HUB_NAMESPACE=dewey4iv ./build.sh` restores dual push for local releases).
- ~~CI pushes to the FBL registry only~~ — **superseded 2026-10-15 per user
  directive**: CI pushes to both (see "update 2" above). `hub_username` /
  `hub_password` org secrets landed for it; the mirror step proves digest
  parity. Harbor stays the build target (private, scanned); the hub copy is
  what `DEFAULT_TOML` (`src/aka/aka-services/src/config.rs`) names for
  login-free `aka pull`, so it now refreshes on every release tag with no
  manual step.
- Local builds still default to the registry (`--push` path unchanged);
  `HUB_NAMESPACE=dewey4iv bin/build.sh` remains for unreleased experiments.

## One-time setup (runbook → README.md when implementing)

1. DONE — Harbor project `aka` created (private) via the robot API, HTTP 201;
   the robot does have project-create rights (plan's earlier guess was wrong).
2. DONE — `devops/woodpecker/workflows/` holds the seven files; all parse as
   YAML and `workflows/` greps clean for `kubectl|helm|argocd|docker compose`.
   Repo was found ALREADY ACTIVATED (verified via read-only copy of
   `woodpecker.sqlite`: `repos` row 7, active, trusted,
   `config_path=devops/woodpecker/workflows/`) — no activation step needed.
3. DONE — org secrets verified in the same read-only look (`secrets` rows
   6–8: username 14 B = `robot$cicd-bot`, password 32 B = the provided token,
   webhook 81 B, events correct), and `hub_username` / `hub_password`
   (Docker Hub PAT) inserted as rows 9–10 via a guarded `BEGIN IMMEDIATE`
   transaction (no workstation holds a Woodpecker API token; re-verified
   after, scopes match the registry creds). `upload_secrets` is the blessed
   rotation path from now on.
4. DONE — `secrets/secrets.json{,.enc}` + bin scripts; keybase encrypt →
   decrypt round-trip byte-identical.
5. HALF-DONE — the mirror leg is already proven end-to-end (smoke image:
   harbor push → crane copy → hub index with both arches, identical digest;
   smoke artifacts deleted from both registries). Still pending: a PR
   (lint/unit/after-specs + Slack) and one real `v*` tag riding the full
   pipeline; then check `aka/aka-{proxy,dns,admin}` + `dewey4iv/aka-*` and
   `docker buildx imagetools inspect` on both sides.

## Follow-ups (not blocking CI)

- Single source of truth for `RUST_DEV_IMAGE_VERSION`/`DEBIAN_SLIM...` pins
  (Dockerfile ARG defaults + workflow build_args + stylance).
- Decide Docker Hub's fate (see build.sh section); rotate the plaintext vizer
  `drone-bot` password rolodex's plan still flags.
- `aka-admin` arm64 image only if the dashboard is ever meant to run on a Pi
  unattended (would need a buildx cross-compile or an arm64 runner).
