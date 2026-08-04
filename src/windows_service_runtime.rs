//! Native Windows Service Control Manager integration.

use std::{ffi::OsString, path::PathBuf, time::Duration};

use tokio::sync::watch;
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

use crate::{config::Config, observability::logging, server::listener};

const SERVICE_NAME: &str = "Webserver";
const DEFAULT_CONFIG_PATH: &str = r"C:\ProgramData\Webserver\webserver.toml";

define_windows_service!(ffi_service_main, service_main);

/// Returns `true` when the Service Control Manager started this process.
pub fn try_run_service() -> bool {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).is_ok()
}

fn service_main(arguments: Vec<OsString>) {
    logging::init();
    if let Err(error) = run_service(arguments) {
        tracing::error!(%error, "Windows service stopped with an error");
    }
}

fn run_service(arguments: Vec<OsString>) -> windows_service::Result<()> {
    let (stop_sender, stop_receiver) = watch::channel(false);
    let event_handler = move |event| match event {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_sender.send(true);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;
    status_handle.set_service_status(status(ServiceState::Running, ServiceControlAccept::STOP))?;

    let config_path = service_config_path(&arguments);
    let result = run_server(config_path, stop_receiver);

    status_handle
        .set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()))?;
    result
}

fn run_server(
    config_path: PathBuf,
    stop_receiver: watch::Receiver<bool>,
) -> windows_service::Result<()> {
    let config = Config::load(&config_path)
        .and_then(|config| {
            config.validate()?;
            Ok(config)
        })
        .map_err(service_error)?;
    let runtime = tokio::runtime::Runtime::new().map_err(windows_service::Error::Winapi)?;
    runtime
        .block_on(listener::run_service(config, stop_receiver))
        .map_err(service_error)
}

fn service_error(error: impl std::fmt::Display) -> windows_service::Error {
    windows_service::Error::Winapi(std::io::Error::other(error.to_string()))
}

fn service_config_path(arguments: &[OsString]) -> PathBuf {
    arguments
        .windows(2)
        .find(|pair| pair[0] == "--config")
        .map(|pair| PathBuf::from(&pair[1]))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH))
}

fn status(state: ServiceState, controls: ServiceControlAccept) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: controls,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    }
}
