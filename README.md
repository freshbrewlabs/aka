# aka

Your development proxy for docker — a Rust spiritual successor to
[dory](https://github.com/FreedomBen/dory). Containers become reachable by
hostname: set `VIRTUAL_HOST` on a container, run `aka up`, open
`http://that-host.docker`. Unlike dory, aka routes **raw TCP and TLS** too,
using [angie](https://angie.software) (an nginx fork with a first-class
`stream` module) as the proxy image.

## Quick start

```bash
bin/build.sh                          # builds aka-proxy:local + aka-dns:local
cargo build                           # the aka binary
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

`~/.aka/aka.yml` (written by `aka config-file`), or `./.aka.yml` searched
upward from the cwd; `-c` wins. Everything lives under a top-level `aka:`
key. Highlights:

```yaml
aka:
  dns:
    image: aka-dns:local
    port: 53                # published on dns.bind_ip
    bind_ip: 127.0.0.1
    domains:                # wildcards resolved to the address below
      - domain: docker
        address: 127.0.0.1  # loopback: docker publishes the proxy here
    kill_others: ask        # true | false | ask | comma-list of ports
  resolv:
    enabled: true           # write /etc/resolver/<domain> (sudo)
  proxy:
    image: aka-proxy:local
    http_port: 80
    tls_port: 443
    tls_enabled: true
    ssl_certs_dir: ""       # enables :8443 terminated https vhosts
    network: aka
```

Runtime home is `~/.aka` (`AKA_HOME` overrides; used by tests): rendered
angie configs (`proxy.d/http`, `proxy.d/stream`), `state.json`,
`proxyd.pid`, `proxyd.log`.

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
- **`.localhost` hosts** work through browsers (which resolve `*.localhost`
  to loopback themselves) but dnsmasq answers queries for them from its
  built-in zone, not aka's — prefer the `docker` TLD.
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
config merge/upgrade, state round-trip, dnsmasq args, and the port-conflict
classifier. Live verification (images + daemon + http/tcp/tls termination
through a real Docker Desktop) is documented in the commit history.
