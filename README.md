# Anti-Burn OLED Guard (`anti-burn-win`)

**Anti-Burn OLED Guard** is a native, lightweight Rust application for Windows 11 designed to prevent OLED burn-in on ultrawide and high-resolution displays (tested at `5120 × 2160` @ 120Hz+) with near-zero CPU/GPU overhead.

---

## ✨ Key Features

### 1. True Z-Order Active/Inactive Window Dimming
- **Zero-Lag Dragging:** Instead of drawing a cutout mask over the screen that lags behind moved windows, **Anti-Burn** places a native Win32 layered backdrop (`MonitorBackdrop`) directly *behind* the active foreground window in the Windows Z-order (`SetWindowPos(..., active_hwnd, ...)`). Dragging or resizing the active window is handled natively by DWM at full monitor refresh rate with zero cutout lag.
- **Zero-White-Flash Window Switching:** Uses a low-level mouse hook (`WH_MOUSE_LL`), `EVENT_SYSTEM_FOREGROUND`, and pre-allocated rounded-corner `TransitionMask` surfaces with Porter-Duff alpha compensation so clicking an inactive window covers it at full dim opacity *before* `win32k.sys` raises it, then smoothly crossfades it to full brightness at 125 FPS.
- **All-Dimmed Desktop Fallback:** If no application window is focused (e.g., clicking the desktop), all open windows and desktop icons are automatically dimmed behind the primary backdrop.

### 2. Taskbar Proximity Dimmer
- **Independent Taskbar Protection:** Dims `Shell_TrayWnd` (the Windows taskbar) when the mouse cursor is away from the bottom edge, and smoothly fades to full brightness based on cursor proximity.
- **Start Menu & Flyout Aware:** Automatically restores brightness when the Start Menu, Search, Action Center, or Calendar flyout is open, while still dimming the taskbar when the taskbar itself is clicked.

### 3. Global Idle Dimming (`MagSetFullscreenColorEffect` + Hardware Cursor Hide)
- **Hardware-Accelerated Fullscreen Dimming:** Uses the Windows Magnification API (`MagSetFullscreenColorEffect`) to apply a zero-copy GPU color matrix to the DWM desktop composition framebuffer when idle.
- **Hardware Cursor Burn-In Protection:** Because the GPU hardware cursor scanout plane bypasses `MagSetFullscreenColorEffect`, **Anti-Burn** automatically hides the hardware mouse cursor across all applications whenever Idle Dimming is active, and restores it on the very first frame of mouse movement (`>= 2px`), mouse click, or key press.
- **Anti-Jiggler & Wake Diagnostics:** Ignores synthetic `0px` `SendInput` wakes from background apps and ignores audio output so music or background videos never prevent idle dimming. Includes a live **Idle & Wakeup Diagnostics** panel and persistent `idle_wake.log`.

### 4. Coupled Flow-Field + Metaballs OLED Screensaver
- **Multi-Stage OLED Protection:**
  - **Stage 1:** Renders a smooth coupled curl-noise flow-field particle simulation with organic metaball glow over an initially semi-transparent dark backdrop (with configurable metaball visibility and solid vs. dark-center metaball modes). Particles flow freely off-screen and curve naturally back onto the canvas without piling up on screen edges.
  - **Stage 2 (100% Opaque Backdrop):** Smoothly fades the backdrop behind the particles and metaballs to `100%` opaque `#000000` black.
  - **Turn Off Monitor After (5-Min Increments):** After a configurable duration (`Never`, `5`, `10`, `15`, ..., `120 min`), fades out all particles and metaballs to a guaranteed **100% pure `#000000` black screen** (zero pixel emission, zero DWM/GDI blits, and zero CPU simulation cost) AND sends a hardware display power-off command (`WM_SYSCOMMAND` / `SC_MONITORPOWER = 2`) to put the monitor to sleep.
- **Configurable Bottom-Left Hot Corner:** Hold the mouse cursor in the bottom-left `12 × 12` pixel corner of the primary monitor for **1.0 second** to either **launch the screensaver** on demand (fading straight to a 100% opaque `#000000` black backdrop with idle dimming applied) or **immediately turn off the monitor** (`SC_MONITORPOWER = 2` hardware sleep + pure `#000000` blackout).
- **Always Hidden Cursor:** Ensures the mouse cursor is never visible over the running screensaver.

### 5. System Tray & Windows Startup Integration
- **Launch at Startup:** Automatically registers `AntiBurnOLEDGuard` in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (with `--minimized` flag) so protection starts silently in the system tray at login.
- **Neo-Brutalist Dark UI:** Custom GDI double-buffered dark interface with live sliders, toggles, preview buttons, and real-time idle diagnostics.

---

## 🛠️ Building from Source

### Prerequisites
- **Windows 10 / 11 (x64)**
- **Rust Toolchain** (`cargo`, `rustc` 1.75+)
- **Windows 10/11 SDK** (`rc.exe` is used automatically by `build.rs` if present to embed the multi-resolution 32-bit ARGB application icon into the executable)

### Build & Run
```powershell
# Run unit tests
cargo test

# Build optimized release binary
cargo build --release

# Launch Anti-Burn OLED Guard
.\target\release\anti-burn.exe
```

To start minimized directly to the system tray:
```powershell
.\target\release\anti-burn.exe --minimized
```

---

## 📁 Project Structure

| File | Description |
| :--- | :--- |
| [`src/main.rs`](src/main.rs) | Main Win32 message loop, `125 Hz` high-resolution tick, `WH_MOUSE_LL` & `EVENT_SYSTEM_FOREGROUND` hooks, hot corner, and system tray |
| [`src/window_dimmer.rs`](src/window_dimmer.rs) | Dual-backdrop Z-order active/inactive window dimmer with Porter-Duff transition compensation |
| [`src/taskbar_dimmer.rs`](src/taskbar_dimmer.rs) | Taskbar proximity dimmer (`Shell_TrayWnd` tracking and Z-order enforcement) |
| [`src/idle_controller.rs`](src/idle_controller.rs) | `MagSetFullscreenColorEffect` global idle dimmer, hardware cursor hider, and wake diagnostics |
| [`src/screensaver.rs`](src/screensaver.rs) | Coupled curl-noise flow-field + metaball screensaver engine (Stage 1 & Stage 2) |
| [`src/neo_ui.rs`](src/neo_ui.rs) | Custom double-buffered GDI dark settings UI |
| [`src/win_apis.rs`](src/win_apis.rs) | Win32 / DWM / Magnification / WASAPI / Registry startup wrappers |
| [`src/icon.rs`](src/icon.rs) | Procedural supersampled 32-bit ARGB app icon generator |
| [`src/config.rs`](src/config.rs) | Persistent JSON configuration (`settings.json`) |

---

## 📜 License

This project is licensed under the **GNU General Public License v3.0 (GPLv3)** — see the [LICENSE](LICENSE) file for details.
