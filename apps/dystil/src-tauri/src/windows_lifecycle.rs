//! Windows process-lifecycle integration used by the Microsoft Store build.
//!
//! Store servicing closes a packaged app before replacing its files. Registering
//! here lets Windows relaunch Dystil after that servicing has completed.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;
use tauri::Manager;
use tracing::{error, info, warn};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Recovery::{
    RegisterApplicationRestart, RESTART_NO_CRASH, RESTART_NO_HANG, RESTART_NO_REBOOT,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass, SUBCLASSPROC,
};
use windows::Win32::UI::WindowsAndMessaging::{
    ENDSESSION_CLOSEAPP, WM_CLOSE, WM_ENDSESSION, WM_NCDESTROY, WM_QUERYENDSESSION,
};

use crate::recording::{stop_engine, RecordingState};
use crate::tray::QUIT_REQUESTED;

const SERVICING_SUBCLASS_ID: usize = 0x4459_5354; // "DYST"
const SERVICING_CLEANUP_BUDGET: Duration = Duration::from_millis(4_500);

static EXTERNAL_SERVICING_PENDING: AtomicBool = AtomicBool::new(false);
static EXTERNAL_SERVICING_SHUTDOWN_STARTED: AtomicBool = AtomicBool::new(false);

/// Register the primary Dystil process for restart after MSIX package servicing.
///
/// `RESTART_NO_PATCH` is intentionally omitted: a Store/MSIX update is the one
/// shutdown reason for which we want Windows to relaunch Dystil.
pub fn register_store_update_restart() {
    let flags = RESTART_NO_CRASH | RESTART_NO_HANG | RESTART_NO_REBOOT;
    match unsafe { RegisterApplicationRestart(w!("--restarted-after-store-update"), flags) } {
        Ok(()) => info!("registered Dystil for restart after Store package updates"),
        Err(error) => warn!(%error, "failed to register Dystil for Store update restart"),
    }
}

/// Install a native message hook on Dystil's persistent Home window.
///
/// Tauri's `CloseRequested` event does not expose the `ENDSESSION_CLOSEAPP`
/// reason used by Restart Manager during package servicing, so this hook must
/// sit below Tauri at the Win32 window-procedure layer.
pub fn install_store_servicing_hook(window: &tauri::WebviewWindow) -> Result<(), String> {
    let hwnd = crate::windows_overlay::get_hwnd(window)
        .ok_or_else(|| "failed to resolve the Home window HWND".to_string())?;

    // The subclass owns this boxed handle until WM_NCDESTROY. The callback
    // clones it before handing work to another thread, so no borrowed state
    // escapes the window procedure.
    let app = Box::new(window.app_handle().clone());
    let app_ptr = Box::into_raw(app);
    let installed = unsafe {
        SetWindowSubclass(
            hwnd,
            Some(store_servicing_subclass_proc),
            SERVICING_SUBCLASS_ID,
            app_ptr as usize,
        )
    };
    if !installed.as_bool() {
        unsafe {
            drop(Box::from_raw(app_ptr));
        }
        return Err("SetWindowSubclass failed for the Home window".to_string());
    }

    info!("installed Windows package-servicing message hook");
    Ok(())
}

fn is_package_servicing(lparam: LPARAM) -> bool {
    (lparam.0 as u32 & ENDSESSION_CLOSEAPP) != 0
}

fn perform_external_servicing_shutdown(app: &tauri::AppHandle) {
    if EXTERNAL_SERVICING_SHUTDOWN_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }

    info!("starting fast shutdown for external Windows package servicing");
    QUIT_REQUESTED.store(true, Ordering::SeqCst);

    let shutdown_app = app.clone();
    let (completed_tx, completed_rx) = mpsc::sync_channel(1);
    let spawn_result = std::thread::Builder::new()
        .name("store-servicing-shutdown".to_string())
        .spawn(move || {
            let result = tauri::async_runtime::block_on(async move {
                stop_engine(shutdown_app.state::<RecordingState>(), shutdown_app.clone()).await
            });
            let _ = completed_tx.send(result);
        });

    if let Err(error) = spawn_result {
        error!(%error, "failed to start external servicing cleanup thread");
        return;
    }

    match completed_rx.recv_timeout(SERVICING_CLEANUP_BUDGET) {
        Ok(Ok(())) => {
            info!("external package-servicing cleanup completed; exiting Dystil");
            app.exit(0);
        }
        Ok(Err(error)) => {
            error!(%error, "external package-servicing cleanup failed");
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            warn!(
                "external package-servicing cleanup exceeded 4.5 seconds; allowing Windows to continue"
            );
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            error!("external package-servicing cleanup thread disconnected");
        }
    }
}

/// Win32 window subclass callback. No panic may cross this FFI boundary.
unsafe extern "system" fn store_servicing_subclass_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _subclass_id: usize,
    app_refdata: usize,
) -> LRESULT {
    let handled = catch_unwind(AssertUnwindSafe(|| {
        // Own a clone for the duration of this callback. `app.exit()` may
        // synchronously cause WM_NCDESTROY and release the boxed refdata.
        let app = unsafe { (&*(app_refdata as *const tauri::AppHandle)).clone() };
        match message {
            WM_QUERYENDSESSION if is_package_servicing(lparam) => {
                info!("WM_QUERYENDSESSION: external package servicing requested");
                EXTERNAL_SERVICING_PENDING.store(true, Ordering::SeqCst);
                register_store_update_restart();
                Some(LRESULT(1))
            }
            WM_ENDSESSION if is_package_servicing(lparam) => {
                if wparam.0 != 0 {
                    info!("WM_ENDSESSION: external package servicing confirmed");
                    perform_external_servicing_shutdown(&app);
                } else {
                    info!("WM_ENDSESSION: external package servicing cancelled");
                    EXTERNAL_SERVICING_PENDING.store(false, Ordering::SeqCst);
                }
                None
            }
            WM_CLOSE if EXTERNAL_SERVICING_PENDING.load(Ordering::SeqCst) => {
                // Restart Manager may send WM_CLOSE if the application has not
                // exited after WM_ENDSESSION. Preserve the servicing reason so
                // Tauri does not convert this close into a tray minimize.
                info!("WM_CLOSE received during external package servicing");
                perform_external_servicing_shutdown(&app);
                None
            }
            WM_NCDESTROY => {
                let _ = unsafe {
                    RemoveWindowSubclass(
                        hwnd,
                        Some(store_servicing_subclass_proc),
                        SERVICING_SUBCLASS_ID,
                    )
                };
                unsafe {
                    drop(Box::from_raw(app_refdata as *mut tauri::AppHandle));
                }
                None
            }
            _ => None,
        }
    }));

    match handled {
        Ok(Some(result)) => result,
        Ok(None) => unsafe { DefSubclassProc(hwnd, message, wparam, lparam) },
        Err(_) => {
            // Logging here could itself panic while unwinding from the original
            // failure. Fall through to the original window procedure instead.
            unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_package_servicing_bit_in_combined_end_session_reason() {
        assert!(is_package_servicing(LPARAM(ENDSESSION_CLOSEAPP as isize)));
        assert!(is_package_servicing(LPARAM(
            (ENDSESSION_CLOSEAPP | 0x8000_0000) as isize
        )));
        assert!(!is_package_servicing(LPARAM(0)));
    }
}
