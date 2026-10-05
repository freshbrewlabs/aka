use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// What aka cares about on a docker container, gathered from list+inspect.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContainerSummary {
    pub id: String,
    /// Container name without the leading `/`.
    pub name: String,
    /// Docker state: `running`, `exited`, ...
    pub state: String,
    pub image: String,
    /// The image id (`sha256:…`) the container was created from — the content
    /// it actually runs. `image` is only the reference named at create time,
    /// so this is what says whether a retagged `name:latest` has landed.
    pub image_id: Option<String>,
    /// Environment as key/value (entries without `=` are dropped).
    pub env: BTreeMap<String, String>,
    pub labels: BTreeMap<String, String>,
    /// Ports the container declares/exposes (parsed from `3000/tcp` keys).
    pub ports: BTreeSet<u16>,
    /// Network endpoints, ordered: configured network first, then the rest
    /// alphabetically.
    pub networks: Vec<NetworkEndpoint>,
}

impl ContainerSummary {
    /// First non-empty network endpoint, i.e. the address reachable from
    /// containers attached to the same networks.
    pub fn backend_ip(&self) -> Option<&str> {
        self.networks
            .iter()
            .map(|e| e.ip.as_str())
            .find(|ip| !ip.is_empty())
    }

    pub fn all_networks(&self) -> impl Iterator<Item = &str> {
        self.networks.iter().map(|e| e.network.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkEndpoint {
    pub network: String,
    pub ip: String,
}
