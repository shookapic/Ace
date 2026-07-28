use tauri::{command, WebviewWindow};

/// Sets the true OS-level window alpha (not CSS opacity), so the whole
/// composited window becomes see-through rather than just its content fading.
#[command]
pub fn set_window_opacity(window: WebviewWindow, alpha: f64) -> Result<(), String> {
    let alpha = alpha.clamp(0.0, 1.0);

    #[cfg(target_os = "windows")]
    {
        windows_impl::set_opacity(&window, alpha).map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        macos_impl::set_opacity(&window, alpha).map_err(|e| e.to_string())?;
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let _ = (&window, alpha);
        return Err("window opacity is not yet implemented on this platform".into());
    }

    Ok(())
}

/// Excludes (or re-includes) the window from screenshots, screen shares, and
/// screen recordings, while it remains fully visible/interactive locally.
#[command]
pub fn set_capture_hidden(window: WebviewWindow, hidden: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::set_capture_hidden(&window, hidden).map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        macos_impl::set_capture_hidden(&window, hidden).map_err(|e| e.to_string())?;
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let _ = (&window, hidden);
        return Err(
            "capture-hiding has no OS-level primitive on this platform; use the panic-hide shortcut instead"
                .into(),
        );
    }

    Ok(())
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use tauri::WebviewWindow;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetLayeredWindowAttributes, SetWindowDisplayAffinity,
        SetWindowLongPtrW, GWL_EXSTYLE, LWA_ALPHA, WDA_EXCLUDEFROMCAPTURE, WDA_NONE,
        WS_EX_LAYERED,
    };

    fn hwnd(window: &WebviewWindow) -> Result<HWND, String> {
        window
            .hwnd()
            .map(|h| HWND(h.0))
            .map_err(|e| e.to_string())
    }

    pub fn set_opacity(window: &WebviewWindow, alpha: f64) -> Result<(), String> {
        let hwnd = hwnd(window)?;
        unsafe {
            let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | (WS_EX_LAYERED.0 as isize));
            let byte = (alpha * 255.0).round() as u8;
            SetLayeredWindowAttributes(hwnd, windows::Win32::Foundation::COLORREF(0), byte, LWA_ALPHA)
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn set_capture_hidden(window: &WebviewWindow, hidden: bool) -> Result<(), String> {
        let hwnd = hwnd(window)?;
        let affinity = if hidden {
            WDA_EXCLUDEFROMCAPTURE
        } else {
            WDA_NONE
        };
        unsafe {
            SetWindowDisplayAffinity(hwnd, affinity).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod macos_impl {
    use objc::runtime::Object;
    use objc::{msg_send, sel, sel_impl};
    use tauri::WebviewWindow;

    // NSWindowSharingType values: excluding a window from capture uses `None`.
    const NS_WINDOW_SHARING_NONE: u64 = 0;
    const NS_WINDOW_SHARING_READ_ONLY: u64 = 1;

    fn ns_window(window: &WebviewWindow) -> Result<*mut Object, String> {
        let ptr = window.ns_window().map_err(|e| e.to_string())? as *mut Object;
        if ptr.is_null() {
            return Err("no NSWindow handle".into());
        }
        Ok(ptr)
    }

    pub fn set_opacity(window: &WebviewWindow, alpha: f64) -> Result<(), String> {
        let ns_window = ns_window(window)?;
        unsafe {
            // A window must be non-opaque for alpha < 1 to composite through.
            let _: () = msg_send![ns_window, setOpaque: (alpha >= 1.0)];
            let _: () = msg_send![ns_window, setAlphaValue: alpha];
        }
        Ok(())
    }

    pub fn set_capture_hidden(window: &WebviewWindow, hidden: bool) -> Result<(), String> {
        let ns_window = ns_window(window)?;
        // sharingType = .none makes AppKit exclude the window from screenshots,
        // screen recordings, and screen sharing — the macOS analogue of Windows'
        // WDA_EXCLUDEFROMCAPTURE.
        let sharing_type: u64 = if hidden {
            NS_WINDOW_SHARING_NONE
        } else {
            NS_WINDOW_SHARING_READ_ONLY
        };
        unsafe {
            let _: () = msg_send![ns_window, setSharingType: sharing_type];
        }
        Ok(())
    }
}
