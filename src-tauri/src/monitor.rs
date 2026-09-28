use std::sync::mpsc::{self, Receiver, Sender};
use windows::core::{w, GUID};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    RegisterPowerSettingNotification, UnregisterPowerSettingNotification, HPOWERNOTIFY,
    POWERBROADCAST_SETTING,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, PostQuitMessage, RegisterClassW, SetWindowLongPtrW, CREATESTRUCTW,
    GWLP_USERDATA, MSG, REGISTER_NOTIFICATION_FLAGS, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CREATE,
    WM_DESTROY, WM_POWERBROADCAST, WNDCLASSW,
};

const GUID_ACDC_POWER_SOURCE: GUID = GUID::from_u128(0x5d3e9a59_e9d5_4b00_a6bd_ff34ff516548);
const PBT_POWERSETTINGCHANGE: u32 = 0x8013;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerEvent {
    AcConnected,
    BatteryActive,
    ShortTermPower,
}

/// RAII guard for the power notification registration and hidden window.
///
/// Ensures `UnregisterPowerSettingNotification` and `DestroyWindow` are called
/// when the message loop exits, and the `Box<Sender>` is reclaimed.
struct MonitorGuard {
    hwnd: Option<HWND>,
    hnotify: Option<HPOWERNOTIFY>,
    sender_ptr: *mut Sender<PowerEvent>,
}

impl Drop for MonitorGuard {
    fn drop(&mut self) {
        if let Some(hnotify) = self.hnotify {
            // SAFETY: `hnotify` was obtained from a successful
            // `RegisterPowerSettingNotification` call in the same thread.
            unsafe {
                let _ = UnregisterPowerSettingNotification(hnotify);
            }
        }
        if let Some(hwnd) = self.hwnd {
            // SAFETY: `hwnd` was created by `CreateWindowExW` in this thread.
            // `DestroyWindow` must be called from the creating thread, which
            // is guaranteed because `MonitorGuard` is not `Send`.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
        // SAFETY: `sender_ptr` was created via `Box::into_raw` during setup
        // and has not been freed elsewhere. Reclaiming it here prevents a leak.
        if !self.sender_ptr.is_null() {
            unsafe {
                let _ = Box::from_raw(self.sender_ptr);
            }
        }
    }
}

/// Starts the event-driven power source monitor on a dedicated thread.
///
/// Returns a `Receiver` that emits `PowerEvent` on AC/battery state changes.
/// The monitor uses a hidden Win32 message-only window instead of polling.
pub fn start_power_event_monitor() -> Receiver<PowerEvent> {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        if let Err(e) = run_monitor_loop(tx) {
            eprintln!("Power monitor failed: {e}");
        }
    });

    rx
}

fn run_monitor_loop(tx: Sender<PowerEvent>) -> Result<(), String> {
    // SAFETY: `GetModuleHandleW(None)` returns the handle of the current
    // executable. It is always valid for the process lifetime.
    let hinstance =
        unsafe { GetModuleHandleW(None) }.map_err(|e| format!("GetModuleHandleW: {e}"))?;

    let class_name = w!("PowerMonitorHiddenClass");

    let wnd_class = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance.into(),
        lpszClassName: class_name,
        ..Default::default()
    };

    // SAFETY: `RegisterClassW` registers a window class for this process.
    // The `WNDCLASSW` struct is stack-allocated and valid for this call.
    let atom = unsafe { RegisterClassW(&wnd_class) };
    if atom == 0 {
        return Err("RegisterClassW failed".into());
    }

    let sender_ptr = Box::into_raw(Box::new(tx));
    let mut guard = MonitorGuard {
        hwnd: None,
        hnotify: None,
        sender_ptr,
    };

    // SAFETY: `CreateWindowExW` creates a message-only hidden window.
    // `sender_ptr` is passed via `lpCreateParams` and stored in GWLP_USERDATA
    // during WM_CREATE. The pointer remains valid until `MonitorGuard::drop`
    // reclaims it via `Box::from_raw`.
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("PowerMonitorHiddenWindow"),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance.into()),
            Some(sender_ptr as *const std::ffi::c_void),
        )
    }
    .map_err(|e| format!("CreateWindowExW: {e}"))?;

    guard.hwnd = Some(hwnd);

    // SAFETY: `RegisterPowerSettingNotification` registers for AC/DC change
    // notifications delivered as WM_POWERBROADCAST to `hwnd`. The GUID is a
    // static constant. Flag 0 = DEVICE_NOTIFY_WINDOW_HANDLE.
    let hnotify = unsafe {
        RegisterPowerSettingNotification(
            HANDLE(hwnd.0),
            &GUID_ACDC_POWER_SOURCE,
            REGISTER_NOTIFICATION_FLAGS(0),
        )
    }
    .map_err(|e| format!("RegisterPowerSettingNotification: {e}"))?;

    guard.hnotify = Some(hnotify);

    // SAFETY: Standard Win32 message pump. `GetMessageW` blocks until a
    // message is available or WM_QUIT is received. The MSG struct is
    // stack-allocated and valid for the loop duration.
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).into() {
            DispatchMessageW(&msg);
        }
    }

    Ok(())
}

/// Window procedure for the hidden power-monitor window.
///
/// # Safety
///
/// Called by Windows as a callback. `lparam` during `WM_CREATE` points to a
/// `CREATESTRUCTW` whose `lpCreateParams` holds a `*mut Sender<PowerEvent>`.
/// During `WM_POWERBROADCAST`, `lparam` points to `POWERBROADCAST_SETTING`.
/// Both are guaranteed valid by the Win32 contract for these messages.
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            // SAFETY: During WM_CREATE, `lparam` points to a valid
            // `CREATESTRUCTW` allocated by the system. We read
            // `lpCreateParams` (our `sender_ptr`) and store it in
            // GWLP_USERDATA for later retrieval.
            let cs = lparam.0 as *const CREATESTRUCTW;
            if !cs.is_null() {
                let sender_ptr = (*cs).lpCreateParams as isize;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, sender_ptr);
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            if wparam.0 as u32 == PBT_POWERSETTINGCHANGE {
                // SAFETY: When wparam is PBT_POWERSETTINGCHANGE, lparam
                // points to a valid POWERBROADCAST_SETTING struct allocated
                // by the system. We only read from it.
                let setting = lparam.0 as *const POWERBROADCAST_SETTING;
                if !setting.is_null() {
                    let s = &*setting;
                    if s.PowerSetting == GUID_ACDC_POWER_SOURCE && s.DataLength >= 4 {
                        // SAFETY: DataLength >= 4 guarantees at least one
                        // u32 is present in the Data field.
                        let value = *(s.Data.as_ptr() as *const u32);
                        let event = match value {
                            0 => PowerEvent::AcConnected,
                            1 => PowerEvent::BatteryActive,
                            2 => PowerEvent::ShortTermPower,
                            _ => return LRESULT(1),
                        };
                        // SAFETY: GWLP_USERDATA holds a *const Sender set
                        // during WM_CREATE. The pointer remains valid
                        // because MonitorGuard owns it and outlives the
                        // message loop.
                        let sender_ptr =
                            GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Sender<PowerEvent>;
                        if !sender_ptr.is_null() {
                            let _ = (*sender_ptr).send(event);
                        }
                    }
                }
            }
            LRESULT(1) // WM_POWERBROADCAST requires returning TRUE
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
