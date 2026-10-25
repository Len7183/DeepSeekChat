// 主题同步（FR-10）：官网页面深浅色 → 主窗口/设置窗口标题栏与边框、设置窗口与托盘菜单样式。
// 对标基准 deepseek-ai/deepseek-harness（apps/desktop）的两段式：
//   1) 启动回退色 chromeFallbackFill：深 #1b1b1c / 浅 #f9fafb，符号色深 #f9fafb / 浅 #0f1115；
//   2) 渲染端实测颜色经 IPC 上报后替换（本实现由注入脚本实测页面背景色上报）。
// Windows 11：DWMWA_CAPTION_COLOR/TEXT_COLOR/BORDER_COLOR 精确着色；
// Windows 10 1809+：仅 DWMWA_USE_IMMERSIVE_DARK_MODE 深浅切换（属性 34/35/36 在 Win10 调用失败即忽略）。

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};

/// 启动回退色（页面尚未上报时使用）
const FALLBACK_DARK_BG: &str = "#1b1b1c";
const FALLBACK_LIGHT_BG: &str = "#f9fafb";
const SYMBOL_DARK: (u8, u8, u8) = (0xf9, 0xfa, 0xfb); // 深色标题栏上的文字
const SYMBOL_LIGHT: (u8, u8, u8) = (0x0f, 0x11, 0x15); // 浅色标题栏上的文字

#[derive(Default)]
pub struct ThemeState {
    pub dark: Mutex<bool>,
    /// 页面实测背景色（#RRGGBB），未上报为空
    pub page_bg: Mutex<Option<String>>,
}

fn current(app: &AppHandle) -> bool {
    // 锁毒化恢复真实值：默认 false 会让 get_theme 报出与标题栏实际相反的深浅
    app.state::<ThemeState>()
        .dark
        .lock()
        .map(|d| *d)
        .unwrap_or_else(|e| *e.into_inner())
}

/// 页面上报主题：记录 → 标题栏/边框着色 → 广播给设置窗/托盘菜单。
/// 深浅标志翻转（或首次上报）才记日志：逐帧跟踪过渡色期间 bg 每帧都在变。
/// 事件在锁内判定、锁释放后发射：listen_any 回调都在主事件循环串行执行，
/// 实际无并发调用方；若未来改为多线程调用，需把 emit 挪进锁内。
pub fn set_page_theme(app: &AppHandle, dark: bool, bg: &str) {
    let (changed, dark_changed) = {
        let state = app.state::<ThemeState>();
        let mut d = state.dark.lock().unwrap_or_else(|e| e.into_inner());
        let mut b = state.page_bg.lock().unwrap_or_else(|e| e.into_inner());
        let dark_changed = *d != dark;
        let changed = dark_changed || b.as_deref() != Some(bg);
        *d = dark;
        *b = Some(bg.to_string());
        (changed, dark_changed)
    };
    if dark_changed {
        log::info!("页面主题上报: {} {}", if dark { "深色" } else { "浅色" }, bg);
    }
    apply_titlebar(app, dark, Some(bg));
    if changed {
        let _ = app.emit("theme-changed", serde_json::json!({ "dark": dark, "bg": bg }));
    }
}

/// 按当前记录状态应用（窗口创建后调用，覆盖回退态）
pub fn apply_current(app: &AppHandle) {
    let state = app.state::<ThemeState>();
    let dark = *state.dark.lock().unwrap_or_else(|e| e.into_inner());
    let bg = state.page_bg.lock().unwrap_or_else(|e| e.into_inner()).clone();
    apply_titlebar(app, dark, bg.as_deref());
}

/// 主/设置窗口标题栏与边框着色
fn apply_titlebar(app: &AppHandle, dark: bool, bg_hex: Option<&str>) {
    // 回退色（页面未上报时）
    let bg = bg_hex
        .and_then(parse_hex)
        .unwrap_or_else(|| parse_hex(if dark { FALLBACK_DARK_BG } else { FALLBACK_LIGHT_BG }).unwrap());
    let (sr, sg, sb) = if dark { SYMBOL_DARK } else { SYMBOL_LIGHT };

    // 边框：浅色加深 15%，深色提亮约 12%
    let border = if dark { lighten(bg, 0.12) } else { darken(bg, 0.15) };

    for label in ["main", "settings"] {
        let Some(win) = app.get_webview_window(label) else { continue };
        // Win10：沉浸式深浅切换（也影响窗口主题语义）
        let theme = if dark { tauri::Theme::Dark } else { tauri::Theme::Light };
        let _ = win.set_theme(Some(theme));
        // Win11：精确着色（Win10 上调用失败被忽略）
        #[cfg(windows)]
        apply_dwm(&win, dark, bg, border, (sr, sg, sb));
    }
}

#[cfg(windows)]
fn apply_dwm(
    win: &tauri::WebviewWindow,
    dark: bool,
    bg: (u8, u8, u8),
    border: (u8, u8, u8),
    text: (u8, u8, u8),
) {
    use windows::Win32::Foundation::{COLORREF, HWND};
    use windows::Win32::Graphics::Dwm::{
        DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR,
        DWMWA_USE_IMMERSIVE_DARK_MODE,
    };

    let Ok(raw) = win.hwnd() else { return };
    // tauri 返回的 HWND 可能是 isize 或指针包装，经 isize 统一转换
    let hwnd = HWND(raw.0 as isize as *mut core::ffi::c_void);

    let immersive: i32 = if dark { 1 } else { 0 };
    let caption = COLORREF(ref_from(bg));
    let text_c = COLORREF(ref_from(text));
    let border_c = COLORREF(ref_from(border));

    // SAFETY: hwnd 来自 tauri 已创建窗口的合法 HWND；DwmSetWindowAttribute 按
    // 契约只读取 cbSize=4 字节的属性值缓冲，不转移指针所有权；四个属性在
    // Win10 上部分不支持（返回错误码），调用方有意忽略——见下方按错误码留痕
    unsafe {
        let r_imm = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &immersive as *const _ as *const core::ffi::c_void,
            4,
        );
        let r_caption = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption as *const _ as *const core::ffi::c_void,
            4,
        );
        let r_text = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR,
            &text_c as *const _ as *const core::ffi::c_void,
            4,
        );
        let r_border = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border_c as *const _ as *const core::ffi::c_void,
            4,
        );
        // Win10 不支持精确着色属性（E_INVALIDARG）属预期；其它失败留痕，
        // 避免「标题栏没跟上主题」这类问题在日志里完全不可见
        for (name, r) in [
            ("IMMERSIVE_DARK_MODE", r_imm),
            ("CAPTION_COLOR", r_caption),
            ("TEXT_COLOR", r_text),
            ("BORDER_COLOR", r_border),
        ] {
            if let Err(e) = r {
                log::debug!("DwmSetWindowAttribute({name}) 失败: {e}（Win10 无精确着色属预期）");
            }
        }
    }
}

/// RGB → COLORREF 字节序（0x00BBGGRR）
#[cfg(windows)]
fn ref_from(c: (u8, u8, u8)) -> u32 {
    c.0 as u32 | ((c.1 as u32) << 8) | ((c.2 as u32) << 16)
}

fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    // 按字节解析：6 字节非 ASCII（如 "日日"）在字节切片下不会 panic，
    // 且天然把非十六进制字节拒掉。上游 main.rs 已有 ASCII 白名单，
    // 这里是第二道防线（一处重构距离的崩溃路径直接消除）
    let b = s.trim_start_matches('#').as_bytes();
    if b.len() != 6 {
        return None;
    }
    let hex = |pair: &[u8]| -> Option<u8> {
        u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()
    };
    Some((hex(&b[0..2])?, hex(&b[2..4])?, hex(&b[4..6])?))
}

fn darken(c: (u8, u8, u8), factor: f32) -> (u8, u8, u8) {
    (
        (c.0 as f32 * (1.0 - factor)) as u8,
        (c.1 as f32 * (1.0 - factor)) as u8,
        (c.2 as f32 * (1.0 - factor)) as u8,
    )
}

fn lighten(c: (u8, u8, u8), factor: f32) -> (u8, u8, u8) {
    (
        (c.0 as f32 + (255.0 - c.0 as f32) * factor) as u8,
        (c.1 as f32 + (255.0 - c.1 as f32) * factor) as u8,
        (c.2 as f32 + (255.0 - c.2 as f32) * factor) as u8,
    )
}

/// 主题查询命令（设置窗口/托盘菜单初始化用）
#[tauri::command]
pub fn get_theme(app: AppHandle) -> serde_json::Value {
    serde_json::json!({ "dark": current(&app) })
}
