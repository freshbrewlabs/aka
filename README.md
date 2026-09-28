# aka

Your development proxy for docker — a Rust spiritual successor to
[dory](https://github.com/FreedomBen/dory). Containers become reachable by
hostname: set `VIRTUAL_HOST` on a container, run `aka up`, open
`http://that-host.docker`. Unlike dory, aka routes **raw TCP and TLS** too,
using [angie](https://angie.software) (an nginx fork with a first-class
`stream` module) as the proxy image.

## Install

```bash
bin/install.sh
```

Builds the two managed images, `cargo install`s the `aka` binary into
`~/.cargo/bin`, and writes a commented `~/.aka.toml` if you don't have one
(never overwrites). Nothing needs sudo; `aka up` prompts for it when it
writes `/etc/resolver` files. Re-run any time — image builds are cached.

## Quick start

```bash
docker compose -f src/aka/docker-compose.yml up -d demo-web demo-cache

sudo aka up                           # first run: sudo to write /etc/resolver
curl http://demo.docker               # -> nginx welcome page
redis-cli -h cache.docker             # -> PONG  (raw tcp route)
aka status
aka down
```

Without the compose demo, any running container that sets `VIRTUAL_HOST`
is picked up automatically — no restart of aka required.

## Route declarations

Environment variables (labels of the same name also work; labels win).
Names are compatible with dory / nginx-proxy, so existing containers just work.

| Key | Default | Meaning |
|-----|---------|---------|
| `VIRTUAL_HOST` | — | comma-separated hostnames; presence enables routing |
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

## Commands

```
aka up                start dns + proxy + host resolver + proxyd (the daemon)
aka down              stop everything, remove resolver entries
aka restart           down + up
aka status            container states, daemon health, route table
aka routes [--json]   discovered routes (also in ~/.aka/state.json)
aka logs [dns|proxy]  follow container logs
aka attach [service]  docker attach to a service container
aka ip [service]      print a service container IP
aka pull              pull the managed images
aka config-file       write the default config (--force, --upgrade)
aka version
```

Global flags: `-v/--verbose`, `-c/--config <path>`.

## Configuration

`~/.aka.toml` (written by `aka config-file`), or `./.aka.toml` searched
upward from the cwd; `-c` wins. TOML, with dory's config mapped onto
top-level sections: `[dns]` ← dory `dnsmasq`, `[proxy]` ← `nginx_proxy`,
`[resolv]` ← `resolv`.

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
  `*.docker` to 127.0.0.1 without touching system DNS; writing them needs
  sudo (`aka up` shells out; `aka down` removes them). `resolv.enabled:
  false` skips this for testing with `dig @127.0.0.1` / `curl --resolve`.
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
bin/build.sh       # docker images (devops/docker/aka-proxy, aka-dns)
bin/unit_test.sh   # cargo test --workspace
cargo run -p aka-cli -- up -c ./dev.yml   # dev loop; AKA_HOME=./.aka-dev
```

Unit tests cover discovery, renderers (golden fragments + map-line shape),
TOML config merge/upgrade with multi-domain lists, state round-trip, dnsmasq
args, and the port-conflict classifier. Live verification (images + daemon + http/tcp/tls termination
through a real Docker Desktop) is documented in the commit history.
