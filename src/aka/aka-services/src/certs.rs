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
}
