use clap::Parser;
use webserver::{app, cli, observability};

#[cfg(windows)]
use webserver::windows_service_runtime;

#[cfg(not(windows))]
#[tokio::main]
async fn main() {
    observability::logging::init();

    if let Err(error) = app::run(cli::Cli::parse()).await {
        tracing::error!(%error, "webserver stopped with an error");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn main() {
    if windows_service_runtime::try_run_service() {
        return;
    }

    observability::logging::init();
    let runtime = tokio::runtime::Runtime::new().expect("create Tokio runtime");
    if let Err(error) = runtime.block_on(app::run(cli::Cli::parse())) {
        tracing::error!(%error, "webserver stopped with an error");
        std::process::exit(1);
    }
}
