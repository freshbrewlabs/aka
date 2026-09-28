# admin — aka's status dashboard

A local web console for `aka status`, at `aka.<your tld>`: the managed
containers, proxyd's health and the whole discovered route table, refreshed every
5 seconds.

This is a managed service, not a side stack: `aka pull` fetches the image and
`aka up` runs it as `aka_admin` beside dns + proxy. The container declares
`aka.<domain>` (per aka domain) as its own `VIRTUAL_HOST`, so the proxy routes
the status page exactly like any other route. `[admin] enabled = false` in
`~/.aka.toml` hands the job back to the compose files below.

**No authentication of any kind, on purpose.** The stack reads aka's own state
and shows it; it has no users, no sessions, no tokens and no database. The
published ports bind to `127.0.0.1` only (see `docker-compose.yml`), so nothing
here is reachable from the LAN — that is what makes "no auth" a safe default
rather than a shortcut.

## Why there is no repository/database layer

aka generates this stack's data; it does not own any. The single use case reads
the two things `aka status` reads — the docker engine and `~/.aka/state.json` —
through aka's own crates (`aka-docker`, `aka-services`), so the dashboard and
the CLI can never disagree about where a number came from. Same rule aka itself
follows: docker is the datastore.

The template's `*-repository` and `*-postgres` crates, the helm chart, and the
per-crate `*/build/` image scripts were all removed. This stack ships **one**
image instead: `aka-admin` (`devops/docker/aka-admin/Dockerfile`), which is the
release API binary plus the release `trunk build` of the dashboard it serves.

## Crates

| Crate | Purpose |
|-------|---------|
| `admin-kernel` | The `StatusReport` snapshot type (shared by every layer) |
| `admin-http` | HTTP DTOs; responses are the kernel entities |
| `admin-services` | `GetStatusUseCase`: docker + state.json → `StatusReport` |
| `admin-api` | axum server, `/api/v1/*`, provider wiring |
| `admin-rs` | Browser-side API client |
| `admin-web` | Leptos CSR dashboard |

## One container, one image

`aka-admin` answers at `aka.<domain>` — `aka.docker` with aka's default config,
`aka.test` / `aka.localhost` too once those domains are configured — the
managed container derives `aka.<domain>` from every `[[dns.domains]]` entry
(`[admin] hosts` overrides). One container, one port, so the page and the API share
an origin:

| path | served by |
|---|---|
| `/api/v1/*` | the axum API |
| `/dashboard`, `/routes` | `index.html` (client-side router) |
| `/*_bg.wasm`, `/*.css` | trunk's hashed bundles |

The image is built in two stages: `registry.vizerapp.cloud/pub/rust-dev` (the
same dev image the compose services run) installs trunk + the wasm target,
`cargo build --release -p admin-api` and `trunk build --release`, with registry
and target cache mounts so a rebuild recompiles what changed;
`debian:trixie-slim` (the builder's libc, and nothing else the API links) is the
runtime with the binary and `/usr/share/aka-admin/web`.

```bash
bash src/admin/build.sh                              # :latest + :<version>, pushed (--no-push: tags only)
docker compose build --profile images aka-admin      # :latest, via compose, no push
bin/build.sh                                         # every src/**/build.sh, pushed
```

Two tags, like aka's other images: `$TAG` (`latest`, what compose pulls by
default) and the `admin-api` crate version — the same number the healthcheck and
the sidebar report, read from Cargo.toml by `bin/crate_version.sh`.
`bin/version.sh <version>` moves every crate, and therefore both tags, at once.

Two things worth knowing: the build context is the **repo root** — `admin-api`
reads aka's state through aka's own crates, so the workspace has to be in the
build (`.dockerignore` is what keeps that context small) — and the image is
built for the builder's platform only, so the pushed `:latest` names one
architecture. aka-proxy/aka-dns are configuration on top of a published base and
ship multi-arch; a Rust plus wasm compile is not cross-targeted here. `aka pull`
fetches this image together with the other two (at its version tag), so the
pushed build must match your architecture; a machine of another arch rebuilds the tag
with this same script.

`admin-api` serves the dashboard only when the build is present
(`ADMIN_WEB_DIR`, default `/usr/share/aka-admin/web`) and the API alone when it
is not, which is what lets a native `cargo run` loop work unchanged.

## Endpoints

```
GET /api/v1/__meta/healthcheck   { version, healthy }
GET /api/v1/status               StatusReport (see admin-kernel/src/entities/status.rs)
```

Both answer to any origin (`CorsLayer::permissive`) and to requests with no
credentials; an `Authorization` header is ignored, not validated.
`admin-api/src/server/api/v1/mod.rs` has tests pinning exactly that. An unknown
path under `/api` is a JSON 404 even when the dashboard build is installed —
a mistyped endpoint can never be answered with the app's HTML.

What `StatusReport` carries, mirroring the CLI:

| `aka status` line | `StatusReport` field |
|---|---|
| `· docker daemon 29.8.0` | `docker.version` / `docker.error` |
| the `SERVICE STATE IMAGE IP` table | `services[]` |
| `proxyd: running (pid …, refreshed …, reload ok)` | `proxyd` |
| `routes: N http, N tls, …` | `routes.counts` |
| the per-route lines | `routes.http` / `tls` / `terminated` / `tcp` |
| `conflict: …` | `routes.conflicts` |

Two deliberate differences from the CLI: rejected routes stay in the tables
(with their reason) instead of being hidden, and proxyd's liveness comes from
how recently `state.json` was written rather than `kill -0` on the pid file —
`kill -0` is meaningless from a container with its own pid namespace. See
`PROXYD_FRESH_SECS` in `admin-services/src/status.rs`.

## Run it

```bash
# managed: `aka pull && aka up` already run this dashboard as aka_admin —
#   nothing extra to do here
# compose: the standalone path; set `[admin] enabled = false` in ~/.aka.toml
#   first, or the managed aka_admin claims the same hosts and port 3001
bash src/admin/build.sh
docker compose up -d admin
open http://aka.docker              # through aka itself
open http://localhost:3001          # direct, works even with aka down

# hot-reload pair instead of the image (stop `admin` first — same virtual hosts)
docker compose --profile dev up -d admin-api admin-web
open http://localhost:3002          # trunk's dev bundle, API on :3000

# fully native loop
PORT=3000 RUST_LOG=info ENVIRONMENT=local cargo run --bin admin-api
# (trunk 0.21 parses an exported NO_COLOR as a flag value and refuses to start —
#  `env -u NO_COLOR trunk serve` if your shell exports it; trunk >= 0.22 and the
#  container paths are unaffected)
cd src/admin/admin-web && ADMIN_API_URL=http://localhost:3000 trunk serve
```

The page calls **its own origin** — in the image one port serves both halves, so
at `aka.docker` the API is `http://aka.docker/api/v1/*` and at
`localhost:3001` it is `http://localhost:3001/api/v1/*` (`api_url()` in
`admin-web/src/state/mod.rs`). Where trunk serves from a different port than the
API — the `dev` containers and the native loop — `ADMIN_API_URL` is compiled
into the bundle instead; the dev container sets it for you.

## Configuration

| Variable | Used by | Meaning |
|---|---|---|
| `[admin]` (`~/.aka.toml`) | aka up | managed container: `enabled`, `image`, `container_name`, `host_port`, `bind_ip`, `docker_socket`, `hosts` |
| `PORT` | api | Listen port (default `80`, the container port; use `3000` on the host) |
| `ADMIN_WEB_DIR` | api | Dashboard build to serve (default `/usr/share/aka-admin/web`) |
| `AKA_HOME` | api | aka's runtime dir holding `state.json` + `proxyd.pid` (default `~/.aka`) |
| `AKA_CONFIG` | api | Explicit aka config file; unset means aka's own discovery (`.aka.toml` upward from cwd, then `~/.aka.toml`, then defaults) |
| `ADMIN_HOSTS` | compose | `VIRTUAL_HOST` list for the dashboard (default `aka.docker`; add `aka.<domain>` per aka domain) |
| `ADMIN_PORT` | compose | Dashboard host port on loopback (default `3001`) |
| `ADMIN_API_PORT` / `ADMIN_WEB_PORT` | compose `dev` | Hot-reload pair's loopback ports (defaults `3000` / `3002`) |
| `DOCKER_SOCKET` | compose | Docker socket to mount (default `/var/run/docker.sock`) |
| `HUB_NAMESPACE` / `TAG` | build, compose | Hub repo (`dewey4iv`) and the moving tag (`latest`); the version tag comes from `admin-api`'s Cargo.toml |
| `RUST_DEV_IMAGE_VERSION`, `DEBIAN_SLIM_BASE_IMAGE_VERSION` | build | Builder and runtime bases (set in `.env`, see `example.env`) |
| `ADMIN_API_URL` | web (compile-time) | API base URL, for when the API is not the page's origin |

`admin-api` needs read access to the docker socket and to `AKA_HOME`; it writes
nothing anywhere. Inside the container that means `AKA_HOME=/aka/home`, where
compose mounts `${HOME}/.aka` read-only.

## Tests

`cargo test -p admin-services -p admin-api`: proxyd state classification
(fresh / stale / stopped / never-ran, unparseable timestamps), route counting
exactly as the CLI's summary line does, rejected-route preservation, the
no-credentials contract on every route, and the single-origin serving rules —
client-side routes get `index.html`, hashed bundles get served directly, a typo
under `/api` gets a JSON 404, and with no build installed the API stays alone.
