//! Thin aka-specific wrapper over the docker engine API (bollard, unix socket).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use aka_kernel::BoxedError;
use aka_kernel::container::{ContainerSummary, NetworkEndpoint};
use bollard::Docker;
use bollard::container::LogOutput;
use bollard::errors::Error as DockerError;
use bollard::exec::{StartExecOptions, StartExecResults};
use bollard::models::{
    ContainerCreateBody, EventMessage, ExecConfig, HostConfig, NetworkConnectRequest,
    NetworkCreateRequest, PortBinding, PortMap, RestartPolicy, RestartPolicyNameEnum,
};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, EventsOptionsBuilder,
    ListContainersOptionsBuilder, LogsOptionsBuilder, RemoveContainerOptionsBuilder,
    StopContainerOptionsBuilder, TagImageOptionsBuilder,
};

/// Re-export so dependents cannot drift from the engine API version.
pub use bollard;
use futures::{Stream, StreamExt};
use tracing::{debug, warn};

/// Which host stream a log line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub stream: LogStream,
    pub message: String,
}

pub struct AkaDocker {
    docker: Docker,
}

impl AkaDocker {
    /// Connect over the docker unix socket (`/var/run/docker.sock` or
    /// `$DOCKER_HOST` for non-unix transports is not supported; aka only
    /// supports a local daemon).
    pub fn connect() -> Result<Self, BoxedError> {
        Ok(Self {
            docker: Docker::connect_with_unix_defaults()?,
        })
    }

    pub async fn ping(&self) -> Result<(), BoxedError> {
        self.docker.ping().await?;
        Ok(())
    }

    pub async fn daemon_version(&self) -> Result<String, BoxedError> {
        let version = self.docker.version().await?;
        Ok(version.version.unwrap_or_else(|| "unknown".into()))
    }

    // --------------------------------------------------------------- listings

    /// All containers (running and stopped), enriched via inspect. `preferred`
    /// network sorts each container's endpoint list.
    pub async fn list_summaries(
        &self,
        preferred: &str,
    ) -> Result<Vec<ContainerSummary>, BoxedError> {
        let listed = self
            .docker
            .list_containers(Some(
                ListContainersOptionsBuilder::default().all(true).build(),
            ))
            .await?;

        let mut summaries = Vec::with_capacity(listed.len());
        for listed in listed {
            let Some(id) = listed.id.clone() else {
                continue;
            };
            match self.summary_of(&id, preferred).await {
                Ok(summary) => summaries.push(summary),
                Err(err) => {
                    // Container vanished between list and inspect; skip it.
                    debug!("skipping container {id}: {err}");
                }
            }
        }

        summaries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(summaries)
    }

    pub async fn summary(
        &self,
        name: &str,
        preferred: &str,
    ) -> Result<ContainerSummary, BoxedError> {
        self.summary_of(name, preferred).await
    }

    pub async fn running(&self, name: &str) -> Result<bool, BoxedError> {
        match self.summary_opt(name, "bridge").await {
            Ok(Some(s)) => Ok(s.state == "running"),
            Ok(None) => Ok(false),
            Err(err) => Err(err),
        }
    }

    pub async fn exists(&self, name: &str) -> Result<bool, BoxedError> {
        Ok(self.summary_opt(name, "bridge").await?.is_some())
    }

    /// The container's primary IP, preferring `preferred`.
    pub async fn ip_of(&self, name: &str, preferred: &str) -> Result<Option<String>, BoxedError> {
        Ok(self
            .summary_opt(name, preferred)
            .await?
            .and_then(|s| s.backend_ip().map(str::to_owned)))
    }

    /// Running containers whose HOST-published (`public_port`) socket claims
    /// `port` (Docker Desktop forwards published ports through a host socket
    /// owned by com.docker.backend, so a host lsof alone hides the holder.
    /// Container-internal ports do not conflict and must not match).
    pub async fn containers_publishing(&self, port: u16) -> Result<Vec<String>, BoxedError> {
        let listed = self
            .docker
            .list_containers(Some(ListContainersOptionsBuilder::default().build()))
            .await?;

        let mut names = Vec::new();
        for item in listed {
            let published = item
                .ports
                .unwrap_or_default()
                .iter()
                .any(|p| p.public_port == Some(port));
            if published {
                let name = item
                    .names
                    .and_then(|ns| {
                        ns.into_iter()
                            .next()
                            .map(|n| n.trim_start_matches('/').to_owned())
                    })
                    .or(item.id);
                if let Some(name) = name {
                    names.push(name);
                }
            }
        }
        names.sort();
        Ok(names)
    }

    pub async fn labels_of(&self, name: &str) -> Result<BTreeMap<String, String>, BoxedError> {
        let inspect = self.docker.inspect_container(name, None).await?;
        Ok(inspect
            .config
            .and_then(|c| c.labels)
            .unwrap_or_default()
            .into_iter()
            .collect())
    }

    pub async fn summary_opt(
        &self,
        name: &str,
        preferred: &str,
    ) -> Result<Option<ContainerSummary>, BoxedError> {
        match self.summary_of(name, preferred).await {
            Ok(s) => Ok(Some(s)),
            Err(err) if is_not_found(&err) => Ok(None),
            Err(err) => Err(err),
        }
    }

    async fn summary_of(
        &self,
        id_or_name: &str,
        preferred: &str,
    ) -> Result<ContainerSummary, BoxedError> {
        let inspect = self.docker.inspect_container(id_or_name, None).await?;

        let name = inspect
            .name
            .unwrap_or_default()
            .trim_start_matches('/')
            .to_owned();

        let state = inspect
            .state
            .and_then(|s| s.status)
            .map(|status| format!("{status:?}").to_lowercase())
            .unwrap_or_default();

        let config = inspect.config.unwrap_or_default();

        let env = config
            .env
            .unwrap_or_default()
            .into_iter()
            .filter_map(|pair| {
                pair.split_once('=')
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
            })
            .collect::<BTreeMap<_, _>>();

        let labels = config
            .labels
            .unwrap_or_default()
            .into_iter()
            .collect::<BTreeMap<_, _>>();

        let mut ports = BTreeSet::new();
        for key in config.exposed_ports.unwrap_or_default() {
            if let Some(port) = port_from_key(&key) {
                ports.insert(port);
            }
        }
        if let Some(networks) = &inspect
            .network_settings
            .as_ref()
            .and_then(|n| n.ports.as_ref())
        {
            for key in networks.keys() {
                if let Some(port) = port_from_key(key) {
                    ports.insert(port);
                }
            }
        }

        let mut endpoints = inspect
            .network_settings
            .and_then(|n| n.networks)
            .unwrap_or_default()
            .into_iter()
            .map(|(network, endpoint)| NetworkEndpoint {
                network,
                ip: endpoint.ip_address.unwrap_or_default(),
            })
            .collect::<Vec<_>>();

        endpoints.sort_by(|a, b| {
            (a.network != preferred, &a.network).cmp(&(b.network != preferred, &b.network))
        });

        Ok(ContainerSummary {
            id: inspect.id.unwrap_or_default(),
            name,
            state,
            image: config.image.unwrap_or_default(),
            image_id: inspect.image,
            env,
            labels,
            ports,
            networks: endpoints,
        })
    }

    // --------------------------------------------------------------- networks

    pub async fn ensure_network(&self, name: &str) -> Result<(), BoxedError> {
        let networks = self.docker.list_networks(None).await?;
        if networks.iter().any(|n| n.name.as_deref() == Some(name)) {
            return Ok(());
        }

        self.docker
            .create_network(NetworkCreateRequest {
                name: name.to_owned(),
                driver: Some("bridge".into()),
                ..Default::default()
            })
            .await?;
        debug!("created docker network {name}");
        Ok(())
    }

    /// Attach a container to a network; already-attached is not an error.
    pub async fn connect_container(
        &self,
        network: &str,
        container: &str,
    ) -> Result<(), BoxedError> {
        match self
            .docker
            .connect_network(
                network,
                NetworkConnectRequest {
                    container: container.to_owned(),
                    endpoint_config: Option::<bollard::models::EndpointSettings>::None,
                },
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(err) if is_already_attached(&err.to_string()) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    // -------------------------------------------------------------- lifecycle

    pub async fn create_start(
        &self,
        name: &str,
        body: ContainerCreateBody,
    ) -> Result<String, BoxedError> {
        let created = self
            .docker
            .create_container(
                Some(CreateContainerOptionsBuilder::default().name(name).build()),
                body,
            )
            .await?;

        let id = created.id.clone();
        self.docker.start_container(&id, None).await?;
        debug!("created and started container {name} ({})", created.id);
        Ok(id)
    }

    pub async fn start(&self, name: &str) -> Result<(), BoxedError> {
        self.docker.start_container(name, None).await?;
        Ok(())
    }

    /// Stop and remove a container if it exists; missing is fine.
    pub async fn stop_remove(&self, name: &str) -> Result<(), BoxedError> {
        let opts = StopContainerOptionsBuilder::default().t(2).build();
        match self.docker.stop_container(name, Some(opts)).await {
            Ok(())
            | Err(DockerError::DockerResponseServerError {
                status_code: 304, ..
            }) => {}
            Err(err) if is_not_found_msg(&err.to_string()) => return Ok(()),
            Err(err) => return Err(err.into()),
        }

        let opts = RemoveContainerOptionsBuilder::default().force(true).build();
        match self.docker.remove_container(name, Some(opts)).await {
            Ok(()) => {}
            Err(err) if is_not_found_msg(&err.to_string()) => {}
            Err(err) => return Err(err.into()),
        }
        Ok(())
    }

    pub async fn stop(&self, name: &str) -> Result<(), BoxedError> {
        let opts = StopContainerOptionsBuilder::default().t(2).build();
        match self.docker.stop_container(name, Some(opts)).await {
            Ok(())
            | Err(DockerError::DockerResponseServerError {
                status_code: 304, ..
            }) => Ok(()),
            Err(err) if is_not_found_msg(&err.to_string()) => Ok(()),
            Err(err) => Err(err.into()),
        }
    }

    // ------------------------------------------------------------------- exec

    /// Run a command inside a container, detached-wait style; returns
    /// `(success, combined output)`.
    pub async fn exec(&self, container: &str, cmd: &[&str]) -> Result<(bool, String), BoxedError> {
        let exec = self
            .docker
            .create_exec(
                container,
                ExecConfig {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    tty: Some(false),
                    cmd: Some(cmd.iter().map(|s| (*s).to_owned()).collect()),
                    ..Default::default()
                },
            )
            .await?;

        let exec_id = exec.id;
        let output = match self
            .docker
            .start_exec(
                &exec_id,
                Some(StartExecOptions {
                    detach: false,
                    tty: false,
                    output_capacity: None,
                }),
            )
            .await?
        {
            StartExecResults::Attached { mut output, .. } => {
                let mut buf = String::new();
                while let Some(line) = output.next().await {
                    if let Ok(line) = line {
                        buf.push_str(&line.to_string());
                    }
                }
                buf
            }
            StartExecResults::Detached => String::new(),
        };

        let inspect = self.docker.inspect_exec(&exec_id).await?;
        let ok = inspect.running == Some(false) && inspect.exit_code.unwrap_or(1) == 0;
        Ok((ok, output))
    }

    // ------------------------------------------------------------------ logs

    pub fn logs(
        &self,
        container: &str,
        follow: bool,
    ) -> impl Stream<Item = LogLine> + Send + 'static {
        self.docker
            .logs(
                container,
                Some(
                    LogsOptionsBuilder::default()
                        .follow(follow)
                        .stdout(true)
                        .stderr(true)
                        .tail("all")
                        .build(),
                ),
            )
            .filter_map(|item| async move {
                match item.ok()? {
                    LogOutput::StdOut { message } => Some(LogLine {
                        stream: LogStream::Stdout,
                        message: String::from_utf8_lossy(&message).into_owned(),
                    }),
                    LogOutput::StdErr { message } => Some(LogLine {
                        stream: LogStream::Stderr,
                        message: String::from_utf8_lossy(&message).into_owned(),
                    }),
                    LogOutput::StdIn { .. } | LogOutput::Console { .. } => None,
                }
            })
    }

    // ------------------------------------------------------------------ pull

    /// Pull an image, streaming human-readable progress lines.
    pub fn pull(
        &self,
        image: &str,
    ) -> impl Stream<Item = Result<String, BoxedError>> + Send + 'static {
        let (name, tag) = split_image(image);
        self.docker
            .create_image(
                Some(
                    CreateImageOptionsBuilder::default()
                        .from_image(&name)
                        .tag(&tag)
                        .build(),
                ),
                None,
                None,
            )
            .map(move |item| match item {
                Ok(info) => {
                    let status = info.status.unwrap_or_default();
                    let id = info.id.unwrap_or_default();
                    let error = info
                        .error_detail
                        .and_then(|d| d.message)
                        .map(|e| format!("error: {e}"));
                    Ok(match error {
                        Some(error) => error,
                        None if id.is_empty() => status,
                        None => format!("{status} {id}"),
                    })
                }
                Err(err) => Err(err.into()),
            })
    }

    /// The image id a reference (`name:tag`, `name@digest`) resolves to on
    /// this machine, or `None` when the engine has no such image.
    pub async fn image_id(&self, image: &str) -> Option<String> {
        match self.docker.inspect_image(image).await {
            Ok(inspect) => inspect.id,
            Err(err) if is_not_found_msg(&err.to_string()) => None,
            Err(err) => {
                warn!("image inspect for {image} failed: {err}");
                None
            }
        }
    }

    pub async fn image_exists(&self, image: &str) -> bool {
        self.image_id(image).await.is_some()
    }

    /// Point `dst` (a `name:tag` reference) at the same image as `src`.
    pub async fn tag(&self, src: &str, dst: &str) -> Result<(), BoxedError> {
        let (repo, tag) = split_image(dst);
        self.docker
            .tag_image(src, Some(TagImageOptionsBuilder::default().repo(&repo).tag(&tag).build()))
            .await?;
        Ok(())
    }

    // ---------------------------------------------------------------- events

    /// Container events from the engine (type=container, all actions).
    pub fn events(&self) -> impl Stream<Item = EventMessage> + Send + 'static {
        let filters: HashMap<String, Vec<String>> =
            [("type".to_owned(), vec!["container".to_owned()])]
                .into_iter()
                .collect();

        self.docker
            .events(Some(
                EventsOptionsBuilder::default().filters(&filters).build(),
            ))
            .filter_map(|item| async move { item.ok() })
    }

    // ------------------------------------------------------------ host specs

    /// Hostconfig for managed containers: port bindings on `bind_ip`, a
    /// restart policy, optional binds/capabilities.
    pub fn host_config(
        bind_ip: &str,
        ports: &[(u16, u16)], // (host, container) tcp/udp published as-is via keys below
        binds: &[String],
        cap_add: &[String],
        restart: &str,
    ) -> HostConfig {
        let mut port_bindings = PortMap::new();
        for (host, container) in ports {
            port_bindings.insert(
                format!("{container}/tcp"),
                Some(vec![PortBinding {
                    host_ip: Some(bind_ip.to_owned()),
                    host_port: Some(host.to_string()),
                }]),
            );
        }

        let name = match restart {
            "always" => RestartPolicyNameEnum::ALWAYS,
            "on-failure" => RestartPolicyNameEnum::ON_FAILURE,
            "no" => RestartPolicyNameEnum::NO,
            _ => RestartPolicyNameEnum::UNLESS_STOPPED,
        };

        HostConfig {
            port_bindings: Some(port_bindings),
            binds: (!binds.is_empty()).then(|| binds.to_vec()),
            cap_add: (!cap_add.is_empty()).then(|| cap_add.to_vec()),
            restart_policy: Some(RestartPolicy {
                name: Some(name),
                maximum_retry_count: None,
            }),
            ..Default::default()
        }
    }

    /// Expose ports (`"80/tcp"`) matching published ports, as the API requires.
    pub fn exposed_ports(ports: &[u16]) -> Option<Vec<String>> {
        (!ports.is_empty()).then(|| ports.iter().map(|p| format!("{p}/tcp")).collect())
    }
}

/// `(name, tag)` for a docker image reference, defaulting to `latest`.
pub fn split_image(image: &str) -> (String, String) {
    // Guard registry ports (`registry:5000/x:tag`): only the last path
    // segment may carry the tag.
    let last = image.rsplit('/').next().unwrap_or(image);
    match last.rsplit_once(':') {
        Some((name_part, tag)) if !tag.contains('/') => (
            image[..image.len() - last.len()].to_owned() + name_part,
            tag.to_owned(),
        ),
        _ => (image.to_owned(), "latest".to_owned()),
    }
}

fn port_from_key(key: &str) -> Option<u16> {
    key.split('/').next()?.parse().ok()
}

fn is_not_found(err: &BoxedError) -> bool {
    is_not_found_msg(&err.to_string())
}

fn is_not_found_msg(msg: &str) -> bool {
    msg.contains("404") || msg.contains("no such container") || msg.contains("No such container")
}

fn is_already_attached(msg: &str) -> bool {
    msg.contains("already exists") || msg.contains("already attached")
}
