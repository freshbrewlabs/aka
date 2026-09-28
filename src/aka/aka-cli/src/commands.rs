use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use aka_docker::AkaDocker;
use aka_kernel::BoxedError;
use aka_kernel::config::AkaConfig;
use aka_services::paths::AkaPaths;
use futures::{Stream, StreamExt};
use provider::Provider;
use tokio::signal::unix::{SignalKind, signal};
use tracing::info;

/// The CLI's resolved config selection (kept for `config-file` target path).
pub struct ConfigSelection {
    pub explicit: Option<PathBuf>,
}

pub fn resolve_config(explicit: &Option<PathBuf>, _daemon: bool) -> Result<AkaConfig, BoxedError> {
    let start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    aka_services::load_or_default(&start, explicit.as_deref())
}

fn ctx(provider: &Provider) -> (&AkaConfig, &AkaPaths, &AkaDocker) {
    (
        provider.fetch_unchecked::<AkaConfig>(),
        provider.fetch_unchecked::<AkaPaths>(),
        provider.fetch_unchecked::<AkaDocker>(),
    )
}

fn service_name(cfg: &AkaConfig, service: Option<&str>) -> Result<String, BoxedError> {
    match service.unwrap_or("proxy") {
        "proxy" => Ok(cfg.proxy.container_name.clone()),
        "dns" => Ok(cfg.dns.container_name.clone()),
        other if service_maybe_container(other) => Ok(other.to_owned()),
        other => Err(format!(
            "unknown service {other:?} (expected dns, proxy or a container name)"
        )
        .into()),
    }
}

fn service_maybe_container(name: &str) -> bool {
    !name.is_empty() && !name.contains(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
}

// ---------------------------------------------------------------------- up

pub async fn up(provider: &Provider) -> Result<(), BoxedError> {
    let (cfg, paths, docker) = ctx(provider);

    docker
        .ping()
        .await
        .map_err(|_| "docker daemon is not reachable; start docker and try again")?;

    if cfg.dns.enabled && !docker.running(&cfg.dns.container_name).await? {
        ensure_port_free(
            docker,
            &cfg.dns.kill_others,
            cfg.dns.port,
            true,
            &cfg.dns.container_name,
        )
        .await?;
    }

    if cfg.proxy.enabled && !docker.running(&cfg.proxy.container_name).await? {
        ensure_port_free(
            docker,
            &cfg.dns.kill_others,
            cfg.proxy.http_port,
            false,
            &cfg.proxy.container_name,
        )
        .await?;
        if cfg.proxy.tls_enabled {
            ensure_port_free(
                docker,
                &cfg.dns.kill_others,
                cfg.proxy.tls_port,
                false,
                &cfg.proxy.container_name,
            )
            .await?;
        }
    }

    let outcomes = aka_services::lifecycle::up(docker, cfg, paths).await?;
    for (name, outcome) in &outcomes {
        println!("{name}: {outcome:?}");
    }

    if cfg.resolv.enabled {
        let files = aka_services::resolv::configure(cfg)?;
        for file in files {
            println!("resolver: {file:?}");
        }
    }

    match ensure_daemon(paths, provider) {
        DaemonSpawn::Spawned { pid, log } => println!("proxyd: started (pid {pid}, log {log:?})"),
        DaemonSpawn::AlreadyRunning { pid } => println!("proxyd: already running (pid {pid})"),
    }

    if outcomes.is_empty() {
        println!("nothing enabled in the aka config");
    }
    Ok(())
}

enum DaemonSpawn {
    Spawned { pid: u32, log: PathBuf },
    AlreadyRunning { pid: u32 },
}

fn daemon_pid(pid_file: &std::path::Path) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(pid_file)
        .ok()?
        .trim()
        .parse()
        .ok()?;
    alive(pid).then_some(pid)
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn ensure_daemon(paths: &AkaPaths, provider: &Provider) -> DaemonSpawn {
    let pid_file = paths.pid_file();
    if let Some(pid) = daemon_pid(&pid_file) {
        return DaemonSpawn::AlreadyRunning { pid };
    }

    let log_path = paths.log_file();
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .expect("open proxyd log");
    let log_err = log.try_clone().expect("clone log");

    let selection = provider.fetch_unchecked::<ConfigSelection>();
    let mut command = Command::new(std::env::current_exe().expect("current exe"));
    command
        .arg("_proxyd")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .env(
            "RUST_LOG",
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        )
        .env("ENVIRONMENT", "local");
    if let Some(explicit) = &selection.explicit {
        command.arg("-c").arg(explicit);
    }
    // run in a fresh process group so a terminal ctrl-c cannot kill it
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let child = command.spawn().expect("spawn aka _proxyd");
    let pid = child.id();
    let _ = std::fs::write(&pid_file, format!("{pid}\n"));
    info!("spawned proxyd pid {pid}");
    // deliberately detached: re-parented to init, stopped via `aka down`
    std::mem::forget(child);
    DaemonSpawn::Spawned { pid, log: log_path }
}

fn stop_daemon(paths: &AkaPaths) {
    let pid_file = paths.pid_file();
    if let Some(pid) = daemon_pid(&pid_file) {
        println!("proxyd: stopping (pid {pid})");
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status();
        for _ in 0..20 {
            if !alive(pid) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    let _ = std::fs::remove_file(&pid_file);
}

// -------------------------------------------------------------------- down

pub async fn down(provider: &Provider) -> Result<(), BoxedError> {
    let (cfg, paths, docker) = ctx(provider);

    stop_daemon(paths);
    aka_services::lifecycle::down(docker, cfg).await?;
    println!("{}: stopped", cfg.dns.container_name);
    println!("{}: stopped", cfg.proxy.container_name);

    if cfg.resolv.enabled {
        // the services are already stopped; failing resolver cleanup must
        // not make `down` itself fail (sudo-free flow: install-sudo-rule)
        match aka_services::resolv::clean(cfg) {
            Ok(()) => println!("resolver entries removed"),
            Err(err) => println!("warn: resolver entries not removed: {err}"),
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ status

pub async fn status(provider: &Provider) -> Result<(), BoxedError> {
    let (cfg, paths, docker) = ctx(provider);

    let version = docker.daemon_version().await.unwrap_or_else(|_| "?".into());
    println!(
        "aka {}\n · docker daemon {version}",
        env!("CARGO_PKG_VERSION")
    );

    println!("\n{:<14} {:<10} {:<26} IP", "SERVICE", "STATE", "IMAGE");
    for s in aka_services::lifecycle::status(docker, cfg).await? {
        println!(
            "{:<14} {:<10} {:<26} {}",
            s.name,
            s.state,
            s.image,
            s.ip.unwrap_or_else(|| "-".into())
        );
    }

    match aka_services::statefile::read(&paths.state_file())? {
        Some(state) => {
            let daemon = if daemon_pid(&paths.pid_file()) == Some(state.daemon_pid) {
                "running"
            } else {
                "not running"
            };
            println!(
                "\nproxyd: {daemon} (pid {}, refreshed {}, reload {})",
                state.daemon_pid,
                state.updated_at,
                if state.proxy.last_reload_ok {
                    "ok"
                } else {
                    "pending"
                }
            );
            print_routes(&state);
        }
        None => println!("\nproxyd: never ran (run `aka up`)"),
    }
    Ok(())
}

fn print_routes(state: &aka_kernel::state::DaemonState) {
    let table = &state.table;
    println!(
        "\nroutes: {} http, {} tls, {} terminated, {} tcp, {} conflicts",
        table.http.iter().filter(|r| r.rejected.is_none()).count(),
        table.tls.iter().filter(|r| r.rejected.is_none()).count(),
        table.terminated.len(),
        table.tcp.iter().filter(|r| r.rejected.is_none()).count(),
        table.conflicts.len()
    );

    for route in table.http.iter().filter(|r| r.rejected.is_none()) {
        println!(
            "  {} {}://{} -> {} [{}]",
            route.hosts.join(", "),
            route.proto.as_str(),
            route.port,
            route.upstream.servers.join(", "),
            route.containers.join(",")
        );
    }
    for route in table.tls.iter().filter(|r| r.rejected.is_none()) {
        println!(
            "  tls {}:{} -> {} [{}]",
            route.hosts.join(", "),
            route.port,
            route.upstream.servers.join(", "),
            route.containers.join(",")
        );
    }
    for route in &table.terminated {
        println!(
            "  terminated {} -> {} [{}]",
            route.host,
            route.upstream.servers.join(", "),
            route.container
        );
    }
    for route in table.tcp.iter().filter(|r| r.rejected.is_none()) {
        println!(
            "  tcp {}:{} -> {} [{}] published: :{}",
            route.hosts.join(", "),
            route.port,
            route.upstream.servers.join(", "),
            route.containers.join(","),
            route.port
        );
    }
    for conflict in &table.conflicts {
        println!("  conflict: {conflict}");
    }
}

// -------------------------------------------------------------------- logs

pub async fn logs(provider: &Provider, service: Option<&str>) -> Result<(), BoxedError> {
    let (cfg, _, docker) = ctx(provider);
    let name = service_name(cfg, service)?;

    let names: Vec<String> = match service {
        Some(_) => vec![name.clone()],
        None => vec![
            cfg.dns.container_name.clone(),
            cfg.proxy.container_name.clone(),
        ],
    };

    let mut streams: Vec<PinnedLogStream> = names
        .into_iter()
        .map(|n| {
            Box::pin(
                docker
                    .logs(&n, true)
                    .fuse()
                    .map(move |line| (n.clone(), line)),
            ) as PinnedLogStream
        })
        .collect();

    while let Some((prefix, line)) = next_line(&mut streams).await {
        let mut out = std::io::stdout();
        match line.stream {
            aka_docker::LogStream::Stdout => {
                let _ = write!(out, "[{prefix}] {}", line.message);
            }
            aka_docker::LogStream::Stderr => {
                let _ = write!(out, "[{prefix}!] {}", line.message);
            }
        }
        let _ = out.flush();
    }
    Ok(())
}

type PinnedLogStream = std::pin::Pin<Box<dyn Stream<Item = (String, aka_docker::LogLine)> + Send>>;

async fn next_line(streams: &mut [PinnedLogStream]) -> Option<(String, aka_docker::LogLine)> {
    use futures::future::poll_fn;
    use std::task::Poll;

    if streams.is_empty() {
        return None;
    }
    poll_fn(|cx| {
        for stream in streams.iter_mut() {
            if let Poll::Ready(Some(item)) = stream.as_mut().poll_next(cx) {
                return Poll::Ready(Some(item));
            }
        }
        Poll::Pending
    })
    .await
}

// ------------------------------------------------------------------ attach

pub fn attach(provider: &Provider, service: Option<&str>) -> Result<(), BoxedError> {
    let (cfg, _, _) = ctx(provider);
    let name = service_name(cfg, service)?;
    println!("attaching to {name} (detach: ctrl-p ctrl-q)");
    let status = Command::new("docker")
        .args(["attach", "--detach-keys", "ctrl-p,ctrl-q", &name])
        .status()?;
    if !status.success() {
        return Err(format!("docker attach {name} failed").into());
    }
    Ok(())
}

// ---------------------------------------------------------------------- ip

pub async fn ip(provider: &Provider, service: Option<&str>) -> Result<(), BoxedError> {
    let (cfg, _, docker) = ctx(provider);
    let name = service_name(cfg, service)?;
    match aka_services::lifecycle::ip_of(docker, cfg, &name).await? {
        Some(ip) => println!("{ip}"),
        None => return Err(format!("{name} has no ip (not running?)").into()),
    }
    Ok(())
}

// -------------------------------------------------------------------- pull

pub async fn pull(provider: &Provider) -> Result<(), BoxedError> {
    let (cfg, _, docker) = ctx(provider);
    aka_services::lifecycle::pull(docker, cfg, |line| println!("{line}")).await
}

// ------------------------------------------------------------------ routes

pub fn routes(provider: &Provider, json: bool) -> Result<(), BoxedError> {
    let paths = provider.fetch_unchecked::<AkaPaths>();
    let state = aka_services::statefile::read(&paths.state_file())?
        .ok_or("proxyd has not run yet (start with `aka up`)")?;

    if json {
        println!("{}", serde_json::to_string_pretty(&state)?);
    } else {
        print_routes(&state);
    }
    Ok(())
}

// ------------------------------------------------------------- config-file

pub fn config_file(provider: &Provider, force: bool, upgrade: bool) -> Result<(), BoxedError> {
    let selection = provider.fetch_unchecked::<ConfigSelection>();
    let target = selection
        .explicit
        .clone()
        .unwrap_or_else(aka_services::paths::default_config_path);

    if upgrade {
        aka_services::config::upgrade_config_file(&target)?;
        println!("upgraded {target:?}");
        return Ok(());
    }

    aka_services::write_default_config(&target, force)?;
    println!("wrote {target:?}");
    Ok(())
}

// ----------------------------------------------------------------- version

pub fn version() -> Result<(), BoxedError> {
    println!("aka {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}

// --------------------------------------------------------------- privileged

/// Root-side resolver writes, invoked via `sudo aka _privileged ...`.
pub fn privileged(cmd: crate::PrivilegedCmd) -> Result<(), BoxedError> {
    use crate::PrivilegedCmd as P;

    match cmd {
        P::Write {
            domain,
            nameserver,
            port,
        } => {
            let req = aka_services::privileged::Request {
                domains: domain,
                nameserver: nameserver
                    .parse()
                    .map_err(|_| format!("invalid nameserver {nameserver:?}"))?,
                port,
            };
            for path in aka_services::privileged::write(&req)? {
                println!("wrote {}", path.display());
            }
            Ok(())
        }
        P::Clean { domain } => aka_services::privileged::clean(&domain),
        P::Noop => Ok(()),
    }
}

pub fn install_sudo_rule() -> Result<(), BoxedError> {
    aka_services::privileged::install_sudo_rule()
}

// ------------------------------------------------------------------ update

/// The canonical install repo; `AKA_REPO` overrides it (same knob as install.sh).
const AKA_REPO: &str = "https://github.com/freshbrewlabs/aka.git";

/// `git clone` parses any leading-dash argument as an option (`--upload-pack`
/// runs a command), so a ref only gets these characters, never `..`.
fn valid_git_ref(git_ref: &str) -> bool {
    !git_ref.is_empty()
        && !git_ref.starts_with('-')
        && !git_ref.contains("..")
        && git_ref
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
}

fn tool_present(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run a tool with inherited stdio (the user watches cargo compile).
fn run(program: &str, args: &[&str], cwd: Option<&std::path::Path>) -> Result<(), BoxedError> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} exited with {status}").into())
    }
}

/// Where `cargo install` puts the aka binary: install root (cargo's own
/// env-var order) with `bin/aka` underneath it.
fn installed_bin() -> PathBuf {
    let root = std::env::var("CARGO_INSTALL_ROOT")
        .or_else(|_| std::env::var("CARGO_HOME"))
        .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_else(|_| "~".into()) + "/.cargo");
    PathBuf::from(root).join("bin").join("aka")
}

fn installed_version(bin: &std::path::Path) -> Option<String> {
    let out = Command::new(bin).arg("version").output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let text = text.trim().to_string();
    (!text.is_empty()).then(|| text.trim_start_matches("aka ").to_string())
}

/// Reinstall aka from the repo: clone the ref to a temp dir, then the exact
/// `cargo install --path src/aka/aka-cli --locked --force` bin/install.sh
/// runs. Same version or not, the binary is replaced; cargo removing a
/// running binary is fine on unix, and the path does not change, so the
/// sudoers rule keeps pointing at the new one.
pub fn update(provider: &Provider, git_ref: Option<&str>) -> Result<(), BoxedError> {
    let paths = provider.fetch_unchecked::<AkaPaths>();
    let git_ref = git_ref
        .map(str::to_string)
        .or_else(|| std::env::var("AKA_REF").ok())
        .unwrap_or_else(|| "main".into());
    if !valid_git_ref(&git_ref) {
        return Err(format!(
            "invalid git ref {git_ref:?} (a branch or tag: letters, digits, . _ / -)"
        )
        .into());
    }
    if !tool_present("git") {
        return Err(
            "git is required; macOS: xcode-select --install, Debian/Ubuntu: sudo apt install git"
                .into(),
        );
    }
    if !tool_present("cargo") {
        return Err("cargo (Rust) is required to update aka; install it with: \
             curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
            .into());
    }

    let repo = std::env::var("AKA_REPO").unwrap_or_else(|_| AKA_REPO.into());
    let was = env!("CARGO_PKG_VERSION");
    let clone = std::env::temp_dir().join(format!("aka-update-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&clone);

    println!("==> updating aka from {repo} ({git_ref}); a cold build takes a few minutes");
    let step = (|| -> Result<(), BoxedError> {
        let dest = clone.to_string_lossy();
        run(
            "git",
            &[
                "clone", "--quiet", "--depth", "1", "--branch", &git_ref, &repo, &dest,
            ],
            None,
        )?;
        run(
            "cargo",
            &[
                "install",
                "--path",
                "src/aka/aka-cli",
                "--locked",
                "--force",
            ],
            Some(&clone),
        )
    })();
    let _ = std::fs::remove_dir_all(&clone);
    step?;

    match installed_version(&installed_bin()).as_deref() {
        Some(now) if now != was => {
            println!("==> aka updated {was} -> {now}");
            println!("the images may have moved too: aka pull");
        }
        Some(now) => println!("==> aka {now} reinstalled from {git_ref}"),
        None => println!("==> aka reinstalled; run `aka version` to see the new version"),
    }
    if let Some(pid) = daemon_pid(&paths.pid_file()) {
        println!("proxyd (pid {pid}) still runs the old binary: aka restart");
    }
    Ok(())
}

// ------------------------------------------------------------------ proxyd

pub async fn proxyd(provider: &Provider) -> Result<(), BoxedError> {
    let (cfg, paths, _docker) = ctx(provider);

    paths.ensure_dirs()?;
    let pid_file = paths.pid_file();
    let _ = std::fs::write(&pid_file, format!("{}\n", std::process::id()));

    let selection = provider.fetch_unchecked::<ConfigSelection>();
    let config_path = selection
        .explicit
        .as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    let mut daemon = aka_services::proxyd::Proxyd::new(
        AkaDocker::connect()?,
        cfg.clone(),
        paths.clone(),
        config_path,
    );

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    let result = tokio::select! {
        result = daemon.run() => result,
        _ = sigterm.recv() => { info!("proxyd: SIGTERM"); Ok(()) }
        _ = sigint.recv() => { info!("proxyd: SIGINT"); Ok(()) }
    };

    if daemon_pid(&pid_file) == Some(std::process::id()) {
        let _ = std::fs::remove_file(&pid_file);
    }
    result
}

/// Free a host port before publishing it: real processes are offered for a
/// kill; docker-published ports are mapped back to their containers and
/// stopped (the holder socket on Docker Desktop belongs to com.docker.backend).
async fn ensure_port_free(
    docker: &AkaDocker,
    kill_others: &aka_kernel::config::KillOthers,
    port: u16,
    udp: bool,
    label: &str,
) -> Result<(), BoxedError> {
    use aka_services::conflicts;

    // Docker-published ports are the common holder (the host socket belongs
    // to com.docker.backend); a container match is definitive.
    let holders = docker.containers_publishing(port).await?;
    if !holders.is_empty() {
        if !conflicts::offer_to_stop_containers(port, &holders, kill_others) {
            return Err(format!("cannot start {label}: port {port} in use").into());
        }
        for name in &holders {
            docker.stop(name).await?;
        }
        println!("port {port} freed ({label})");
        return Ok(());
    }

    if conflicts::port_free(port, udp) {
        return Ok(());
    }

    eprintln!("port {port} is in use ({label})");
    if !conflicts::offer_to_kill(port, udp, kill_others) {
        return Err(format!("cannot start {label}: port {port} in use").into());
    }
    if !conflicts::wait_port_free(port, udp) {
        return Err(format!("port {port} still busy after freeing it").into());
    }
    println!("port {port} freed ({label})");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::valid_git_ref;

    #[test]
    fn git_refs_that_git_would_read_as_options_are_rejected() {
        assert!(valid_git_ref("main"));
        assert!(valid_git_ref("v1.2.3"));
        assert!(valid_git_ref("feature/add-update"));
        assert!(!valid_git_ref(""));
        assert!(!valid_git_ref("--upload-pack=evil"));
        assert!(!valid_git_ref("-b"));
        assert!(!valid_git_ref("a..b"));
        assert!(!valid_git_ref("v1 ; rm -rf /"));
    }
}
