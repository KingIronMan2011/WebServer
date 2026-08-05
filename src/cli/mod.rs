//! Command-line interface and command dispatch.

use std::path::PathBuf;

use clap::{ArgGroup, Parser, Subcommand};
use clap_complete::Shell;

#[derive(Debug, Parser)]
#[command(
    name = "webserver",
    version,
    about = "A small Rust web server and reverse proxy"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum AdminCommand {
    /// Locally create the first management account and enable the HTTPS admin API.
    WebInit {
        #[arg(short, long, default_value = "webserver.toml")]
        config: PathBuf,
        #[arg(long)]
        host: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        username: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Bootstrap and manage the authenticated administration interface.
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
    /// Start the HTTP server.
    Run {
        #[arg(short, long, default_value = "webserver.toml")]
        config: PathBuf,
    },
    /// Parse and validate a configuration file without starting the server.
    Check {
        #[arg(short, long, default_value = "webserver.toml")]
        config: PathBuf,
    },
    /// Create a documented starter configuration.
    Init {
        #[arg(short, long, default_value = "webserver.toml")]
        config: PathBuf,
    },
    /// Add a virtual host to the configuration.
    SiteAdd {
        #[arg(short, long, default_value = "webserver.toml")]
        config: PathBuf,
        #[arg(long)]
        host: String,
    },
    /// Remove a virtual host and all of its routes.
    SiteRemove {
        #[arg(short, long, default_value = "webserver.toml")]
        config: PathBuf,
        #[arg(long)]
        host: String,
    },
    /// Add a static-file or reverse-proxy route to an existing host.
    #[command(group(ArgGroup::new("target").required(true).args(["static_root", "upstream"])))]
    RouteAdd {
        #[arg(short, long, default_value = "webserver.toml")]
        config: PathBuf,
        #[arg(long)]
        host: String,
        #[arg(long)]
        path: String,
        #[arg(long = "static")]
        static_root: Option<PathBuf>,
        #[arg(long)]
        upstream: Option<String>,
    },
    /// Remove a route from an existing host.
    RouteRemove {
        #[arg(short, long, default_value = "webserver.toml")]
        config: PathBuf,
        #[arg(long)]
        host: String,
        #[arg(long)]
        path: String,
    },
    /// Print shell completion code to standard output.
    Completion {
        #[arg(value_enum)]
        shell: Shell,
    },
}
