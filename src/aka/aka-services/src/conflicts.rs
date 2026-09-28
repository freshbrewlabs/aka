//! Host port bookkeeping: detect what already holds a port, offer to kill it
//! (dory's `kill_others`). On Docker Desktop the host probe cannot see ports
//! published by containers (they live in the VM), so this detects real host
//! processes; docker-publish conflicts surface as container-start errors.

use aka_kernel::BoxedError;
use aka_kernel::config::KillOthers;
use std::io::Write;
use std::process::{Command, Stdio};
use tracing::{debug, info};

/// Ports cannot be probed with bind() on modern macOS (privileged ports
/// deny unprivileged binds outright, and wildcard binds collide with
/// unrelated TIME_WAIT sockets). The truth is `lsof`: LISTEN tcp sockets and
/// bound udp sockets on `port`, any process. (Root-owned holders need
/// sudo-elevated lsof; best effort, same tradeoff as dory took.)
pub fn listener_pids_all(port: u16, udp: bool) -> Vec<u32> {
    let mut pids = listener_pids(port);
    if udp {
        let output = Command::new("lsof")
            .args(["-nP", "-t", &format!("-iUDP:{port}")])
            .stderr(Stdio::null())
            .output();
        if let Ok(out) = output
            && out.status.success()
        {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                if let Ok(pid) = line.trim().parse::<u32>()
                    && !pids.contains(&pid)
                {
                    pids.push(pid);
                }
            }
        }
    }
    pids
}

/// True when no process listens on the port (docker-published ports are
/// checked separately via containers_publishing; they live in the VM).
pub fn port_free(port: u16, udp: bool) -> bool {
    listener_pids_all(port, udp).is_empty()
}
/// PIDs of host processes listening on tcp/`port` (lsof on macOS, ss fallback
/// via lsof on linux). Best effort; empty when unresolvable.
pub fn listener_pids(port: u16) -> Vec<u32> {
    let output = Command::new("lsof")
        .args(["-nP", "-t", &format!("-iTCP:{port}"), "-sTCP:LISTEN"])
        .stderr(Stdio::null())
        .output();

    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect(),
        Ok(_) => {
            debug!("no lsof listeners on {port}");
            Vec::new()
        }
        Err(err) => {
            debug!("lsof unavailable: {err}");
            Vec::new()
        }
    }
}

/// Process names behind a port, for human-facing conflict messages.
pub fn listener_descriptions(port: u16) -> Vec<String> {
    let output = Command::new("lsof")
        .args(["-nP", "-sTCP:LISTEN"])
        .arg(format!("-iTCP:{port}"))
        .stderr(Stdio::null())
        .output();

    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .skip(1)
            .map(|line| {
                let mut fields = line.split_whitespace();
                let command = fields.next().unwrap_or("?");
                let pid = fields.next().unwrap_or("?");
                format!("{command} (pid {pid})")
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Kill the listed pids via `sudo kill`.
pub fn kill_pids(pids: &[u32]) -> Result<(), BoxedError> {
    let mut cmd = Command::new("sudo");
    cmd.arg("kill");
    for pid in pids {
        cmd.arg(pid.to_string());
    }
    let status = cmd.status()?;
    if !status.success() {
        return Err(format!("sudo kill failed ({status})").into());
    }
    Ok(())
}

/// Offer to free `port`, honouring `kill_others`. Returns true when the port
/// is (or should be) usable after this call.
pub fn offer_to_kill(port: u16, udp: bool, kill_others: &KillOthers) -> bool {
    let pids = listener_pids_all(port, udp);
    if pids.is_empty() {
        eprintln!(
            "port {port} is in use by something the port probe cannot identify \
             (perhaps a docker-published port); stop it or change aka.dns.port"
        );
        return false;
    }

    let descriptions = listener_descriptions(port);
    for description in &descriptions {
        eprintln!("process {description} is listening on port {port}");
    }
    eprintln!("this interferes with aka's dns service (dns.kill_others: {port})");

    let answer = match kill_others.answer() {
        Some(answer) => answer,
        None => {
            eprint!(
                "kill PID(s) {}? (Y/N): ",
                pids.iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            std::io::stderr().flush().ok();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
                false
            } else {
                line.trim().starts_with('y') || line.trim().starts_with('Y')
            }
        }
    };

    if !answer {
        eprintln!("not killing anything; free port {port} manually and retry");
        return false;
    }

    match kill_pids(&pids) {
        Ok(()) => {
            info!("killed pids on port {port}");
            true
        }
        Err(err) => {
            eprintln!("{err}");
            false
        }
    }
}

/// Offer to stop docker containers publishing `port`; honours `kill_others`.
/// Returns true when the containers should be stopped.
pub fn offer_to_stop_containers(port: u16, names: &[String], kill_others: &KillOthers) -> bool {
    eprintln!(
        "docker container(s) {} publish port {port}",
        names.join(", ")
    );
    eprintln!("this interferes with aka's services (dns.kill_others: {port})");

    let answer = match kill_others.answer() {
        Some(answer) => answer,
        None => {
            eprint!("stop {}? (Y/N): ", names.join(", "));
            std::io::stderr().flush().ok();
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
                false
            } else {
                line.trim().starts_with('y') || line.trim().starts_with('Y')
            }
        }
    };

    if !answer {
        eprintln!("not stopping anything; free port {port} manually and retry");
        return false;
    }
    true
}

/// Wait (up to ~4s) for lsof listeners on the port to disappear.
pub fn wait_port_free(port: u16, udp: bool) -> bool {
    for _ in 0..40 {
        if port_free(port, udp) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    port_free(port, udp)
}
