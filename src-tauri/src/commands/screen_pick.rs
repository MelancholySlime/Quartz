//! Native screen-color eyedropper with a cursor-following loupe overlay.
//!
//! WebView2's built-in `EyeDropper` API is unreliable on some runtime builds
//! (it opens then instantly aborts with "user canceled"), so the picker's
//! pipette uses this native path instead.
//!
//! Flow: `screen_pick_color` spawns a full-screen transparent always-on-top
//! overlay window (`public/loupe.html`) and a blocking driver thread. The driver
//! reads a small screen region under the OS cursor with GDI each tick and emits
//! it to the overlay, which draws a magnified pixel grid + swatch + hex that
//! follows the cursor. The overlay window CAPTURES the confirming click (so it
//! never falls through to whatever is underneath) and reports it back through
//! `screen_pick_confirm`; the driver then samples the centre pixel, closes the
//! overlay, and resolves. Escape / a 30s timeout cancels.
//!
//! Windows-only; the command errors on other platforms so the UI falls back.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

/// Shared state between the overlay window (which reports the confirming click)
/// and the blocking driver loop (which samples + tears down).
struct PickSession {
    /// Set true when the overlay captured the confirming click.
    confirmed: AtomicBool,
    /// Set true to cancel (Escape / overlay closed / timeout).
    cancelled: AtomicBool,
    /// Cursor position (screen px) captured at the confirming click.
    click_x: AtomicI32,
    click_y: AtomicI32,
}

const OVERLAY_LABEL: &str = "eyedropper-loupe";

/// Reported by the loupe overlay when the user clicks to confirm the pick,
/// carrying the screen-pixel coordinate of the click.
#[tauri::command]
pub fn screen_pick_confirm(app: tauri::AppHandle, x: i32, y: i32) {
    use tauri::Manager;
    if let Some(session) = app.try_state::<Arc<PickSession>>() {
        session.click_x.store(x, Ordering::Release);
        session.click_y.store(y, Ordering::Release);
        session.confirmed.store(true, Ordering::Release);
    }
}

/// Reported by the loupe overlay when the user presses Escape.
#[tauri::command]
pub fn screen_pick_cancel(app: tauri::AppHandle) {
    use tauri::Manager;
    if let Some(session) = app.try_state::<Arc<PickSession>>() {
        session.cancelled.store(true, Ordering::Release);
    }
}

/// Sample a screen colour with the loupe overlay. Resolves to the `#rrggbb`
/// under the cursor at the confirming click, or `Ok(None)` when cancelled.
#[tauri::command]
pub async fn screen_pick_color(app: tauri::AppHandle) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        run_pick(app).await
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("screen color picking is only supported on Windows".into())
    }
}

#[cfg(windows)]
async fn run_pick(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};

    // Drop any stale session from a prior pick so `manage` never panics on a
    // double-managed type, then install a fresh one shared with the overlay's
    // confirm/cancel commands.
    app.unmanage::<Arc<PickSession>>();
    let session = Arc::new(PickSession {
        confirmed: AtomicBool::new(false),
        cancelled: AtomicBool::new(false),
        click_x: AtomicI32::new(0),
        click_y: AtomicI32::new(0),
    });
    app.manage(session.clone());

    // Full-screen transparent, always-on-top overlay covering the primary
    // monitor. NOT click-through: it captures the confirming click so it never
    // reaches the app underneath (the overlay JS reports the click back).
    let overlay = WebviewWindowBuilder::new(&app, OVERLAY_LABEL, WebviewUrl::App("loupe.html".into()))
        .transparent(true)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .resizable(false)
        .focused(true)
        .build()
        .map_err(|e| format!("create loupe overlay: {e}"))?;
    // Cover the ENTIRE virtual desktop (all monitors), so the loupe follows the
    // cursor onto any screen. Physical pixels, since the window is sized in them.
    let (vx, vy, vw, vh) = virtual_screen_rect();
    let _ = overlay.set_position(tauri::PhysicalPosition::new(vx, vy));
    let _ = overlay.set_size(tauri::PhysicalSize::new(vw, vh));
    let _ = overlay.set_focus();

    // Drive loop on a blocking worker: emit zoom frames + watch for confirm/cancel.
    let app2 = app.clone();
    let session2 = session.clone();
    let result = tokio::task::spawn_blocking(move || drive_loop(&app2, &session2))
        .await
        .map_err(|e| format!("screen pick task failed: {e}"))?;

    // Teardown: close overlay + drop the shared session state.
    if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
        let _ = w.close();
    }
    app.unmanage::<Arc<PickSession>>();

    result
}

#[cfg(windows)]
fn drive_loop(app: &tauri::AppHandle, session: &Arc<PickSession>) -> Result<Option<String>, String> {
    use std::thread::sleep;
    use std::time::{Duration, Instant};
    use tauri::Emitter;
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

    const VK_ESCAPE: i32 = 0x1B;
    const DOWN: i16 = -0x8000;
    // Odd zoom size so there is a true centre pixel.
    const ZOOM: i32 = 15;

    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        if session.cancelled.load(Ordering::Acquire) || Instant::now() > deadline {
            return Ok(None);
        }
        // Esc via the underlying key state (also handled by the overlay JS).
        if unsafe { GetAsyncKeyState(VK_ESCAPE) } & DOWN != 0 {
            return Ok(None);
        }
        if session.confirmed.load(Ordering::Acquire) {
            let x = session.click_x.load(Ordering::Acquire);
            let y = session.click_y.load(Ordering::Acquire);
            let (_, _, pixels) = capture_region(x, y, ZOOM);
            let c = center_hex(&pixels, ZOOM);
            return Ok(Some(c));
        }

        // Emit the current zoom frame.
        let mut p = POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut p) } != 0 {
            let (w, h, pixels) = capture_region(p.x, p.y, ZOOM);
            let hex = center_hex(&pixels, ZOOM);
            let _ = app.emit_to(
                OVERLAY_LABEL,
                "loupe://frame",
                serde_json::json!({ "x": p.x, "y": p.y, "w": w, "h": h, "hex": hex, "pixels": pixels }),
            );
        }
        sleep(Duration::from_millis(16));
    }
}

/// The full virtual-desktop rectangle `(x, y, width, height)` spanning every
/// monitor, in physical pixels.
#[cfg(windows)]
fn virtual_screen_rect() -> (i32, i32, u32, u32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    // SAFETY: GetSystemMetrics reads system config values only.
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1) as u32;
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1) as u32;
        (x, y, w, h)
    }
}

/// Grab an `n`×`n` screen region centred on `(cx, cy)` as flat RGB bytes
/// (row-major, length `n*n*3`). Out-of-range pixels read as black.
#[cfg(windows)]
fn capture_region(cx: i32, cy: i32, n: i32) -> (i32, i32, Vec<u8>) {
    use windows_sys::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC,
        GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        SRCCOPY,
    };

    let half = n / 2;
    let ox = cx - half;
    let oy = cy - half;
    let mut out = vec![0u8; (n * n * 3) as usize];

    // SAFETY: standard GDI capture: screen DC -> compatible mem DC + bitmap,
    // BitBlt the region, GetDIBits to read pixels, then release everything.
    unsafe {
        let screen = GetDC(std::ptr::null_mut());
        if screen.is_null() {
            return (n, n, out);
        }
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, n, n);
        let old = SelectObject(mem, bmp);
        BitBlt(mem, 0, 0, n, n, screen, ox, oy, SRCCOPY);

        // Top-down 24bpp DIB so row 0 is the top row.
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: n,
            biHeight: -n, // negative = top-down
            biPlanes: 1,
            biBitCount: 24,
            biCompression: BI_RGB as u32,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        // 24bpp rows are padded to 4-byte boundaries.
        let row_stride = (((n * 3) + 3) & !3) as usize;
        let mut raw = vec![0u8; row_stride * n as usize];
        GetDIBits(
            mem,
            bmp,
            0,
            n as u32,
            raw.as_mut_ptr() as *mut _,
            &mut bi,
            DIB_RGB_COLORS,
        );
        // DIB is BGR; repack to tight RGB.
        for row in 0..n as usize {
            for col in 0..n as usize {
                let s = row * row_stride + col * 3;
                let d = (row * n as usize + col) * 3;
                out[d] = raw[s + 2]; // R
                out[d + 1] = raw[s + 1]; // G
                out[d + 2] = raw[s]; // B
            }
        }

        SelectObject(mem, old);
        DeleteObject(bmp);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);
    }
    (n, n, out)
}

/// `#rrggbb` of the centre pixel of an `n`×`n` RGB region.
#[cfg(windows)]
fn center_hex(pixels: &[u8], n: i32) -> String {
    let half = (n / 2) as usize;
    let idx = (half * n as usize + half) * 3;
    if idx + 2 < pixels.len() {
        format!("#{:02x}{:02x}{:02x}", pixels[idx], pixels[idx + 1], pixels[idx + 2])
    } else {
        "#000000".into()
    }
}
