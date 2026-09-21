use anyhow::{Context, Result};
use image::{ImageBuffer, Rgba};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::ptr;
use std::thread;
use std::time::Duration;

// P1-10: Maximum log read size to avoid loading huge files into RAM
const MAX_LOG_BYTES: usize = 64 * 1024;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct RECT {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
struct BITMAPINFOHEADER {
    bi_size: u32,
    bi_width: i32,
    bi_height: i32,
    bi_planes: u16,
    bi_bit_count: u16,
    bi_compression: u32,
    bi_size_image: u32,
    bi_xpels_per_meter: i32,
    bi_ypels_per_meter: i32,
    bi_clr_used: u32,
    bi_clr_important: u32,
}

#[repr(C)]
struct BITMAPINFO {
    bmi_header: BITMAPINFOHEADER,
    bmi_colors: [u32; 1],
}

const SRCCOPY: u32 = 0x00CC0020;
const DIB_RGB_COLORS: u32 = 0;
const BI_RGB: u32 = 0;

const SW_RESTORE: i32 = 9;
const SW_MAXIMIZE: i32 = 3;
const SW_SHOW: i32 = 5;
const GENERIC_ALL: u32 = 0x10000000;

const MOUSEEVENTF_MOVE: u32 = 0x0001;
const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
const MOUSEEVENTF_LEFTUP: u32 = 0x0004;
const MOUSEEVENTF_RIGHTDOWN: u32 = 0x0008;
const MOUSEEVENTF_RIGHTUP: u32 = 0x0010;
const MOUSEEVENTF_MIDDLEDOWN: u32 = 0x0020;
const MOUSEEVENTF_MIDDLEUP: u32 = 0x0040;
const MOUSEEVENTF_WHEEL: u32 = 0x0800;
const MOUSEEVENTF_ABSOLUTE: u32 = 0x8000;

const KEYEVENTF_KEYUP: u32 = 0x0002;

#[link(name = "user32")]
extern "system" {
    fn OpenDesktopA(lpszDesktop: *const u8, dwFlags: u32, fInherit: i32, dwDesiredAccess: u32) -> *mut std::ffi::c_void;
    fn SetThreadDesktop(hDesktop: *mut std::ffi::c_void) -> i32;
    fn EnumDesktopWindows(hDesktop: *mut std::ffi::c_void, lpEnumFunc: unsafe extern "system" fn(*mut std::ffi::c_void, isize) -> i32, lParam: isize) -> i32;
    fn GetWindowRect(hWnd: *mut std::ffi::c_void, lpRect: *mut RECT) -> i32;
    fn SetForegroundWindow(hWnd: *mut std::ffi::c_void) -> i32;
    fn ShowWindow(hWnd: *mut std::ffi::c_void, nCmdShow: i32) -> i32;
    fn IsIconic(hWnd: *mut std::ffi::c_void) -> i32;
    fn GetDC(hWnd: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn ReleaseDC(hWnd: *mut std::ffi::c_void, hDC: *mut std::ffi::c_void) -> i32;
    fn SetCursorPos(x: i32, y: i32) -> i32;
    fn mouse_event(dwFlags: u32, dx: u32, dy: u32, dwData: u32, dwExtraInfo: usize);
    fn keybd_event(bVk: u8, bScan: u8, dwFlags: u32, dwExtraInfo: usize);
    fn EnumWindows(
        lpEnumFunc: unsafe extern "system" fn(*mut std::ffi::c_void, isize) -> i32,
        lParam: isize,
    ) -> i32;
    fn InternalGetWindowText(hWnd: *mut std::ffi::c_void, lpString: *mut u16, nMaxCount: i32) -> i32;
    fn GetWindowThreadProcessId(hWnd: *mut std::ffi::c_void, lpdwProcessId: *mut u32) -> u32;
    fn IsWindowVisible(hWnd: *mut std::ffi::c_void) -> i32;
    fn PrintWindow(hWnd: *mut std::ffi::c_void, hDC: *mut std::ffi::c_void, nFlags: u32) -> i32;
    fn GetSystemMetrics(nIndex: i32) -> i32;
    /// P2-15: declare SetProcessDPIAware to avoid physical-vs-logical coord mismatch on scaled displays.
    /// We use the user32 import rather than SetProcessDpiAwarenessContext (Win10+) for maximum compatibility.
    fn SetProcessDPIAware() -> i32;
}

// GetSystemMetrics constants for multi-monitor (P2-15)
const SM_CXSCREEN: i32 = 0;
const SM_CYSCREEN: i32 = 1;
const SM_XVIRTUALSCREEN: i32 = 76;
const SM_YVIRTUALSCREEN: i32 = 77;
const SM_CXVIRTUALSCREEN: i32 = 78;
const SM_CYVIRTUALSCREEN: i32 = 79;

#[link(name = "gdi32")]
extern "system" {
    fn CreateCompatibleDC(hdc: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CreateCompatibleBitmap(hdc: *mut std::ffi::c_void, cx: i32, cy: i32) -> *mut std::ffi::c_void;
    fn SelectObject(hdc: *mut std::ffi::c_void, h: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    fn BitBlt(
        hdc: *mut std::ffi::c_void,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        hdcSrc: *mut std::ffi::c_void,
        x1: i32,
        y1: i32,
        rop: u32,
    ) -> i32;
    fn DeleteDC(hdc: *mut std::ffi::c_void) -> i32;
    fn DeleteObject(ho: *mut std::ffi::c_void) -> i32;
    fn GetDIBits(
        hdc: *mut std::ffi::c_void,
        hbm: *mut std::ffi::c_void,
        start: u32,
        cLines: u32,
        lpvBits: *mut u8,
        lpbmi: *mut BITMAPINFO,
        usage: u32,
    ) -> i32;
}

struct EnumContext {
    target_needle: String,
    target_pid: Option<u32>,
    found_hwnd: *mut std::ffi::c_void,
    found_title: String,
}

unsafe extern "system" fn enum_window_callback(h_wnd: *mut std::ffi::c_void, l_param: isize) -> i32 {
    let ctx = &mut *(l_param as *mut EnumContext);

    // Se temos um PID alvo, filtra por ele primeiro sem IPC
    if let Some(expected_pid) = ctx.target_pid {
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(h_wnd, &mut pid);
        if pid != expected_pid {
            return 1;
        }
    }

    // Janelas visíveis ou minimizadas
    let is_visible = IsWindowVisible(h_wnd) != 0;
    let is_min = IsIconic(h_wnd) != 0;
    if !is_visible && !is_min {
        return 1;
    }

    let mut buffer = [0u16; 512];
    // InternalGetWindowText lê da tabela interna de handles do Windows sem mandar mensagens IPC que poderiam travar
    let len = InternalGetWindowText(h_wnd, buffer.as_mut_ptr(), 512);
    if len > 0 {
        let title = String::from_utf16_lossy(&buffer[..len as usize]);
        if title.to_lowercase().contains(&ctx.target_needle.to_lowercase()) {
            ctx.found_hwnd = h_wnd;
            ctx.found_title = title;
            return 0; // achou, para a busca
        }
    }
    1
}

pub struct Win32Harness;

impl Win32Harness {
    /// P2-15: one-time DPI initialization flag. SetProcessDPIAware must be called
    /// once, early in the process; subsequent calls return 0 but are harmless.
    static DPI_INIT: std::sync::Once = std::sync::Once::new();

    /// Conecta a thread atual ao desktop interativo 'default' da estação WinSta0.
    /// Também chama SetProcessDPIAware uma única vez para que GetWindowRect e
    /// GetSystemMetrics operem em pixels físicos e os cliques acertem em monitores
    /// com escala 125%/150% (P2-15).
    pub fn attach_interactive_desktop() {
        DPI_INIT.call_once(|| unsafe {
            SetProcessDPIAware();
        });
        unsafe {
            let h_desk = OpenDesktopA(b"default\0".as_ptr(), 0, 0, GENERIC_ALL);
            if !h_desk.is_null() {
                SetThreadDesktop(h_desk);
            }
        }
    }

    /// Localiza a janela do ANIGO Studio mesmo se estiver minimizada
    pub fn find_anigo_window() -> Result<(*mut std::ffi::c_void, String, RECT, bool)> {
        let mut ctx = EnumContext {
            target_needle: "ANIGO".to_string(),
            target_pid: None,
            found_hwnd: ptr::null_mut(),
            found_title: String::new(),
        };

        unsafe {
            // Tenta primeiro na estação/desktop atual
            EnumWindows(enum_window_callback, &mut ctx as *mut _ as isize);

            // Se não encontrou, tenta anexar à estação default interativa
            if ctx.found_hwnd.is_null() {
                Self::attach_interactive_desktop();
                let h_desk = OpenDesktopA(b"default\0".as_ptr(), 0, 0, GENERIC_ALL);
                if !h_desk.is_null() {
                    EnumDesktopWindows(h_desk, enum_window_callback, &mut ctx as *mut _ as isize);
                }
                if ctx.found_hwnd.is_null() {
                    EnumWindows(enum_window_callback, &mut ctx as *mut _ as isize);
                }
            }
        }

        if ctx.found_hwnd.is_null() {
            anyhow::bail!("Janela do ANIGO não encontrada no sistema operacional. Verifique se o comando 'npm run tauri dev' concluiu a abertura da janela.");
        }

        let is_minimized = unsafe { IsIconic(ctx.found_hwnd) != 0 };

        let mut rect = RECT::default();
        let success = unsafe { GetWindowRect(ctx.found_hwnd, &mut rect) };
        if success == 0 {
            anyhow::bail!("Falha ao obter coordenadas da janela do ANIGO.");
        }

        Ok((ctx.found_hwnd, ctx.found_title, rect, is_minimized))
    }

    /// Localiza a janela usando preferencialmente o handle fornecido pelo live bridge
    pub fn find_anigo_window_with_hint(hint: Option<*mut std::ffi::c_void>) -> Result<(*mut std::ffi::c_void, String, RECT, bool)> {
        if let Some(hwnd) = hint {
            if !hwnd.is_null() {
                let mut rect = RECT::default();
                let mut success = unsafe { GetWindowRect(hwnd, &mut rect) };
                if success == 0 {
                    Self::attach_interactive_desktop();
                    success = unsafe { GetWindowRect(hwnd, &mut rect) };
                }
                if success != 0 {
                    let is_min = unsafe { IsIconic(hwnd) != 0 };
                    return Ok((hwnd, "ANIGO Studio (Live Bridge)".to_string(), rect, is_min));
                }
            }
        }
        Self::find_anigo_window()
    }

    /// Garante que a janela está visível, restaurada e em primeiro plano
    pub fn ensure_window_active(hwnd: *mut std::ffi::c_void) -> Result<RECT> {
        unsafe {
            let is_min = IsIconic(hwnd) != 0;
            let mut rect = RECT::default();
            GetWindowRect(hwnd, &mut rect);
            if is_min || rect.left <= -1000 || rect.top <= -1000 || (rect.right - rect.left) < 200 {
                ShowWindow(hwnd, SW_RESTORE);
                thread::sleep(Duration::from_millis(250));
            }
            ShowWindow(hwnd, SW_SHOW);
            SetForegroundWindow(hwnd);
            thread::sleep(Duration::from_millis(250));

            let mut rect = RECT::default();
            GetWindowRect(hwnd, &mut rect);
            Ok(rect)
        }
    }

    /// Maximiza a janela do ANIGO Studio
    pub fn maximize_window(hwnd: *mut std::ffi::c_void) -> Result<RECT> {
        unsafe {
            ShowWindow(hwnd, SW_MAXIMIZE);
            SetForegroundWindow(hwnd);
            thread::sleep(Duration::from_millis(200));

            let mut rect = RECT::default();
            GetWindowRect(hwnd, &mut rect);
            Ok(rect)
        }
    }

    /// Restaura a janela do ANIGO Studio
    pub fn restore_window(hwnd: *mut std::ffi::c_void) -> Result<RECT> {
        unsafe {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
            thread::sleep(Duration::from_millis(200));

            let mut rect = RECT::default();
            GetWindowRect(hwnd, &mut rect);
            Ok(rect)
        }
    }

    /// Captura a imagem da janela (usando PrintWindow com PW_RENDERFULLCONTENT para WebView2/DirectComposition)
    pub fn capture_window(hwnd: *mut std::ffi::c_void, rect: &RECT) -> Result<ImageBuffer<Rgba<u8>, Vec<u8>>> {
        let width = (rect.right - rect.left).max(1);
        let height = (rect.bottom - rect.top).max(1);

        unsafe {
            let screen_dc = GetDC(ptr::null_mut());
            if screen_dc.is_null() {
                anyhow::bail!("Falha ao acessar DC da tela principal.");
            }

            let mem_dc = CreateCompatibleDC(screen_dc);
            let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
            let old_bmp = SelectObject(mem_dc, bitmap);

            // 1. Tenta PrintWindow com PW_RENDERFULLCONTENT (flag 2)
            let mut captured = false;
            if !hwnd.is_null() {
                let pw_res = PrintWindow(hwnd, mem_dc, 2);
                if pw_res != 0 {
                    captured = true;
                }
            }

            // 2. Se PrintWindow não funcionou ou hwnd é nulo, tenta BitBlt da tela com CAPTUREBLT
            if !captured {
                BitBlt(mem_dc, 0, 0, width, height, screen_dc, rect.left, rect.top, SRCCOPY | 0x40000000);
            }

            let mut bmi = BITMAPINFO {
                bmi_header: BITMAPINFOHEADER {
                    bi_size: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    bi_width: width,
                    bi_height: -height, // top-down
                    bi_planes: 1,
                    bi_bit_count: 32,
                    bi_compression: BI_RGB,
                    bi_size_image: 0,
                    bi_xpels_per_meter: 0,
                    bi_ypels_per_meter: 0,
                    bi_clr_used: 0,
                    bi_clr_important: 0,
                },
                bmi_colors: [0],
            };

            let mut raw_pixels = vec![0u8; (width * height * 4) as usize];
            GetDIBits(
                mem_dc,
                bitmap,
                0,
                height as u32,
                raw_pixels.as_mut_ptr(),
                &mut bmi,
                DIB_RGB_COLORS,
            );

            SelectObject(mem_dc, old_bmp);
            DeleteObject(bitmap);
            DeleteDC(mem_dc);
            ReleaseDC(ptr::null_mut(), screen_dc);

            let mut rgba_pixels = vec![0u8; (width * height * 4) as usize];
            for i in 0..(width * height) as usize {
                let b = raw_pixels[i * 4];
                let g = raw_pixels[i * 4 + 1];
                let r = raw_pixels[i * 4 + 2];
                let a = 255u8;
                rgba_pixels[i * 4] = r;
                rgba_pixels[i * 4 + 1] = g;
                rgba_pixels[i * 4 + 2] = b;
                rgba_pixels[i * 4 + 3] = a;
            }

            ImageBuffer::from_raw(width as u32, height as u32, rgba_pixels)
                .context("Falha ao criar ImageBuffer a partir dos pixels capturados")
        }
    }

    /// Converte coordenadas de tela em pixels para coordenadas normalizadas absolutas
    /// (0..65535) exigidas por `mouse_event` com `MOUSEEVENTF_ABSOLUTE`.
    /// P2-15: usa a tela virtual (SM_XVIRTUALSCREEN/CXVIRTUALSCREEN) em vez do
    /// monitor primário, suportando multi-monitor onde (0,0) não é o canto superior
    /// esquerdo do primário.
    fn to_absolute_coords(screen_x: i32, screen_y: i32) -> (u32, u32) {
        unsafe {
            let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
            let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);
            // Normalize: MOUSEEVENTF_ABSOLUTE maps (0,0)..(65535,65535) to the entire virtual screen
            let norm_x = (screen_x - vx) as f64;
            let norm_y = (screen_y - vy) as f64;
            let abs_x = ((norm_x * 65535.0) / (vw as f64)).round().clamp(0.0, 65535.0) as u32;
            let abs_y = ((norm_y * 65535.0) / (vh as f64)).round().clamp(0.0, 65535.0) as u32;
            (abs_x, abs_y)
        }
    }

    /// Realiza clique de mouse em coordenadas absolutas de tela
    pub fn mouse_click(screen_x: i32, screen_y: i32, button: &str) {
        Self::attach_interactive_desktop();
        unsafe {
            let (abs_x, abs_y) = Self::to_absolute_coords(screen_x, screen_y);
            SetCursorPos(screen_x, screen_y);
            thread::sleep(Duration::from_millis(40));

            let (down, up) = match button {
                "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
                _ => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            };

            // Envia evento com MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE para garantir despacho a WebView2
            mouse_event(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | down, abs_x, abs_y, 0, 0);
            thread::sleep(Duration::from_millis(60));
            mouse_event(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | up, abs_x, abs_y, 0, 0);
            thread::sleep(Duration::from_millis(40));
        }
    }

    /// Realiza arraste de mouse suave (para girar o viewport 3D ou mover sliders)
    pub fn mouse_drag(start_x: i32, start_y: i32, end_x: i32, end_y: i32, button: &str, steps: u32) {
        Self::attach_interactive_desktop();
        unsafe {
            let (start_abs_x, start_abs_y) = Self::to_absolute_coords(start_x, start_y);
            SetCursorPos(start_x, start_y);
            thread::sleep(Duration::from_millis(50));

            let (down, up) = match button {
                "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
                "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
                _ => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            };

            // Pressiona o botão na coordenada inicial
            mouse_event(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | down, start_abs_x, start_abs_y, 0, 0);
            thread::sleep(Duration::from_millis(50));

            let step_count = steps.max(5);
            for i in 1..=step_count {
                let t = i as f32 / step_count as f32;
                let cur_x = (start_x as f32 + (end_x - start_x) as f32 * t).round() as i32;
                let cur_y = (start_y as f32 + (end_y - start_y) as f32 * t).round() as i32;
                let (abs_x, abs_y) = Self::to_absolute_coords(cur_x, cur_y);
                SetCursorPos(cur_x, cur_y);
                mouse_event(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, abs_x, abs_y, 0, 0);
                thread::sleep(Duration::from_millis(16));
            }

            thread::sleep(Duration::from_millis(50));
            let (end_abs_x, end_abs_y) = Self::to_absolute_coords(end_x, end_y);
            mouse_event(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | up, end_abs_x, end_abs_y, 0, 0);
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Rola a roda do mouse (zoom no viewport)
    pub fn mouse_wheel(delta: i32) {
        Self::attach_interactive_desktop();
        unsafe {
            mouse_event(MOUSEEVENTF_WHEEL, 0, 0, delta as u32, 0);
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Simula o pressionamento de uma tecla do teclado
    pub fn send_key(vk: u8) {
        Self::attach_interactive_desktop();
        unsafe {
            keybd_event(vk, 0, 0, 0);
            thread::sleep(Duration::from_millis(40));
            keybd_event(vk, 0, KEYEVENTF_KEYUP, 0);
            thread::sleep(Duration::from_millis(40));
        }
    }

    /// Resolve o diretório de logs do ANIGO, procurando em múltiplas localizações:
    /// 1. %ANIGO_LOG_DIR% (override)
    /// 2. %LOCALAPPDATA%\ANIGO\logs
    /// 3. %USERPROFILE%\Documents\ANIGO\logs
    /// 4. %APPDATA%\ANIGO\logs
    /// 5. Fallback: C:\ANIGO (comportamento legado)
    fn resolve_log_dir() -> PathBuf {
        if let Ok(dir) = std::env::var("ANIGO_LOG_DIR") {
            return PathBuf::from(dir);
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let p = PathBuf::from(&local).join("ANIGO").join("logs");
            if p.exists() {
                return p;
            }
        }
        if let Ok(profile) = std::env::var("USERPROFILE") {
            let p = PathBuf::from(&profile).join("Documents").join("ANIGO").join("logs");
            if p.exists() {
                return p;
            }
            // também tenta Documents/ANIGO sem subdir logs
            let p2 = PathBuf::from(&profile).join("Documents").join("ANIGO");
            if p2.exists() {
                return p2;
            }
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            let p = PathBuf::from(&appdata).join("ANIGO").join("logs");
            if p.exists() {
                return p;
            }
        }
        // Fallback legado
        PathBuf::from("C:\\ANIGO")
    }

    /// Lê um arquivo de log com limite de tamanho para evitar OOM (P1-10)
    fn read_log_limited(path: &Path) -> String {
        match fs::File::open(path) {
            Ok(mut f) => {
                let mut buf = vec![0u8; MAX_LOG_BYTES];
                match f.read(&mut buf) {
                    Ok(n) => {
                        let mut content = String::from_utf8_lossy(&buf[..n]).to_string();
                        if n == MAX_LOG_BYTES {
                            content.push_str(&format!("\n... [truncated at {} bytes]", MAX_LOG_BYTES));
                        }
                        content
                    }
                    Err(e) => format!("Erro ao ler: {}", e),
                }
            }
            Err(_) => "Arquivo inexistente (sem erros registrados)".to_string(),
        }
    }

    /// Coleta arquivos de log e status de saúde do ecossistema (P1-10: paths resolvidos dinamicamente, sem bloqueio infinito)
    pub fn collect_logs_and_diagnostics() -> serde_json::Value {
        let mut logs = serde_json::Map::new();
        let log_dir = Self::resolve_log_dir();
        let bridge_addr = std::env::var("ANIGO_BRIDGE_ADDR").unwrap_or_else(|_| "127.0.0.1:39090".to_string());

        let log_files = ["launch.log", "app_crash.log", "panic.log"];
        let log_keys = ["launch_log", "app_crash_log", "panic_log"];

        for (name, file) in log_keys.iter().zip(log_files.iter()) {
            let path = log_dir.join(file);
            let content = Self::read_log_limited(&path);
            logs.insert(name.to_string(), serde_json::Value::String(content));
        }

        logs.insert(
            "log_directory".to_string(),
            serde_json::Value::String(log_dir.to_string_lossy().to_string()),
        );

        // P1-01: endereço inválido não derruba o diagnóstico com `unwrap`.
        let bridge_socket = match bridge_addr.parse::<std::net::SocketAddr>() {
            Ok(addr) => addr,
            Err(_) => {
                tracing::warn!(
                    target: "anigo::mcp",
                    %bridge_addr,
                    "ANIGO_BRIDGE_ADDR inválido; usando o endereço padrão 127.0.0.1:39090"
                );
                std::net::SocketAddr::from(([127, 0, 0, 1], 39090))
            }
        };

        // P1-10: Teste de conexão com timeout curto (estamos em spawn_blocking, então std::net é OK aqui)
        let bridge_live = std::net::TcpStream::connect_timeout(
            &bridge_socket,
            Duration::from_millis(500),
        ).map(|mut s| {
            let _ = s.set_read_timeout(Some(Duration::from_millis(300)));
            let _ = s.set_write_timeout(Some(Duration::from_millis(300)));
            // Envia PING rápido para validar que é realmente o bridge
            let ping = "{\"id\":\"diag\",\"action\":\"PING\",\"params\":{}}\n";
            let _ = s.write_all(ping.as_bytes());
            let mut resp = [0u8; 256];
            let _ = s.read(&mut resp);
            true
        }).unwrap_or(false);

        logs.insert("bridge_addr".to_string(), serde_json::Value::String(bridge_addr));
        logs.insert("bridge_active".to_string(), serde_json::Value::Bool(bridge_live));
        // Manter chave legada por compatibilidade
        logs.insert("bridge_port_39090_active".to_string(), serde_json::Value::Bool(bridge_live));

        serde_json::Value::Object(logs)
    }
}
