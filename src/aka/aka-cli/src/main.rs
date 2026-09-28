mod commands;

use clap::{Parser, Subcommand};

/// aka — your development proxy for docker (spiritual successor to dory).
///
/// DNS + an angie proxy make containers reachable by hostname:
/// set VIRTUAL_HOST (and VIRTUAL_PORT) on a container, run `aka up`,
/// then browse http://that-host.<tld>. Raw TCP routes (`stream`) work too.
#[derive(Parser, Debug)]
#[command(name = "aka", version, about, long_about = None)]
struct Cli {
    /// Verbose output (debug-level logging)
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Path to a config file (default: ./.aka.toml upward, then ~/.aka.toml)
    #[arg(short = 'c', long, global = true)]
    config: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Bring up aka services (dnsmasq, angie proxy, host resolver, proxyd)
    Up,
    /// Stop aka services (and remove the host resolver entries)
    Down,
    /// Stop and bring up aka services again
    Restart,
    /// Report the status of the aka services and discovered routes
    Status,
    /// Follow the logs of the dns and/or proxy container
    Logs {
        /// Only this service: dns | proxy
        service: Option<String>,
    },
    /// Attach to a service container's output (ctrl-c / ctrl-p ctrl-q to exit)
    Attach {
        /// dns | proxy (default: proxy)
        service: Option<String>,
    },
    /// Print the IP of a aka service container (default: proxy)
    Ip {
        /// dns | proxy (default: proxy)
        service: Option<String>,
    },
    /// Pull the managed docker images
    Pull,
    /// Show the routes discovered from running containers
    Routes {
        /// Print raw state JSON
        #[arg(long)]
        json: bool,
    },
    /// Write the default config file (~/.aka.toml)
    ConfigFile {
        /// Overwrite an existing file
        #[arg(short, long)]
        force: bool,
        /// Merge newly introduced keys into the existing file
        #[arg(short, long)]
        upgrade: bool,
    },
    /// Install the sudoers rule that makes `aka up`/`aka down` sudo-free
    /// (a one-time sudo; the rule is scoped to aka's own `_privileged` verb)
    InstallSudoRule,
    /// Report the installed version
    Version,
    /// Internal proxy daemon (spawned by `aka up`)
    #[command(hide = true, name = "_proxyd")]
    Proxyd,
    /// Internal root-side resolver operations (invoked via sudo)
    #[command(hide = true, name = "_privileged")]
    Privileged {
        #[command(subcommand)]
        cmd: PrivilegedCmd,
    },
}

#[derive(Subcommand, Debug)]
enum PrivilegedCmd {
    /// Write resolver entries for the given domains
    Write {
        #[arg(long)]
        domain: Vec<String>,
        #[arg(long)]
        nameserver: String,
        #[arg(long)]
        port: u16,
    },
    /// Remove aka's resolver entries
    Clean {
        #[arg(long)]
        domain: Vec<String>,
    },
    /// Succeed doing nothing (rule probe)
    Noop,
}

#[tokio::main]
async fn main() -> Result<(), aka_kernel::BoxedError> {
    let cli = Cli::parse();

    let environment = std::env::var("ENVIRONMENT").unwrap_or_else(|_| "local".into());
    let tracing_level = std::env::var("RUST_LOG").unwrap_or_else(|_| {
        if cli.verbose {
            "debug".into()
        } else {
            "info".into()
        }
    });
    let subscriber = telemetry::get_subscriber(
        "aka-cli",
        env!("CARGO_PKG_VERSION"),
        &environment,
        &tracing_level,
        std::io::stdout,
    );
    telemetry::init_subscriber(subscriber);

    let daemon = matches!(cli.command, Commands::Proxyd);
    let privileged = matches!(cli.command, Commands::Privileged { .. });

    let mut provider = provider::Provider::new();
    provider.store(aka_services::AkaPaths::resolve()?);
    if !privileged {
        // the root-side verb takes everything from argv; it must not read
        // user config or talk to docker
        provider.store(commands::resolve_config(&cli.config, daemon)?);
        provider.store(aka_docker::AkaDocker::connect()?);
        provider.store(commands::ConfigSelection {
            explicit: cli.config.clone(),
        });
    }

    match cli.command {
        Commands::Up => commands::up(&provider).await,
        Commands::Down => commands::down(&provider).await,
        Commands::Restart => {
            commands::down(&provider).await?;
            commands::up(&provider).await
        }
        Commands::Status => commands::status(&provider).await,
        Commands::Logs { service } => commands::logs(&provider, service.as_deref()).await,
        Commands::Attach { service } => commands::attach(&provider, service.as_deref()),
        Commands::Ip { service } => commands::ip(&provider, service.as_deref()).await,
        Commands::Pull => commands::pull(&provider).await,
        Commands::Routes { json } => commands::routes(&provider, json),
        Commands::ConfigFile { force, upgrade } => commands::config_file(&provider, force, upgrade),
        Commands::InstallSudoRule => commands::install_sudo_rule(),
        Commands::Version => commands::version(),
        Commands::Proxyd => commands::proxyd(&provider).await,
        Commands::Privileged { cmd } => commands::privileged(cmd),
    }
}
