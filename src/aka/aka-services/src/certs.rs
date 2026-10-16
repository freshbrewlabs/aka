use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A `<host>.crt`/`<host>.key` pair found in `ssl_certs_dir`.
#[derive(Debug, Clone)]
pub struct CertPair {
    pub host: String,
    pub cert: PathBuf,
    pub key: PathBuf,
}

/// Scan `dir` for `<host>.crt` + `<host>.key` pairs (both must exist).
pub fn find_cert_pairs(dir: &str) -> BTreeMap<String, CertPair> {
    let mut pairs = BTreeMap::new();
    if dir.is_empty() {
        return pairs;
    }

    let Ok(entries) = std::fs::read_dir(Path::new(dir)) else {
        return pairs;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("crt") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let key = path.with_extension("key");
        if key.is_file() {
            pairs.insert(
                stem.to_owned(),
                CertPair {
                    host: stem.to_owned(),
                    cert: path,
                    key,
                },
            );
        }
    }

    pairs
}

// ------------------------------------------------- certificate generation

use aka_kernel::BoxedError;
use x509_parser::prelude::FromDer as _;

/// Dotted OIDs of the two signature algorithms aka issues; x509-parser
/// exposes no constants to compare against, so names are spelled out.
const SIG_ECDSA_SHA256: &str = "1.2.840.10045.4.3.2";
const SIG_RSA_SHA256: &str = "1.2.840.113549.1.1.11";

/// Subject of the local CA; also its trust-store nickname and the issuer
/// name `--clean` checks pairs against.
pub const CA_COMMON_NAME: &str = "aka dev CA";

/// Validity of generated key material, in days.
pub const CA_VALIDITY_DAYS: i64 = 3650;
pub const LEAF_VALIDITY_DAYS: i64 = 365;

const RSA_CA_BITS: usize = 4096;
const RSA_LEAF_BITS: usize = 2048;

/// Key algorithm for *newly created* key material. An existing CA's algorithm
/// is fixed at creation; a leaf's key is independent of the CA's (the chain
/// signature comes from the issuer), so the choice never conflicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAlg {
    /// ECDSA P-256 — the default for CA and leaves alike.
    Ecdsa,
    /// RSA 4096 (CA) / 2048 (leaf), opt-in.
    Rsa,
}

/// The local CA as an issuer: paths, fingerprint, and the material needed to
/// issue without re-parsing.
pub struct Ca {
    pub created: bool,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    /// SHA-256 over the certificate DER, colon-separated.
    pub fingerprint: String,
    key: rcgen::KeyPair,
    params: rcgen::CertificateParams,
}

/// True when both CA files exist already.
pub fn ca_exists(dir: &Path) -> bool {
    dir.join("ca.crt").is_file() && dir.join("ca.key").is_file()
}

/// Load the CA, or create it (dir 0700, key 0600). Never overwrites.
pub fn ensure_ca(dir: &Path, alg: KeyAlg) -> Result<Ca, BoxedError> {
    let cert_path = dir.join("ca.crt");
    let key_path = dir.join("ca.key");

    if cert_path.is_file() && key_path.is_file() {
        let cert_pem = std::fs::read_to_string(&cert_path)?;
        let key_pem = std::fs::read_to_string(&key_path)?;
        return Ok(Ca {
            created: false,
            fingerprint: fingerprint(&cert_pem)?,
            key: load_key(&key_pem)?,
            params: ca_params(now(), CA_VALIDITY_DAYS),
            cert_path,
            key_path,
        });
    }

    make_dir(dir)?;
    let key = generate_key(alg, RSA_CA_BITS)?;
    let params = ca_params(now(), CA_VALIDITY_DAYS);
    let cert = params.self_signed(&key)?;
    write_key(&key_path, key.serialize_pem().as_bytes())?;
    std::fs::write(&cert_path, cert.pem().as_bytes())?;

    Ok(Ca {
        created: true,
        fingerprint: fingerprint(&cert.pem())?,
        key,
        params,
        cert_path,
        key_path,
    })
}

/// Issue `<host>.crt` + `<host>.key` into `out_dir` (created 0700, key 0600).
/// An existing pair is overwritten — renewal is the whole point.
pub fn generate_cert(
    ca: &Ca,
    host: &str,
    alg: KeyAlg,
    out_dir: &Path,
) -> Result<(PathBuf, PathBuf), BoxedError> {
    validate_host(host)?;
    make_dir(out_dir)?;

    let mut params = leaf_params(host)?;
    params.not_before = now();
    params.not_after = params.not_before + time::Duration::days(LEAF_VALIDITY_DAYS);

    let key = generate_key(alg, RSA_LEAF_BITS)?;
    let issuer = rcgen::Issuer::from_params(&ca.params, &ca.key);
    let cert = params.signed_by(&key, &issuer)?;

    let crt = out_dir.join(format!("{host}.crt"));
    let key_path = out_dir.join(format!("{host}.key"));
    std::fs::write(&crt, cert.pem().as_bytes())?;
    write_key(&key_path, key.serialize_pem().as_bytes())?;
    Ok((crt, key_path))
}

/// Reject anything that is not a literal DNS host name: no wildcards, no
/// spaces, no empty or malformed labels.
pub fn validate_host(host: &str) -> Result<(), BoxedError> {
    let bad = |why: &str| format!("invalid host {host:?}: {why}");
    if host.is_empty() || host.len() > 253 {
        return Err(bad("length must be 1..=253").into());
    }
    for label in host.split('.') {
        if label.is_empty() {
            return Err(bad("empty label").into());
        }
        if label.len() > 63 {
            return Err(bad("label longer than 63 chars").into());
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(bad("label may not start or end with '-'").into());
        }
        if !label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err(bad("label characters must be alphanumeric or '-'").into());
        }
    }
    Ok(())
}

/// SHA-256 fingerprint of a PEM certificate, colon-separated uppercase hex.
pub fn fingerprint(cert_pem: &str) -> Result<String, BoxedError> {
    use sha2::{Digest as _, Sha256};
    let der = pem_der(cert_pem)?;
    Ok(Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":"))
}

/// The issuer common name of a PEM certificate, if it has one.
pub fn issuer_common_name(cert_pem: &str) -> Result<Option<String>, BoxedError> {
    let der = pem_der(cert_pem)?;
    let (_, cert) = x509_parser::prelude::X509Certificate::from_der(&der)
        .map_err(|err| format!("malformed certificate: {err}"))?;
    Ok(cn_of(cert.issuer()))
}

/// True when the PEM certificate was issued by the CA in `ca_pem`: issuer CN
/// matches `CA_COMMON_NAME` *and* the chain signature verifies against the
/// CA's public key. A certificate that merely claims the name — a copy, a
/// different CA with the same CN — fails verification.
pub fn is_aka_issued(ca_pem: &str, cert_pem: &str) -> Result<bool, BoxedError> {
    if issuer_common_name(cert_pem)?.as_deref() != Some(CA_COMMON_NAME) {
        return Ok(false);
    }
    chain_verifies(ca_pem, cert_pem)
}

fn chain_verifies(ca_pem: &str, cert_pem: &str) -> Result<bool, BoxedError> {
    let ca_der = pem_der(ca_pem)?;
    let leaf_der = pem_der(cert_pem)?;
    let (_, leaf) = x509_parser::prelude::X509Certificate::from_der(&leaf_der)
        .map_err(|err| format!("malformed certificate: {err}"))?;

    let (leaf_tbs, _) = tbs_tlv(&leaf_der).ok_or("malformed leaf certificate")?;
    let (_, ca_pubkey) = spki_tlv(&ca_der).ok_or("malformed CA certificate")?;

    use ring::signature::{
        ECDSA_P256_SHA256_ASN1, RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey,
    };
    let sig_alg = leaf.signature_algorithm.algorithm.to_string();
    let sig: &[u8] = &leaf.signature_value.data;
    let verified = if sig_alg == SIG_ECDSA_SHA256 {
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, ca_pubkey)
            .verify(leaf_tbs, sig)
            .is_ok()
    } else if sig_alg == SIG_RSA_SHA256 {
        // ring's RSA verifier takes the PKCS#1 RSAPublicKey — the BIT STRING
        // body of the SPKI, unused-bit prefix dropped — not the whole SPKI.
        UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, ca_pubkey)
            .verify(leaf_tbs, sig)
            .is_ok()
    } else {
        return Err(format!("unsupported certificate signature algorithm {sig_alg}").into());
    };
    Ok(verified)
}

fn now() -> time::OffsetDateTime {
    let n = time::OffsetDateTime::now_utc();
    n - time::Duration::nanoseconds(i64::from(n.nanosecond()))
}

fn ca_params(
    not_before: time::OffsetDateTime,
    days: i64,
) -> rcgen::CertificateParams {
    use rcgen::{BasicConstraints, DnType, DistinguishedName, IsCa, KeyUsagePurpose};
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, CA_COMMON_NAME);
    let mut params = rcgen::CertificateParams::default();
    params.not_before = not_before;
    params.not_after = not_before + time::Duration::days(days);
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
    ];
    params
}

fn leaf_params(host: &str) -> Result<rcgen::CertificateParams, BoxedError> {
    use rcgen::{DnType, ExtendedKeyUsagePurpose, KeyUsagePurpose};
    let mut params = rcgen::CertificateParams::new(vec![host.to_owned()])?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.distinguished_name.push(DnType::CommonName, host);
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    Ok(params)
}

fn generate_key(alg: KeyAlg, rsa_bits: usize) -> Result<rcgen::KeyPair, BoxedError> {
    match alg {
        KeyAlg::Ecdsa => Ok(rcgen::KeyPair::generate()?),
        // rcgen only generates RSA under its aws-lc provider; the pure-Rust
        // `rsa` crate keys in, and rcgen signs with it through PKCS#8.
        KeyAlg::Rsa => {
            use rsa::pkcs8::EncodePrivateKey as _;
            let key = rsa::RsaPrivateKey::new(&mut rand_core::OsRng, rsa_bits)?;
            let pkcs8 =
                rustls_pki_types::PrivatePkcs8KeyDer::from(key.to_pkcs8_der()?.as_bytes().to_vec());
            Ok(rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
                &pkcs8,
                &rcgen::PKCS_RSA_SHA256,
            )?)
        }
    }
}

fn load_key(pem: &str) -> Result<rcgen::KeyPair, BoxedError> {
    Ok(rcgen::KeyPair::from_pem(pem)?)
}

fn pem_der(cert_pem: &str) -> Result<Vec<u8>, BoxedError> {
    let (rest, pem) = x509_parser::pem::parse_x509_pem(cert_pem.as_bytes())
        .map_err(|err| format!("malformed PEM certificate: {err}"))?;
    if !rest.iter().all(u8::is_ascii_whitespace) {
        return Err("trailing data after PEM certificate".into());
    }
    Ok(pem.contents)
}

fn cn_of(name: &x509_parser::x509::X509Name<'_>) -> Option<String> {
    name.iter_common_name().next().map(|atv| {
        atv.attr_value()
            .as_str()
            .unwrap_or("<binary>")
            .to_owned()
    })
}

/// One DER tag-length-value entry: tag, total entry length, value, rest.
fn der_entry(buf: &[u8]) -> Option<(u8, usize, &[u8], &[u8])> {
    let (&tag, rest) = buf.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (len, body) = if first & 0x80 == 0 {
        (first as usize, rest)
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        let len = rest[..n]
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | *b as usize);
        (len, &rest[n..])
    };
    let value = body.get(..len)?;
    let total = (body.as_ptr() as usize - buf.as_ptr() as usize) + len;
    Some((tag, total, value, &body[len..]))
}

/// The exact bytes that get signed: the tbsCertificate TLV and its value.
fn tbs_tlv(der: &[u8]) -> Option<(&[u8], &[u8])> {
    let (tag, _, cert, _) = der_entry(der)?;
    if tag != 0x30 {
        return None;
    }
    let (tag, total, value, _) = der_entry(cert)?;
    if tag != 0x30 {
        return None;
    }
    Some((&cert[..total], value))
}

/// The CA's SubjectPublicKeyInfo TLV (what ring takes for RSA) and the raw
/// subjectPublicKey bits (what ring takes for ECDSA, the unused-bit prefix
/// dropped).
fn spki_tlv(der: &[u8]) -> Option<(&[u8], &[u8])> {
    let (_, tbs) = tbs_tlv(der)?;
    let mut rest = tbs;
    let (tag, _, _, tail) = der_entry(rest)?;
    if tag == 0xa0 {
        rest = tail; // [0] EXPLICIT version
    }
    // serialNumber, signature, issuer, validity, subject
    for expect in [0x02u8, 0x30, 0x30, 0x30, 0x30] {
        let (tag, _, _, tail) = der_entry(rest)?;
        if tag != expect {
            return None;
        }
        rest = tail;
    }
    let (tag, total, spki, _) = der_entry(rest)?;
    if tag != 0x30 {
        return None;
    }
    let (tag, _, _, tail) = der_entry(spki)?; // AlgorithmIdentifier
    if tag != 0x30 {
        return None;
    }
    let (tag, _, bits, _) = der_entry(tail)?; // subjectPublicKey BIT STRING
    if tag != 0x03 {
        return None;
    }
    Some((&rest[..total], bits.get(1..).unwrap_or(bits)))
}

fn make_dir(dir: &Path) -> Result<(), BoxedError> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn write_key(path: &Path, contents: &[u8]) -> Result<(), BoxedError> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(contents)?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, contents)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_complete_pairs_only() {
        let dir = std::env::temp_dir().join(format!("aka-certs-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.docker.crt"), "x").unwrap();
        std::fs::write(dir.join("a.docker.key"), "x").unwrap();
        std::fs::write(dir.join("b.docker.crt"), "x").unwrap();

        let pairs = find_cert_pairs(&dir.to_string_lossy());

        assert_eq!(pairs.keys().collect::<Vec<_>>(), vec!["a.docker"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_dir_is_empty() {
        assert!(find_cert_pairs("").is_empty());
        assert!(find_cert_pairs("/nonexistent-aka-dir").is_empty());
    }

    // ------------------------------------------------------ cert-gen tests

    use super::*;
    use x509_parser::prelude::{FromDer as _, X509Certificate};

    /// Temp dir removed on drop — leaked fixtures survive neither pass nor fail.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("aka-certgen-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn read_pem(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn ca_is_created_once_and_never_overwritten() {
        let tmp = TempDir::new("ca-once");
        let dir = tmp.path().join("ca");
        let first = ensure_ca(&dir, KeyAlg::Ecdsa).unwrap();
        assert!(first.created);
        assert_eq!(first.fingerprint.split(':').count(), 32);
        assert!(
            first
                .fingerprint
                .chars()
                .all(|c| c == ':' || c.is_ascii_hexdigit())
        );
        let before = std::fs::read(&first.cert_path).unwrap();

        let second = ensure_ca(&dir, KeyAlg::Ecdsa).unwrap();
        assert!(!second.created);
        assert_eq!(second.fingerprint, first.fingerprint);
        assert_eq!(std::fs::read(&second.cert_path).unwrap(), before);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let key_mode = std::fs::metadata(&first.key_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            let dir_mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(key_mode, 0o600, "ca.key readable by its owner only");
            assert_eq!(dir_mode, 0o700, "ca dir private");
        }
    }

    #[test]
    fn ca_carries_the_extensions_browsers_require() {
        let tmp = TempDir::new("ca-ext");
        let ca = ensure_ca(&tmp.path().join("ca"), KeyAlg::Ecdsa).unwrap();
        let der = pem_der(&read_pem(&ca.cert_path)).unwrap();
        let (_, cert) = X509Certificate::from_der(&der).unwrap();

        assert_eq!(
            issuer_common_name(&read_pem(&ca.cert_path))
                .unwrap()
                .as_deref(),
            Some(CA_COMMON_NAME)
        );
        assert!(
            cert.basic_constraints().unwrap().unwrap().value.ca,
            "BasicConstraints CA:TRUE"
        );
        assert!(
            cert.key_usage().unwrap().unwrap().value.key_cert_sign(),
            "keyCertSign"
        );
        let v = cert.validity();
        assert_eq!(
            v.not_after.timestamp() - v.not_before.timestamp(),
            CA_VALIDITY_DAYS * 86400
        );
    }

    #[test]
    fn leaf_has_san_eku_validity_and_chains() {
        let tmp = TempDir::new("leaf");
        let ca = ensure_ca(&tmp.path().join("ca"), KeyAlg::Ecdsa).unwrap();
        let certs = tmp.path().join("certs");
        let (crt, key) = generate_cert(&ca, "api.docker", KeyAlg::Ecdsa, &certs).unwrap();

        let der = pem_der(&read_pem(&crt)).unwrap();
        let (_, leaf) = X509Certificate::from_der(&der).unwrap();
        let san = leaf.subject_alternative_name().unwrap().unwrap();
        assert!(
            san.value
                .general_names
                .iter()
                .any(|g| matches!(g, x509_parser::extensions::GeneralName::DNSName(s) if *s == "api.docker")),
            "SAN DNSName=api.docker"
        );
        assert!(
            leaf.extended_key_usage().unwrap().unwrap().value.server_auth,
            "EKU serverAuth"
        );
        let v = leaf.validity();
        assert_eq!(
            v.not_after.timestamp() - v.not_before.timestamp(),
            LEAF_VALIDITY_DAYS * 86400
        );
        assert!(
            is_aka_issued(&read_pem(&ca.cert_path), &read_pem(&crt)).unwrap(),
            "leaf chains to the CA"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&key).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(&certs).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn rsa_leaf_chains_under_the_ecdsa_ca() {
        let tmp = TempDir::new("leaf-rsa");
        let ca = ensure_ca(&tmp.path().join("ca"), KeyAlg::Ecdsa).unwrap();
        let (crt, _) =
            generate_cert(&ca, "old.docker", KeyAlg::Rsa, &tmp.path().join("certs")).unwrap();

        let der = pem_der(&read_pem(&crt)).unwrap();
        let (_, leaf) = X509Certificate::from_der(&der).unwrap();
        assert_eq!(
            leaf.signature_algorithm.algorithm.to_string(),
            SIG_ECDSA_SHA256,
            "the issuer's algorithm signs, not the leaf key's"
        );
        assert!(is_aka_issued(&read_pem(&ca.cert_path), &read_pem(&crt)).unwrap());
    }

    #[test]
    fn rsa_ca_generates_reloads_and_issues() {
        let tmp = TempDir::new("ca-rsa");
        let dir = tmp.path().join("ca");
        let first = ensure_ca(&dir, KeyAlg::Rsa).unwrap();
        assert!(first.created);

        let loaded = ensure_ca(&dir, KeyAlg::Rsa).unwrap();
        assert!(!loaded.created);
        assert_eq!(loaded.fingerprint, first.fingerprint);

        let (crt, _) =
            generate_cert(&loaded, "rsa.docker", KeyAlg::Rsa, &tmp.path().join("certs")).unwrap();
        let der = pem_der(&read_pem(&crt)).unwrap();
        let (_, leaf) = X509Certificate::from_der(&der).unwrap();
        assert_eq!(
            leaf.signature_algorithm.algorithm.to_string(),
            SIG_RSA_SHA256
        );
        assert!(is_aka_issued(&read_pem(&loaded.cert_path), &read_pem(&crt)).unwrap());
    }

    #[test]
    fn forged_issuer_cn_does_not_pass_the_provenance_check() {
        let tmp = TempDir::new("forged");
        let honest = ensure_ca(&tmp.path().join("ca1"), KeyAlg::Ecdsa).unwrap();
        // A different CA, the same CN, signing its own leaf.
        let rogue = ensure_ca(&tmp.path().join("ca2"), KeyAlg::Ecdsa).unwrap();
        let (rogue_leaf, _) =
            generate_cert(&rogue, "api.docker", KeyAlg::Ecdsa, &tmp.path().join("certs")).unwrap();

        assert_eq!(
            issuer_common_name(&read_pem(&rogue_leaf)).unwrap().as_deref(),
            Some(CA_COMMON_NAME),
            "the name alone matches — which is why it proves nothing"
        );
        assert!(
            !is_aka_issued(&read_pem(&honest.cert_path), &read_pem(&rogue_leaf)).unwrap(),
            "the honest CA's key rejects it"
        );
    }

    #[test]
    fn host_validation_accepts_literals_rejects_everything_else() {
        for ok in ["api.docker", "a", "a.b.c", "x-1.docker", "Host1.Example"] {
            validate_host(ok).unwrap_or_else(|err| panic!("{ok}: {err}"));
        }
        for bad in [
            "",
            "*.docker",
            "bad host",
            "-bad",
            "bad-",
            "a..b",
            "a.b.",
            "under_score.docker",
            &format!("{}x", "y".repeat(63)),
        ] {
            assert!(validate_host(bad).is_err(), "{bad:?} must be rejected");
        }
    }
}
