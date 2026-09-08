//! Narrow safe wrapper around the Win32 power-broadcast window API.
//!
//! A hidden top-level window is used because message-only windows do not
//! receive system broadcasts. The window procedure never retains OS-owned
//! pointers and invokes only a caller-provided, path-free lifecycle callback.

#![cfg(windows)]
#![deny(unsafe_op_in_unsafe_fn)]
#![warn(clippy::all, clippy::pedantic)]

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

use windows::{
    Win32::{
        Foundation::{HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
        System::{
            LibraryLoader::GetModuleHandleW,
            Power::{RegisterSuspendResumeNotification, UnregisterSuspendResumeNotification},
            Threading::GetCurrentThreadId,
        },
        UI::WindowsAndMessaging::{
            CREATESTRUCTW, CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW,
            DestroyWindow, DispatchMessageW, GWLP_USERDATA, GetMessageW, MSG,
            PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMECRITICAL, PBT_APMRESUMESTANDBY,
            PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PostMessageW, PostQuitMessage,
            PostThreadMessageW, RegisterClassW, SetWindowLongPtrW, TranslateMessage,
            UnregisterClassW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLOSE, WM_DESTROY, WM_NCCREATE,
            WM_NCDESTROY, WM_POWERBROADCAST, WM_QUIT, WNDCLASSW,
        },
    },
    core::PCWSTR,
};

static WINDOW_CLASS_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Power lifecycle notifications relevant to suspend safety.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerEvent {
    /// Windows is about to suspend the session.
    Suspend,
    /// Windows reports that the session resumed.
    Resume,
}

/// Stable, redacted observer startup failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PowerObserverError {
    /// The observer thread could not be created.
    ThreadUnavailable,
    /// Win32 could not register the private window class.
    ClassUnavailable,
    /// Win32 could not create the hidden broadcast window.
    WindowUnavailable,
    /// Win32 rejected explicit suspend/resume notification registration.
    NotificationUnavailable,
    /// The observer thread terminated before reporting readiness.
    StartupInterrupted,
}

impl std::fmt::Display for PowerObserverError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::ThreadUnavailable => "power observer thread unavailable",
            Self::ClassUnavailable => "power observer class unavailable",
            Self::WindowUnavailable => "power observer window unavailable",
            Self::NotificationUnavailable => "power observer notification unavailable",
            Self::StartupInterrupted => "power observer startup interrupted",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for PowerObserverError {}

type Handler = dyn Fn(PowerEvent) + Send + Sync + 'static;

struct WindowContext {
    handler: Arc<Handler>,
}

/// Owns the dedicated hidden-window thread that receives power broadcasts.
pub struct PowerObserver {
    window: usize,
    thread_id: u32,
    thread: Option<JoinHandle<()>>,
}

#[derive(Clone, Copy)]
struct WindowIdentity {
    window: usize,
    thread_id: u32,
}

impl PowerObserver {
    /// Starts the observer and waits until the hidden window can receive
    /// broadcasts. The handler runs on the observer thread and may block for
    /// the operating system's bounded pre-suspend cleanup window.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when the thread, window class, or window cannot
    /// be created.
    pub fn start(
        handler: impl Fn(PowerEvent) + Send + Sync + 'static,
    ) -> Result<Self, PowerObserverError> {
        let handler: Arc<Handler> = Arc::new(handler);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("cyberkindred-power-observer".to_owned())
            .spawn(move || run_message_window(handler, &ready_sender))
            .map_err(|_| PowerObserverError::ThreadUnavailable)?;
        let identity = match ready_receiver.recv() {
            Ok(Ok(identity)) => identity,
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(error);
            }
            Err(_) => {
                let _ = thread.join();
                return Err(PowerObserverError::StartupInterrupted);
            }
        };
        Ok(Self {
            window: identity.window,
            thread_id: identity.thread_id,
            thread: Some(thread),
        })
    }

    #[cfg(test)]
    fn post_test_broadcast(&self, power_event: u32) -> Result<(), PowerObserverError> {
        // SAFETY: `self.window` is owned by the live observer thread and the
        // message contains only documented integral WM_POWERBROADCAST values.
        unsafe {
            PostMessageW(
                Some(HWND(self.window as *mut core::ffi::c_void)),
                WM_POWERBROADCAST,
                WPARAM(power_event as usize),
                LPARAM(0),
            )
        }
        .map_err(|_| PowerObserverError::WindowUnavailable)
    }
}

impl Drop for PowerObserver {
    fn drop(&mut self) {
        // SAFETY: the handle came from the observer thread's successful
        // `CreateWindowExW`; posting WM_CLOSE transfers teardown to its owner.
        let window_posted = unsafe {
            PostMessageW(
                Some(HWND(self.window as *mut core::ffi::c_void)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        }
        .is_ok();
        let quit_posted = if window_posted {
            false
        } else {
            // SAFETY: the identifier was captured on the observer thread after
            // its message queue and window were created. WM_QUIT is a teardown
            // fallback when the HWND has already become unavailable.
            unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) }.is_ok()
        };
        if let Some(thread) = self.thread.take()
            && (window_posted || quit_posted || thread.is_finished())
        {
            let _ = thread.join();
        }
    }
}

fn run_message_window(
    handler: Arc<Handler>,
    ready_sender: &mpsc::SyncSender<Result<WindowIdentity, PowerObserverError>>,
) {
    let result = create_and_run_message_window(handler, ready_sender);
    if let Err(error) = result {
        let _ = ready_sender.try_send(Err(error));
    }
}

fn create_and_run_message_window(
    handler: Arc<Handler>,
    ready_sender: &mpsc::SyncSender<Result<WindowIdentity, PowerObserverError>>,
) -> Result<(), PowerObserverError> {
    let class_sequence = WINDOW_CLASS_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let class_name = format!(
        "CyberKindredPowerObserver-{}-{class_sequence}",
        std::process::id()
    )
    .encode_utf16()
    .chain(std::iter::once(0))
    .collect::<Vec<_>>();
    // SAFETY: a null module name requests the current process module; the
    // returned handle is copied into the window-class and create structures.
    let module =
        unsafe { GetModuleHandleW(None) }.map_err(|_| PowerObserverError::ClassUnavailable)?;
    let instance = HINSTANCE(module.0);
    let window_class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    // SAFETY: `window_class` and its NUL-terminated class name remain alive
    // until the class is unregistered below.
    if unsafe { RegisterClassW(&raw const window_class) } == 0 {
        return Err(PowerObserverError::ClassUnavailable);
    }

    let context = Box::new(WindowContext { handler });
    let context_pointer = Box::into_raw(context);
    // SAFETY: all string pointers are valid and NUL-terminated for the call;
    // `context_pointer` is installed as GWLP_USERDATA during WM_NCCREATE and
    // reclaimed after the message loop exits.
    let window = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR::null(),
            WINDOW_STYLE::default(),
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            Some(context_pointer.cast_const().cast()),
        )
    };
    let Ok(window) = window else {
        // SAFETY: CreateWindowExW failed, so no window owns this allocation.
        drop(unsafe { Box::from_raw(context_pointer) });
        // SAFETY: the class was registered by this thread and has no window.
        let _ = unsafe { UnregisterClassW(PCWSTR(class_name.as_ptr()), Some(instance)) };
        return Err(PowerObserverError::WindowUnavailable);
    };
    // Desktop Activity Moderator / Modern Standby can omit broadcasts to
    // desktop windows that did not explicitly register for this notification.
    // SAFETY: `window` is a live HWND owned by this thread and the flag declares
    // the recipient as a window handle.
    let notification =
        unsafe { RegisterSuspendResumeNotification(HANDLE(window.0), DEVICE_NOTIFY_WINDOW_HANDLE) };
    let Ok(notification) = notification else {
        // SAFETY: the window belongs to this thread and may be destroyed here.
        let destroyed = destroy_window_if_live(window);
        if destroyed {
            // SAFETY: WM_NCDESTROY has cleared the context pointer.
            drop(unsafe { Box::from_raw(context_pointer) });
            // SAFETY: the private class has no live window after destruction.
            let _ = unsafe { UnregisterClassW(PCWSTR(class_name.as_ptr()), Some(instance)) };
        }
        return Err(PowerObserverError::NotificationUnavailable);
    };
    // SAFETY: this call only returns the current owner thread's numeric ID.
    let thread_id = unsafe { GetCurrentThreadId() };
    if ready_sender
        .send(Ok(WindowIdentity {
            window: window.0 as usize,
            thread_id,
        }))
        .is_ok()
    {
        let mut message = MSG::default();
        loop {
            // SAFETY: `message` is a valid output buffer and this thread owns
            // the window/message queue being pumped.
            let status = unsafe { GetMessageW(&raw mut message, None, 0, 0) }.0;
            if status == -1 {
                break;
            }
            if status == 0 {
                break;
            }
            // SAFETY: `message` was initialized by a successful GetMessageW.
            unsafe {
                let _ = TranslateMessage(&raw const message);
                DispatchMessageW(&raw const message);
            }
        }
    }
    let context_reclaimable = destroy_window_if_live(window);
    // SAFETY: registration belongs to this thread and is no longer needed once
    // the message loop has ended.
    let _ = unsafe { UnregisterSuspendResumeNotification(notification) };
    if context_reclaimable {
        // SAFETY: WM_NCDESTROY has cleared GWLP_USERDATA and no later callback can
        // access the context. This thread created and uniquely owns the allocation.
        drop(unsafe { Box::from_raw(context_pointer) });
    }
    // SAFETY: the message loop has ended and the private class has no live window.
    let _ = unsafe { UnregisterClassW(PCWSTR(class_name.as_ptr()), Some(instance)) };
    Ok(())
}

fn destroy_window_if_live(window: HWND) -> bool {
    // SAFETY: this private HWND is used only on its owner thread. A nonzero
    // GWLP_USERDATA value means the context is still installed on a live
    // window and must be cleared through WM_NCDESTROY before reclamation.
    let context = unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(window, GWLP_USERDATA)
    };
    context == 0 || unsafe { DestroyWindow(window) }.is_ok()
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        let create = lparam.0 as *const CREATESTRUCTW;
        if !create.is_null() {
            // SAFETY: Windows supplies a valid CREATESTRUCTW for WM_NCCREATE.
            let context = unsafe { (*create).lpCreateParams }.cast::<WindowContext>();
            // SAFETY: the pointer was allocated by this crate and remains live
            // through the message loop.
            unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, context as isize) };
            return LRESULT(1);
        }
    }

    if message == WM_POWERBROADCAST
        && let Ok(value) = u32::try_from(wparam.0)
        && let Some(event) = classify_power_broadcast(value)
    {
        // SAFETY: this crate writes only a live WindowContext pointer to this
        // private window's GWLP_USERDATA and clears it during WM_NCDESTROY.
        let context = unsafe {
            windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(window, GWLP_USERDATA)
        } as *const WindowContext;
        if !context.is_null() {
            // SAFETY: the pointer's lifetime is bounded by the message loop.
            let handler = unsafe { &(*context).handler };
            let _ = catch_unwind(AssertUnwindSafe(|| handler(event)));
        }
        return LRESULT(1);
    }

    match message {
        WM_CLOSE => {
            // SAFETY: the observer thread owns the window.
            let _ = unsafe { DestroyWindow(window) };
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: ends only this observer thread's message loop.
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_NCDESTROY => {
            // SAFETY: clearing the private pointer prevents later use.
            unsafe { SetWindowLongPtrW(window, GWLP_USERDATA, 0) };
            // SAFETY: unhandled teardown semantics belong to DefWindowProcW.
            unsafe { DefWindowProcW(window, message, wparam, lparam) }
        }
        // SAFETY: all unhandled messages retain standard Win32 behavior.
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

const fn classify_power_broadcast(value: u32) -> Option<PowerEvent> {
    match value {
        PBT_APMSUSPEND => Some(PowerEvent::Suspend),
        PBT_APMRESUMEAUTOMATIC
        | PBT_APMRESUMECRITICAL
        | PBT_APMRESUMESTANDBY
        | PBT_APMRESUMESUSPEND => Some(PowerEvent::Resume),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn classifies_only_suspend_and_resume_power_transitions() {
        assert_eq!(
            classify_power_broadcast(PBT_APMSUSPEND),
            Some(PowerEvent::Suspend)
        );
        assert_eq!(
            classify_power_broadcast(PBT_APMRESUMEAUTOMATIC),
            Some(PowerEvent::Resume)
        );
        assert_eq!(classify_power_broadcast(0), None);
    }

    #[test]
    fn hidden_window_routes_real_power_broadcast_messages_in_order() {
        let (sender, receiver) = mpsc::channel();
        let observer = PowerObserver::start(move |event| {
            let _ = sender.send(event);
        })
        .expect("observer");
        observer
            .post_test_broadcast(PBT_APMSUSPEND)
            .expect("suspend broadcast");
        observer
            .post_test_broadcast(PBT_APMRESUMEAUTOMATIC)
            .expect("resume broadcast");
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(PowerEvent::Suspend)
        );
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(PowerEvent::Resume)
        );
    }
}
