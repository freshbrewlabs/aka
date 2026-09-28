# aka

Your development proxy for docker — a Rust spiritual successor to
[dory](https://github.com/FreedomBen/dory). Containers become reachable by
hostname: set `VIRTUAL_HOST` on a container, run `aka up`, open
`http://that-host.docker`. Unlike dory, aka routes **raw TCP and TLS** too,
using [angie](https://angie.software) (an nginx fork with a first-class
`stream` module) as the proxy image.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/freshbrewlabs/aka/main/install.sh | bash
```

Needs git, a running Docker, and Rust/cargo — the script checks all three
and tells you what's missing. It clones the repo to a temp dir, `cargo
install`s the `aka` binary into `~/.cargo/bin`, and writes a commented
`~/.aka.toml` if you don't have one (never overwrites). Nothing needs sudo,
and **no docker images are built**: they're published, so `aka pull` fetches
`aka-proxy` / `aka-dns` / `aka-admin` from the hub (run it once, before your
first `aka up`). It pulls the build tagged with your installed aka version and
points the configured tag at it, so re-run it after `aka update`; the first run
compiles aka from source (a few minutes). Pin a branch or tag with
`... | AKA_REF=v1.2.3 bash`.
Afterwards `aka update` re-runs the same flow in place — clones the repo
(default `main`; `--ref <branch|tag>` or `AKA_REF` pins, `AKA_REPO` points at
a fork), cargo-reinstalls `~/.cargo/bin/aka`, and reports the version change.

Already have a clone? `bin/install.sh` does the same steps in place.

## Quick start

```bash
aka install-sudo-rule                 # one-time: see "No sudo" below
docker compose -f src/aka/docker-compose.yml up -d demo-web demo-cache

aka up                                # no password: the rule covers it
open http://aka.docker                  # -> the aka status dashboard
curl http://demo.docker               # -> nginx welcome page
redis-cli -h cache.docker             # -> PONG  (raw tcp route)
aka status
aka down
```

Without the compose demo, any running container that sets `VIRTUAL_HOST`
is picked up automatically — no restart of aka required.

## No sudo

`aka up` / `aka down` edit `/etc/resolver/*` (macOS) or `/etc/resolv.conf`
(Linux) by shelling out `sudo -n aka _privileged ...` — a hidden root-side
verb that only accepts validated domains and IP literals. Run this once:

```bash
aka install-sudo-rule    # the only moment sudo ever asks for a password
```

It writes `you ALL=(root) NOPASSWD: <aka> _privileged *` to
`/etc/sudoers.d/com.freshbrewlabs.aka` and validates it with `visudo -cf`
before accepting. Remove it with
`sudo rm /etc/sudoers.d/com.freshbrewlabs.aka`; without the rule aka just
prompts for your password (and prints this command as the hint). Caveat:
the rule points at the user-owned `aka` binary path, so whoever can replace
that binary can perform aka's root writes — the normal trust level for a
dev-machine user; delete the rule to revoke. The port-conflict stop/kill
flow (`kill_others`) still uses an interactive `sudo kill`.

## Route declarations

Environment variables (labels of the same name also work; labels win).
Names are compatible with dory / nginx-proxy, so existing containers just work.

| Key | Default | Meaning |
|-----|---------|---------|
| `VIRTUAL_HOST` | — | hostnames separated by commas and/or spaces; presence enables routing |
| `VIRTUAL_PORT` | first exposed port, else `80` | backend port |
| `VIRTUAL_PROTO` | `http` | `http`, `https`, `tls`, or `tcp` |

- **http** — vhost on `:80` (Host header routing), websocket upgrades on.
- **https** — vhost proxies to an HTTPS backend (`proxy_ssl_*`).
- **tls** — TCP `:443` SNI passthrough (no certificate in aka; the backend
  terminates TLS). Containers sharing a host load-balance together.
- **tcp** — raw TCP. The proxy publishes the route port on the host
  (`aka_proxy` is recreated when the set of tcp ports changes);
  `cache.docker` answers `127.0.0.1` and `redis-cli -h cache.docker:6379`
  style clients reach the backend via angie's `stream`.
- **terminated https** — drop `<host>.crt` + `<host>.key` into
  `proxy.ssl_certs_dir` and any http route for that host also gets a TLS
  vhost on `:8443` (SNI on the shared `:443` listener hops to it).

Proxied connections get a 1 h idle window in both directions
(`proxy_read_timeout` / `proxy_send_timeout`, rendered once in the http
context so every vhost inherits it), which is what keeps sockets that go quiet
between events alive — websocket hot-reload (trunk), SSE, long polls, a build
streaming its logs. angie's 60 s default drops those sockets, and clients that
read a drop as "the server restarted" reload the page.

## Commands

```
aka up                start dns + proxy + admin dashboard + host resolver + proxyd (the daemon)
aka down              stop everything, remove resolver entries
aka restart           down + up
aka status            container states, daemon health, route table (also http://aka.docker)
aka routes [--json]   discovered routes (also in ~/.aka/state.json)
aka logs [dns|proxy|admin]  follow container logs
aka attach [service]  docker attach to a service container
aka ip [service]      print a service container IP
aka pull              pull the managed images (proxy, dns, admin) at this aka version
aka config-file       write the default config (--force, --upgrade)
aka update [--ref r]  rebuild + reinstall aka from the repo (default: main)
aka version
```

Global flags: `-v/--verbose`, `-c/--config <path>`.

## Configuration

`~/.aka.toml` (written by `aka config-file`), or `./.aka.toml` searched
upward from the cwd; `-c` wins. TOML, with dory's config mapped onto
top-level sections `[dns]` ← dory `dnsmasq`, `[proxy]` ← `nginx_proxy`,
`[resolv]` ← `resolv`, plus `[admin]` for the managed status dashboard.

Multiple TLDs are first-class — each `[[dns.domains]]` entry (dory's
`domains` array, once again) serves that domain **and all subdomains**:

```toml
[[dns.domains]]
domain = "docker"
address = "127.0.0.1"

[[dns.domains]]
domain = "test"
address = "127.0.0.1"

[[dns.domains]]
domain = "localhost"
address = "127.0.0.1"   # explicit entry => dnsmasq answers *.localhost too

[resolv]
enabled = true          # writes /etc/resolver/docker, /etc/resolver/test, ...

[admin]
enabled = true          # aka pull fetches the image, aka up runs the container
# hosts defaults to aka.<domain> for every [[dns.domains]] entry above, and
# the proxy routes those hosts to it automatically; direct access (aka down
# included) is http://localhost:3001
```

Verified: `dig +short a.docker` / `sub.x.test` / `y.localhost` all answer
the configured address; browsers need no resolver files for `*.localhost`
at all (they resolve it to loopback natively).

`dory → aka` key map:

| dory (`~/.dory.yml`) | aka (`~/.aka.toml`) |
|---|---|
| `dnsmasq.domains` (`tld`) | `[[dns.domains]]` |
| `dnsmasq.port` | `dns.port` |
| `dnsmasq.container_name` | `dns.container_name` |
| `kill_others` | `dns.kill_others` |
| `nginx_proxy.port` / `.tls_port` | `proxy.http_port` / `proxy.tls_port` |
| `nginx_proxy.https_enabled` | `proxy.tls_enabled` |
| `nginx_proxy.ssl_certs_dir` | `proxy.ssl_certs_dir` |
| `resolv.nameserver` / `.port` | `resolv.nameserver` / `resolv.port` |
| — (docker-machine IP) | `dns.bind_ip` (loopback on Docker Desktop) |
| — (no dory counterpart) | `[admin]` (managed status dashboard) |

Runtime home is `~/.aka` (`AKA_HOME` overrides; used by tests): rendered
angie configs (`proxy.d/http`, `proxy.d/stream`), `state.json`,
`proxyd.pid`, `proxyd.log`. `-c/--config` always overrides file discovery.

## Architecture

```
+--------------------------------------------------------------+
| aka-cli (aka)  — dory-parity commands; spawns the daemon     |
+-----------------------------+--------------------------------+
                              | docker engine API (bollard)
      +-----------------------v------------------------+
      | aka_proxy (angie)  | aka_dns (dnsmasq)         |
      |  :80  http vhosts  |  :53  *.docker -> 127.0.0.1|
      |  :443 sni stream   |                            |
      |  8443 terminated   |                            |
      |  <tcp ports>       |                            |
      +--------------------+----------------------------+
      ^ writes rendered configs        ^ resolves hostnames
      |                                |
+-----+--------------------------------+-----------------------+
| aka _proxyd (hidden) — watches docker events + 20s resync:   |
| discover VIRTUAL_* -> render angie http/stream -> `angie -t` |
| + reload; recreate proxy when the published tcp set drifts;  |
| attach proxy to every route container's network; publish     |
| ~/.aka/state.json for `aka status` / `aka routes`            |
+--------------------------------------------------------------+
```

Crates: `aka-kernel` (entities/config/state), `aka-docker` (bollard wrapper),
`aka-services` (config, discovery, renderers, resolv, lifecycle, daemon),
`aka-cli` (the binary). No database/repository layers — this is a dev tool,
docker is the datastore.

`aka status` also has a web face: `src/admin/` ships the `aka-admin` image —
the axum API plus the dashboard it serves, reading the same docker engine and
`~/.aka/state.json`, no authentication at all (loopback-only by design). It is
a managed service like dns and proxy: `aka pull` fetches it, `aka up` runs it
as `aka_admin`, and the container declares `aka.<domain>` for every aka domain
as its own `VIRTUAL_HOST`, so the proxy routes the status page exactly like
any other route. `[admin] enabled = false` hands the job back to compose (or
nothing). See `src/admin/README.md`.

## Design notes / macOS

- **Answer IP is `127.0.0.1`**: Docker Desktop publishes the proxy's ports
  on host loopback, so every managed domain resolving to loopback just
  works — and the dns records never change when containers are recreated.
- **Port 53**: Docker Desktop publishes privileged ports through a
  root helper, so aka's dnsmasq container can bind `:53` without any host
  root daemon. Modern macOS *denies* unprivileged binds on privileged
  ports — a bare bind probe is therefore not proof of a free port; aka
  uses `lsof` truth plus the docker publish list instead.
- **`/etc/resolver/<domain>`** files (dory-style) make macOS send
  `*.docker` to 127.0.0.1 without touching system DNS; aka writes/removes
  them via the passwordless `_privileged` verb (see "No sudo").
  `resolv.enabled: false` skips this for testing with `dig @127.0.0.1` /
  `curl --resolve`.
- **`.localhost` hosts**: browsers resolve `*.localhost` to loopback on
  their own. For CLI tools (`curl`, `dig`) dnsmasq needs the explicit
  `[[dns.domains]] domain = "localhost"` entry — once present it answers
  authoritatively (verified).
- **tcp route changes recreate `aka_proxy`** (docker fixes published ports
  at create time); a recreate takes ~1s and briefly drops in-flight
  connections on the proxy.
- **Host conflicts**: `aka up` maps occupied ports back to the containers
  publishing them (or lsof-detected processes) and offers to stop/kill
  them — the path dory's `kill_others` took, docker-aware.

## Development

```bash
bin/build.sh       # every src/**/build.sh, pushed to $HUB_NAMESPACE as :$TAG
                   # (`latest`) and :<crate version> — aka-proxy + aka-dns
                   # multi-arch, aka-admin (dashboard) for one arch
bin/version.sh 0.2.0   # every crate to one version, so the version tag moves
                        # with it (no arguments: show; cargo-edit underneath)
bin/unit_test.sh   # cargo test --workspace
cargo run -p aka-cli -- up -c ./dev.yml   # dev loop; AKA_HOME=./.aka-dev
docker compose up -d public-web   # landing page -> http://www.parkinglot.localhost / :8080 direct
                                   # (site lives in src/public/; prod = static host on that dir)
bash src/admin/build.sh                   # dashboard image: tag + push (--no-push: tag)
docker compose up -d admin      # the compose way to run the dashboard; the
                                # managed `aka_admin` (aka up) already claims
                                # the aka.* hosts and 127.0.0.1:3001 — use
                                # `[admin] enabled = false` when you prefer compose
docker compose --profile dev up -d admin-api admin-web  # hot-reload instead of the image
                                                        # -> http://localhost:3002, API on :3000
```

The admin containers publish on `127.0.0.1` only, and the compose paths need
the managed `aka_admin` stopped — one thing must own the `aka.*` virtual hosts.

Images always carry two tags: `$TAG` (`latest`, what the compose files default
to) and the workspace version — read from Cargo.toml through
`bin/crate_version.sh`, the same number `aka version` prints and the dashboard's
healthcheck reports; `aka pull` tries that version tag and moves the configured
image onto it. `bin/version.sh` prints those versions; `bin/version.sh
<version>` moves every crate at once (needs cargo-edit: `cargo install
--no-default-features --features set-version cargo-edit`); the next
`bin/build.sh` publishes both tags.

Unit tests cover discovery, renderers (golden fragments + map-line shape),
TOML config merge/upgrade with multi-domain lists, state round-trip, dnsmasq
args, the port-conflict classifier, and `_privileged` request validation
(domain injection, foreign-file safety, sudoers scoping). Live verification
(images + daemon + http/tcp/tls termination through a real Docker Desktop)
is documented in the commit history.
