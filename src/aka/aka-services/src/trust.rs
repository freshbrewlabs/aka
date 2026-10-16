//! OS trust stores for the `aka dev CA`: per-store probe / register /
//! deregister, behind an injectable [`Exec`] so unit tests never touch a
//! real keychain, NSS DB, or `sudo`.
//!
//! Trust stores change only in the `cert-gen` verbs (see the feature spec);
//! sudo prompts appear only where a store requires root — the Linux system
//! store, and the macOS system-keychain fallback. Nothing here goes through
//! `_privileged`, and the sudoers rule must stay out of this path: importing
//! a root CA as a permanent passwordless-root verb would let any local
//! process make itself trusted.

use std::path::{Path, PathBuf};

use aka_kernel::BoxedError;

use crate::certs::CA_COMMON_NAME;

/// Trust state of the CA in one store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// Present and trusted for TLS.
    Trusted,
    /// Present in the store without trust (needs re-registration).
    Present,
    /// Not in the store at all.
    Absent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    /// macOS login keychain — user-owned, needs no root.
    LoginKeychain,
    /// `$HOME/.pki/nssdb` (Chrome/Chromium, shared-with-Firefox). No root.
    Nss,
    /// `/usr/local/share/ca-certificates` + `update-ca-certificates`. Root.
    DebianStore,
    /// `/etc/pki/ca-trust/source/anchors` + `update-ca-trust`. Root.
    FedoraStore,
}

const DEBIAN_ANCHOR: &str = "/usr/local/share/ca-certificates/aka-ca.crt";
const FEDORA_ANCHOR: &str = "/etc/pki/ca-trust/source/anchors/aka-ca.crt";

impl Store {
    pub fn label(self) -> &'static str {
        match self {
            Store::LoginKeychain => "login keychain",
            Store::Nss => "nssdb",
            Store::DebianStore | Store::FedoraStore => "system store",
        }
    }
}

/// External effects, so tests can script the whole state machine.
pub trait Exec {
    /// Run and capture stdout; `(success, stdout)`.
    fn capture(&self, program: &str, args: &[&str]) -> Result<(bool, String), BoxedError>;
    /// Whether `program` is on `PATH`.
    fn which(&self, program: &str) -> bool;
    /// `sudo [-n] …` with the prompting fallback — the same path
    /// `install_sudo_rule` uses. Err on a declined prompt.
    fn sudo(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<(), BoxedError>;
}

/// The production [`Exec`]: real processes, real `sudo`.
pub struct Process;

impl Exec for Process {
    fn capture(&self, program: &str, args: &[&str]) -> Result<(bool, String), BoxedError> {
        let out = std::process::Command::new(program).args(args).output()?;
        Ok((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        ))
    }

    fn which(&self, program: &str) -> bool {
        std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| dir.join(program).is_file())
        })
    }

    fn sudo(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<(), BoxedError> {
        crate::privileged::run_sudo(args, stdin)
    }
}

/// The stores this machine supports. Linux skips absent tools/stores; an
/// empty vec means nothing can be automated and the caller must print the
/// manual commands.
pub fn detect(exec: &dyn Exec, os: &str) -> Vec<Store> {
    match os {
        "macos" => vec![Store::LoginKeychain],
        "linux" => {
            let mut stores = Vec::new();
            if exec.which("certutil") {
                stores.push(Store::Nss);
            }
            if exec.which("update-ca-certificates") {
                stores.push(Store::DebianStore);
            } else if exec.which("update-ca-trust") {
                stores.push(Store::FedoraStore);
            }
            stores
        }
        _ => Vec::new(),
    }
}

/// Manual fallback lines for whatever `detect` could not automate.
pub fn manual_commands(ca_path: &Path) -> Vec<String> {
    let ca = ca_path.display();
    vec![
        format!("certutil -d sql:$HOME/.pki/nssdb -A -n \"{CA_COMMON_NAME}\" -t \"C,,\" -i {ca}"),
        format!("sudo tee {DEBIAN_ANCHOR} < {ca} && sudo update-ca-certificates"),
    ]
}

fn nss_db(home: &Path) -> String {
    format!("sql:{}/.pki/nssdb", home.display())
}

fn login_keychain(home: &Path) -> PathBuf {
    home.join("Library/Keychains/login.keychain-db")
}

fn anchor(store: Store) -> &'static str {
    match store {
        Store::DebianStore => DEBIAN_ANCHOR,
        Store::FedoraStore => FEDORA_ANCHOR,
        _ => unreachable!("only system stores have anchors"),
    }
}

fn updater(store: Store) -> &'static str {
    match store {
        Store::DebianStore => "update-ca-certificates",
        Store::FedoraStore => "update-ca-trust",
        _ => unreachable!("only system stores have updaters"),
    }
}

/// Current trust state of one store. A missing store (no NSS DB, no trust
/// settings, no anchor file) is [`Trust::Absent`]; failures of a present
/// store are errors, because guessing wrong here strands or duplicates
/// entries.
pub fn probe(exec: &dyn Exec, home: &Path, store: Store) -> Result<Trust, BoxedError> {
    match store {
        Store::LoginKeychain => {
            let (ok, out) = exec.capture("security", &["dump-trust-settings"])?;
            if !ok {
                return Ok(Trust::Absent); // no user trust settings at all
            }
            // A certificate merely *imported* has no trust settings;
            // trust settings, not cert presence, are the truth.
            Ok(if out.lines().any(|l| l.contains(CA_COMMON_NAME)) {
                Trust::Trusted
            } else {
                Trust::Absent
            })
        }
        Store::Nss => {
            if !home.join(".pki/nssdb").is_dir() {
                return Ok(Trust::Absent);
            }
            let db = nss_db(home);
            let (ok, out) = exec.capture("certutil", &["-L", "-d", &db, "-n", CA_COMMON_NAME])?;
            if !ok {
                return Ok(Trust::Absent);
            }
            // `certutil -L` prints nickname-padded lines ending in the trust
            // flags; web-trust CA is a leading `C` (e.g. `CT,,`). The
            // nickname's own "CA" must not be read as a flag.
            let trusted = out.lines().any(|l| {
                l.contains(CA_COMMON_NAME)
                    && l.split_whitespace()
                        .next_back()
                        .is_some_and(|flags| flags.starts_with('C'))
            });
            Ok(if trusted { Trust::Trusted } else { Trust::Present })
        }
        Store::DebianStore | Store::FedoraStore => Ok(if Path::new(anchor(store)).is_file() {
            // A present anchor: assume it went through the updater;
            // `register` re-runs the updater to re-confirm.
            Trust::Trusted
        } else {
            Trust::Absent
        }),
    }
}

/// Register the CA in one store; no-op when already trusted (the system
/// store re-confirms by re-running its updater).
pub fn register(exec: &dyn Exec, home: &Path, store: Store, ca_pem: &Path) -> Result<(), BoxedError> {
    if probe(exec, home, store)? == Trust::Trusted && !uses_updater(store) {
        return Ok(());
    }
    let ca = ca_pem.display().to_string();

    match store {
        Store::LoginKeychain => {
            let keychain = login_keychain(home).display().to_string();
            let (ok, out) = exec.capture("security", &["add-trusted-cert", "-k", &keychain, &ca])?;
            if !ok {
                return Err(format!("security add-trusted-cert failed: {}", out.trim()).into());
            }
        }
        Store::Nss => {
            let db = nss_db(home);
            if !home.join(".pki/nssdb").is_dir() {
                let (ok, out) = exec.capture("certutil", &["-N", "--empty-password", "-d", &db])?;
                if !ok {
                    return Err(format!("certutil -N failed: {}", out.trim()).into());
                }
            }
            if probe(exec, home, store)? == Trust::Present {
                // Entry without usable trust flags: replace it outright.
                exec.capture("certutil", &["-D", "-d", &db, "-n", CA_COMMON_NAME])?;
            }
            let (ok, out) = exec.capture(
                "certutil",
                &["-A", "-d", &db, "-n", CA_COMMON_NAME, "-t", "C,,", "-i", &ca],
            )?;
            if !ok {
                return Err(format!("certutil -A failed: {}", out.trim()).into());
            }
        }
        Store::DebianStore | Store::FedoraStore => {
            let contents = std::fs::read(ca_pem)?;
            exec.sudo(&["tee", anchor(store)], Some(&contents))?;
            exec.sudo(&[updater(store)], None)?;
        }
    }
    Ok(())
}

/// Remove the CA from one store. Absent is success (idempotent).
pub fn deregister(exec: &dyn Exec, home: &Path, store: Store) -> Result<(), BoxedError> {
    match store {
        Store::LoginKeychain => {
            if probe(exec, home, store)? == Trust::Absent {
                return Ok(());
            }
            let keychain = login_keychain(home).display().to_string();
            let (ok, out) = exec.capture(
                "security",
                &["delete-certificate", "-c", CA_COMMON_NAME, &keychain],
            )?;
            if !ok {
                return Err(format!("security delete-certificate failed: {}", out.trim()).into());
            }
        }
        Store::Nss => {
            if probe(exec, home, store)? == Trust::Absent {
                return Ok(());
            }
            let db = nss_db(home);
            let (ok, out) = exec.capture("certutil", &["-D", "-d", &db, "-n", CA_COMMON_NAME])?;
            if !ok {
                return Err(format!("certutil -D failed: {}", out.trim()).into());
            }
        }
        Store::DebianStore | Store::FedoraStore => {
            if !Path::new(anchor(store)).is_file() {
                return Ok(());
            }
            exec.sudo(&["rm", "-f", anchor(store)], None)?;
            exec.sudo(&[updater(store)], None)?;
        }
    }
    Ok(())
}

fn uses_updater(store: Store) -> bool {
    matches!(store, Store::DebianStore | Store::FedoraStore)
}

/// Register every store that lacks the CA. The error carries which store
/// failed (a declined sudo prompt stops here); earlier stores are done.
pub fn register_all(
    exec: &dyn Exec,
    home: &Path,
    stores: &[Store],
    ca_pem: &Path,
) -> Result<Vec<String>, BoxedError> {
    let mut lines = Vec::new();
    for &store in stores {
        register(exec, home, store, ca_pem)?;
        lines.push(format!(
            "trust: {CA_COMMON_NAME} added to {} (remove: aka cert-gen --untrust)",
            store.label()
        ));
    }
    if lines.is_empty() {
        for line in manual_commands(ca_pem) {
            lines.push(format!("trust: no supported store found; manual: {line}"));
        }
    }
    Ok(lines)
}

/// Deregister every store that holds the CA. A failed removal (declined
/// prompt) aborts with an error so callers holding file-deletion steps
/// behind this never strand a trusted root without its files — or vice versa.
pub fn unregister_all(
    exec: &dyn Exec,
    home: &Path,
    stores: &[Store],
) -> Result<Vec<String>, BoxedError> {
    let mut lines = Vec::new();
    for &store in stores {
        // Only report stores that actually held something; absent is a
        // silent success, so `--untrust` never claims a removal that
        // nothing did.
        if probe(exec, home, store)? == Trust::Absent {
            continue;
        }
        deregister(exec, home, store)?;
        lines.push(format!(
            "untrust: {CA_COMMON_NAME} removed from {}",
            store.label()
        ));
    }
    Ok(lines)
}

/// Per-store status for issuance runs — probing never prompts and never
/// changes anything. Returns `(lines, gap)`; a gap means some supported
/// store is not trusted and the caller should print `run: aka cert-gen --trust`.
pub fn status_lines(exec: &dyn Exec, home: &Path, stores: &[Store]) -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    let mut gap = false;
    for &store in stores {
        match probe(exec, home, store) {
            Ok(Trust::Trusted) => lines.push(format!("trust[{}]: trusted", store.label())),
            Ok(state) => {
                gap = true;
                let state = if state == Trust::Present {
                    "present, not trusted"
                } else {
                    "not installed"
                };
                lines.push(format!("trust[{}]: {state}", store.label()));
            }
            Err(err) => {
                gap = true;
                lines.push(format!("trust[{}]: probe failed: {err}", store.label()));
            }
        }
    }
    (lines, gap)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// Scripted stand-in: canned `capture` results in call order, `PATH`
    /// membership, and a log of every external effect. `sudo` can be made
    /// to fail to simulate a declined prompt.
    struct Fake {
        whiches: Vec<String>,
        captures: RefCell<VecDeque<(bool, String)>>,
        sudo_fails: bool,
        log: RefCell<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                whiches: Vec::new(),
                captures: RefCell::new(VecDeque::new()),
                sudo_fails: false,
                log: RefCell::new(Vec::new()),
            }
        }

        fn with_which(mut self, programs: &[&str]) -> Self {
            self.whiches = programs.iter().map(|p| p.to_string()).collect();
            self
        }

        fn responding(mut self, responses: Vec<(bool, &str)>) -> Self {
            self.captures = RefCell::new(
                responses
                    .into_iter()
                    .map(|(ok, s)| (ok, s.to_string()))
                    .collect(),
            );
            self
        }

        fn with_sudo_failure(mut self) -> Self {
            self.sudo_fails = true;
            self
        }

        fn log(&self) -> Vec<String> {
            self.log.borrow().clone()
        }
    }

    /// A home dir with (or without) the NSS DB present, so the
    /// filesystem-gated probe paths are deliberate in every test.
    struct FakeHome {
        path: PathBuf,
        has_nssdb: bool,
    }

    impl FakeHome {
        fn new(tag: &str, has_nssdb: bool) -> Self {
            let path =
                std::env::temp_dir().join(format!("aka-trust-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            if has_nssdb {
                std::fs::create_dir_all(path.join(".pki/nssdb")).unwrap();
            } else {
                std::fs::create_dir_all(&path).unwrap();
            }
            Self { path, has_nssdb }
        }
    }

    impl Drop for FakeHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    impl Exec for Fake {
        fn capture(&self, program: &str, args: &[&str]) -> Result<(bool, String), BoxedError> {
            self.log
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            Ok(self
                .captures
                .borrow_mut()
                .pop_front()
                .unwrap_or((false, String::new())))
        }

        fn which(&self, program: &str) -> bool {
            self.whiches.iter().any(|p| p == program)
        }

        fn sudo(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<(), BoxedError> {
            let input = stdin
                .map(|b| format!(" < {} bytes", b.len()))
                .unwrap_or_default();
            self.log
                .borrow_mut()
                .push(format!("sudo {}{input}", args.join(" ")));
            if self.sudo_fails {
                Err("sudo declined".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn detection_follows_os_and_tools() {
        assert_eq!(detect(&Fake::new(), "macos"), vec![Store::LoginKeychain]);
        assert_eq!(detect(&Fake::new(), "linux"), Vec::<Store>::new());

        let both = Fake::new().with_which(&["certutil", "update-ca-certificates"]);
        assert_eq!(
            detect(&both, "linux"),
            vec![Store::Nss, Store::DebianStore],
            "NSS first, then the distro system store"
        );

        let fedora = Fake::new().with_which(&["update-ca-trust"]);
        assert_eq!(detect(&fedora, "linux"), vec![Store::FedoraStore]);

        let browserless = Fake::new().with_which(&["certutil"]);
        assert_eq!(detect(&browserless, "linux"), vec![Store::Nss]);

        assert!(detect(&Fake::new(), "windows").is_empty());
    }

    #[test]
    fn nss_probe_reads_the_trust_flags_not_the_nickname() {
        let home = FakeHome::new("probe", true);
        let trusted = Fake::new().responding(vec![(
            true,
            "Nickname                                       Trust Flags Final Issuer\n\
             aka dev CA                                     CT,,\n",
        )]);
        assert_eq!(probe(&trusted, &home.path, Store::Nss).unwrap(), Trust::Trusted);

        let untrusted = Fake::new()
            .responding(vec![(true, "aka dev CA                                     u,,\n")]);
        assert_eq!(
            probe(&untrusted, &home.path, Store::Nss).unwrap(),
            Trust::Present
        );
    }

    #[test]
    fn missing_nssdb_probes_absent_without_running_tools() {
        let home = FakeHome::new("nodb", false);
        let exec = Fake::new();
        assert_eq!(probe(&exec, &home.path, Store::Nss).unwrap(), Trust::Absent);
        assert!(exec.log().is_empty(), "the dir gate short-circuits certutil");
    }

    #[test]
    fn imported_without_trust_settings_is_not_trusted() {
        let unrelated = Fake::new()
            .responding(vec![(true, "Cert 1: cert \"other cert\" (0x01)\n")]);
        assert_eq!(
            probe(&unrelated, Path::new("/home/dev"), Store::LoginKeychain).unwrap(),
            Trust::Absent,
            "cert presence is not trust settings"
        );

        let ours = Fake::new().responding(vec![(true, "Cert 1: cert \"aka dev CA\" (0x1e)\n")]);
        assert_eq!(
            probe(&ours, Path::new("/home/dev"), Store::LoginKeychain).unwrap(),
            Trust::Trusted
        );
    }

    #[test]
    fn nss_registration_creates_db_then_entry() {
        let home = FakeHome::new("reg", false);
        let ca = home.path.join("ca.crt");
        std::fs::write(&ca, "PEM").unwrap();
        // probe (dir gate, no capture) -> certutil -N -> inner probe (dir
        // still absent to the fs: the fake -N creates nothing) -> certutil -A.
        let exec = Fake::new().responding(vec![(true, ""), (true, "")]);
        register(&exec, &home.path, Store::Nss, &ca).unwrap();

        let log = exec.log();
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(log[0].starts_with("certutil -N --empty-password"), "{log:?}");
        assert!(
            log[1].contains("-A")
                && log[1].contains("-t C,,")
                && log[1].contains(&format!("-n {CA_COMMON_NAME}")),
            "{log:?}"
        );
    }

    #[test]
    fn untrusted_entry_is_replaced_before_add() {
        let home = FakeHome::new("repl", true);
        let ca = home.path.join("ca.crt");
        std::fs::write(&ca, "PEM").unwrap();
        let exec = Fake::new().responding(vec![
            (true, "aka dev CA                                     u,,\n"), // probe: Present
            (true, "aka dev CA                                     u,,\n"), // re-probe inside the removal
            (true, ""),           // -D
            (true, ""),           // -A
        ]);
        register(&exec, &home.path, Store::Nss, &ca).unwrap();

        let log = exec.log();
        let d = log
            .iter()
            .position(|l| l.starts_with("certutil -D"))
            .unwrap();
        let a = log
            .iter()
            .position(|l| l.starts_with("certutil -A"))
            .unwrap();
        assert!(d < a, "existing entry removed before re-add: {log:?}");
    }

    #[test]
    fn trusted_store_registration_is_a_no_op_except_updater_recheck() {
        let home = FakeHome::new("noop", true);
        let ca = home.path.join("ca.crt");
        std::fs::write(&ca, "PEM").unwrap();
        let exec = Fake::new().responding(vec![(true, "aka dev CA           CT,,")]);
        register(&exec, &home.path, Store::Nss, &ca).unwrap();
        assert_eq!(
            exec.log(),
            vec![format!(
                "certutil -L -d sql:{}/.pki/nssdb -n aka dev CA",
                home.path.display()
            )]
        );
    }

    #[test]
    fn system_store_writes_anchor_via_sudo_then_re_runs_updater() {
        // The real anchor file cannot exist on a test host, so the store
        // probes Absent and registration goes straight to the sudo legs.
        let home = FakeHome::new("sys", false);
        let ca = home.path.join("ca.crt");
        std::fs::write(&ca, "PEMDATA").unwrap();
        let exec = Fake::new();
        register(&exec, &home.path, Store::DebianStore, &ca).unwrap();

        let log = exec.log();
        assert_eq!(
            log,
            vec![
                format!("sudo tee {DEBIAN_ANCHOR} < 7 bytes"),
                "sudo update-ca-certificates".to_string(),
            ]
        );
    }

    #[test]
    fn declined_root_step_aborts_with_error_so_callers_delete_nothing() {
        // register: the sudo leg failing (declined prompt) must surface.
        let home = FakeHome::new("decline-r", false);
        let ca = home.path.join("ca.crt");
        std::fs::write(&ca, "PEM").unwrap();
        let exec = Fake::new().with_sudo_failure();
        let err = register(&exec, &home.path, Store::FedoraStore, &ca).unwrap_err();
        assert!(err.to_string().contains("declined"), "{err}");
        assert_eq!(
            exec.log(),
            vec![format!("sudo tee {FEDORA_ANCHOR} < 3 bytes")],
            "the updater never ran after the refusal"
        );

        // deregister: a failing removal must abort the loop (files intact).
        let exec = Fake::new().responding(vec![
            (true, "aka dev CA"),   // unregister_all probe: held
            (true, "aka dev CA"),   // deregister's internal probe
            (false, "needs admin"), // delete-certificate refused
        ]);
        let err = unregister_all(&exec, Path::new("/home/dev"), &[Store::LoginKeychain, Store::Nss])
            .unwrap_err();
        assert!(err.to_string().contains("delete-certificate"), "{err}");
        assert!(
            !exec.log().iter().any(|l| l.starts_with("certutil")),
            "later stores untouched after the abort: {:?}",
            exec.log()
        );
    }

    #[test]
    fn idempotent_deregistration_of_absent_stores() {
        let home = FakeHome::new("idem", false);
        // System stores: anchor files cannot exist on a test host.
        let exec = Fake::new();
        let lines =
            unregister_all(&exec, &home.path, &[Store::DebianStore, Store::FedoraStore]).unwrap();
        assert_eq!(lines.len(), 0, "absent stores report nothing");
        assert!(
            exec.log().is_empty(),
            "absent stores touch nothing: {:?}",
            exec.log()
        );

        // Keychain probes absent; NSS dir gate short-circuits.
        let exec = Fake::new().responding(vec![(true, "no trust settings here")]);
        let lines =
            unregister_all(&exec, &home.path, &[Store::LoginKeychain, Store::Nss]).unwrap();
        assert!(lines.is_empty(), "absent is a quiet success");
    }

    #[test]
    fn status_lines_flag_gaps_and_only_probe() {
        let home = FakeHome::new("status", true);
        let exec = Fake::new().responding(vec![
            (true, "aka dev CA           CT,,"),   // nss: trusted
            (true, "some other cert only"),        // mac: absent -> gap
        ]);
        let (lines, gap) = status_lines(&exec, &home.path, &[Store::Nss, Store::LoginKeychain]);
        assert!(gap, "one gap is enough to suggest --trust");
        assert!(lines[0].contains("nssdb]: trusted"));
        assert!(lines[1].contains("login keychain]: not installed"));

        assert!(
            exec.log()
                .iter()
                .all(|l| l.starts_with("certutil -L") || l.starts_with("security dump-trust-settings")),
            "status only probes: {:?}",
            exec.log()
        );
    }

    #[test]
    fn empty_detection_prints_manual_commands() {
        let lines = register_all(&Fake::new(), Path::new("/home/dev"), &[], Path::new("/tmp/ca.crt"))
            .unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("manual: certutil"));
        assert!(lines[1].contains("sudo tee"));
    }
}
