// Trassenger Daemon - Background polling service with system tray
//
// Architecture:
//   main thread: tray icon + event loop (required by macOS)
//   tokio thread: background polling every 60s

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::{
    TrayIconBuilder,
    menu::{Menu, MenuItem, PredefinedMenuItem, CheckMenuItem},
    Icon,
};

mod polling;

/// Shared state between polling thread and main thread
#[derive(Default)]
struct DaemonState {
    unread_count: usize,
    tui_running: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Handle --toggle-autostart flag
    if args.contains(&"--toggle-autostart".to_string()) {
        toggle_autostart();
        return;
    }

    // Single instance guard
    if is_already_running() {
        eprintln!("Trassenger daemon is already running.");
        return;
    }

    write_pid_file();

    // Clean up PID file on SIGTERM (e.g. system shutdown or kill)
    #[cfg(unix)]
    {
        unsafe {
            libc::signal(libc::SIGTERM, handle_sigterm as libc::sighandler_t);
        }
    }

    // Shared daemon state (unread count, tui running flag)
    let state = Arc::new(Mutex::new(DaemonState::default()));

    // Channel from polling thread to main thread
    let (tx, rx) = std::sync::mpsc::channel::<polling::DaemonEvent>();

    // Spawn tokio polling thread
    let state_clone = state.clone();
    let tx_clone = tx.clone();
    std::thread::spawn(move || {
        polling::run_polling(state_clone, tx_clone);
    });

    // Build tray menu
    let open_item = MenuItem::new("Open Trassenger", true, None);
    let separator = PredefinedMenuItem::separator();
    let autostart_item = CheckMenuItem::new(
        "Start at Login",
        true,
        is_autostart_enabled(),
        None,
    );
    let quit_item = MenuItem::new("Quit", true, None);

    let tray_menu = Menu::new();
    let _ = tray_menu.append(&open_item);
    let _ = tray_menu.append(&separator);
    let _ = tray_menu.append(&autostart_item);
    let _ = tray_menu.append(&quit_item);

    // Load tray icons
    let icon_normal = load_icon(include_bytes!("../assets/tray-normal.png"));
    let icon_unread = load_icon(include_bytes!("../assets/tray-unread.png"));

    // Create tray icon
    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_tooltip("Trassenger")
        .with_icon(icon_normal.clone())
        .build()
        .expect("Failed to create tray icon");

    let open_id = open_item.id().clone();
    let autostart_id = autostart_item.id().clone();
    let quit_id = quit_item.id().clone();

    // tao event loop — required on macOS to pump NSApplication run loop so
    // the tray icon actually appears in the menu bar.
    let event_loop = EventLoopBuilder::new().build();

    // The tray icon must be created AFTER the event loop on macOS.
    // Move creation inside so it's owned by the closure.
    let _tray_icon = tray_icon; // keep alive

    event_loop.run(move |_event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(
            std::time::Instant::now() + Duration::from_millis(50),
        );

        // Process polling events from background thread
        while let Ok(event) = rx.try_recv() {
            match event {
                polling::DaemonEvent::UnreadCount(count) => {
                    if let Ok(mut s) = state.lock() {
                        s.unread_count = count;
                    }
                    if count > 0 {
                        let _ = _tray_icon.set_icon(Some(icon_unread.clone()));
                        let _ = _tray_icon.set_tooltip(Some(format!("Trassenger ({} unread)", count)));
                    } else {
                        let _ = _tray_icon.set_icon(Some(icon_normal.clone()));
                        let _ = _tray_icon.set_tooltip(Some("Trassenger".to_string()));
                    }
                }
                polling::DaemonEvent::TuiOpened => {
                    if let Ok(mut s) = state.lock() {
                        s.unread_count = 0;
                        s.tui_running = true;
                    }
                    let _ = _tray_icon.set_icon(Some(icon_normal.clone()));
                    let _ = _tray_icon.set_tooltip(Some("Trassenger".to_string()));
                }
                polling::DaemonEvent::TuiClosed => {
                    if let Ok(mut s) = state.lock() {
                        s.tui_running = false;
                    }
                }
            }
        }

        // Process tray menu events
        if let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            if event.id == quit_id {
                remove_pid_file();
                *control_flow = ControlFlow::Exit;
            } else if event.id == open_id {
                launch_tui();
            } else if event.id == autostart_id {
                toggle_autostart();
                let enabled = is_autostart_enabled();
                let _ = autostart_item.set_checked(enabled);
            }
        }
    });
}

// ── Icon loading ──────────────────────────────────────────────────────────────

fn load_icon(png_bytes: &[u8]) -> Icon {
    let img = image_from_png(png_bytes);
    Icon::from_rgba(img.data, img.width, img.height)
        .expect("Failed to create icon from PNG")
}

struct RgbaImage {
    data: Vec<u8>,
    width: u32,
    height: u32,
}

fn image_from_png(bytes: &[u8]) -> RgbaImage {
    // Minimal PNG decoder using the `png` feature; we use a hand-rolled approach
    // by depending on the `image` crate via tray-icon's dependencies.
    // Actually we'll use a simple raw decode using the png crate indirectly.
    // tray-icon uses image crate, so we replicate what it needs: raw RGBA bytes.
    // We use a lightweight inline PNG decode here.
    decode_png_to_rgba(bytes)
}

fn decode_png_to_rgba(png_bytes: &[u8]) -> RgbaImage {
    // Use minipng / hand-decode. Since we generated the PNGs ourselves with
    // known format (RGB, 22x22), we decode them manually.
    // PNG structure: sig(8) + IHDR(25) + IDAT(var) + IEND(12)
    // We'll use a simple approach: find IHDR for dimensions, then decompress IDAT.

    use std::io::Read;

    // Skip PNG signature (8 bytes)
    let mut pos = 8usize;

    let mut width = 0u32;
    let mut height = 0u32;
    let mut idat_data = Vec::new();

    while pos + 12 <= png_bytes.len() {
        let chunk_len = u32::from_be_bytes(png_bytes[pos..pos+4].try_into().unwrap()) as usize;
        let chunk_type = &png_bytes[pos+4..pos+8];
        let chunk_data = &png_bytes[pos+8..pos+8+chunk_len];
        pos += 12 + chunk_len;

        match chunk_type {
            b"IHDR" => {
                width = u32::from_be_bytes(chunk_data[0..4].try_into().unwrap());
                height = u32::from_be_bytes(chunk_data[4..8].try_into().unwrap());
            }
            b"IDAT" => {
                idat_data.extend_from_slice(chunk_data);
            }
            b"IEND" => break,
            _ => {}
        }
    }

    // Decompress IDAT
    let mut decoder = flate2::read::ZlibDecoder::new(&idat_data[..]);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).expect("Failed to decompress PNG");

    // Convert filtered RGB scanlines to RGBA
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    let stride = 1 + width as usize * 3; // filter byte + RGB pixels
    for y in 0..height as usize {
        let row = &raw[y * stride + 1..(y + 1) * stride]; // skip filter byte
        for x in 0..width as usize {
            rgba.push(row[x * 3]);     // R
            rgba.push(row[x * 3 + 1]); // G
            rgba.push(row[x * 3 + 2]); // B
            rgba.push(255);             // A
        }
    }

    RgbaImage { data: rgba, width, height }
}

// ── Terminal launch ───────────────────────────────────────────────────────────

fn tui_path() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    let dir = exe.parent().unwrap_or(std::path::Path::new("."));
    let tui = dir.join("trassenger-tui");
    tui.to_string_lossy().to_string()
}

fn launch_tui() {
    let tui = tui_path();

    #[cfg(target_os = "macos")]
    launch_tui_macos(&tui);

    #[cfg(target_os = "windows")]
    {
        // Try Windows Terminal first, fall back to cmd /c start (opens default terminal)
        if Command::new("wt.exe")
            .args(["--title", "Trassenger", "--", &tui])
            .spawn()
            .is_err()
        {
            let _ = Command::new("cmd.exe")
                .args(["/c", "start", "Trassenger", &tui])
                .spawn();
        }
    }

    #[cfg(target_os = "linux")]
    {
        for term in &["x-terminal-emulator", "gnome-terminal", "xterm"] {
            if Command::new(term).args(["-e", &tui]).spawn().is_ok() {
                break;
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn app_installed(name: &str) -> bool {
    Command::new("osascript")
        .args(["-e", &format!("exists application \"{}\"", name)])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn launch_tui_macos(tui: &str) {
    // Warp: open with --args (it accepts a command to run)
    if app_installed("Warp") {
        let _ = Command::new("open").args(["-a", "Warp", "--args", tui]).spawn();
        return;
    }
    // iTerm2: AppleScript
    if app_installed("iTerm2") {
        let script = format!(
            "tell application \"iTerm2\" to create window with default profile command \"{}\"",
            tui
        );
        let _ = Command::new("osascript").args(["-e", &script]).spawn();
        return;
    }
    // Alacritty: -e flag
    if app_installed("Alacritty") {
        let _ = Command::new("open").args(["-a", "Alacritty", "--args", "-e", tui]).spawn();
        return;
    }
    // kitty: command line
    if let Ok(kitty) = which_app("kitty") {
        let _ = Command::new(kitty).args([tui]).spawn();
        return;
    }
    // Terminal.app fallback
    let script = format!("tell application \"Terminal\" to do script \"{}\"", tui);
    let _ = Command::new("osascript").args(["-e", &script]).spawn();
}

#[cfg(target_os = "macos")]
fn which_app(name: &str) -> Result<String, ()> {
    Command::new("which")
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .ok_or(())
}

// ── Autostart ─────────────────────────────────────────────────────────────────

fn make_auto_launch() -> auto_launch::AutoLaunch {
    let exe = std::env::current_exe()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    auto_launch::AutoLaunchBuilder::new()
        .set_app_name("Trassenger Daemon")
        .set_app_path(&exe)
        .set_args(&["--daemon"])
        .build()
        .expect("Failed to create AutoLaunch")
}

fn is_autostart_enabled() -> bool {
    make_auto_launch().is_enabled().unwrap_or(false)
}

fn toggle_autostart() {
    let al = make_auto_launch();
    if al.is_enabled().unwrap_or(false) {
        let _ = al.disable();
    } else {
        let _ = al.enable();
    }
}

// ── Single instance guard ─────────────────────────────────────────────────────

fn pid_file_path() -> PathBuf {
    trassenger_lib::storage::get_app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join("daemon.pid")
}

fn is_already_running() -> bool {
    let path = pid_file_path();
    if !path.exists() {
        return false;
    }
    let contents = std::fs::read_to_string(&path).unwrap_or_default();
    let pid: u32 = contents.trim().parse().unwrap_or(0);
    if pid == 0 {
        return false;
    }
    // Check if process is alive
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }
    #[cfg(windows)]
    {
        // Use tasklist to check if PID is alive (no extra crate needed)
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
    #[cfg(not(any(unix, windows)))]
    false
}

fn write_pid_file() {
    let path = pid_file_path();
    let pid = std::process::id();
    let _ = std::fs::write(path, pid.to_string());
}

fn remove_pid_file() {
    let _ = std::fs::remove_file(pid_file_path());
}

#[cfg(unix)]
extern "C" fn handle_sigterm(_: libc::c_int) {
    remove_pid_file();
    std::process::exit(0);
}
