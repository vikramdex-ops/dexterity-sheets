//! Running as a Windows service (`DexSheetsServer service`), the way the installer sets it up:
//! started at boot by Windows, restarted if it stops, stopped cleanly by the Server Manager, the
//! Services console or an uninstall.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;

use windows_service::service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::{define_windows_service, service_dispatcher};

/// The service's name in Windows (the installer registers it under this name).
pub const NAME: &str = "DexSheetsServer";

/// What to serve, set before Windows calls [`service_main`].
static CONFIG: OnceLock<(PathBuf, String, u16)> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

/// Hands the process to the Windows service controller; returns when the service stops.
pub fn run(dir: PathBuf, name: String, port: u16) -> Result<(), String> {
    let _ = CONFIG.set((dir, name, port));
    service_dispatcher::start(NAME, ffi_service_main).map_err(|e| format!("not started by the Windows service controller: {e}"))
}

fn service_main(_args: Vec<OsString>) {
    let Some((dir, name, port)) = CONFIG.get().cloned() else { return };
    let (stop_tx, stop_rx) = mpsc::channel();
    let handler = move |event| match event {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let Ok(status) = service_control_handler::register(NAME, handler) else { return };
    let report = |state, code: u32, accept| {
        let _ = status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: if code == 0 { ServiceExitCode::Win32(0) } else { ServiceExitCode::ServiceSpecific(code) },
            checkpoint: 0,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        });
    };
    match crate::serve(dir, name, port) {
        Ok(server) => {
            report(ServiceState::Running, 0, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN);
            let _ = stop_rx.recv();
            // Every acknowledged edit is already on disk; a final snapshot makes the next start fast.
            log::info!("service stopping");
            server.flush_all();
            report(ServiceState::Stopped, 0, ServiceControlAccept::empty());
        }
        Err(e) => {
            log::error!("cannot start: {e}");
            // A non-zero code: Windows' recovery options restart the service.
            report(ServiceState::Stopped, 1, ServiceControlAccept::empty());
        }
    }
}
