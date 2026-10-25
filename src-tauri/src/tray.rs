// 托盘常驻（FR-03）：鲸鱼图标、左键显隐切换、右键弹出主题化菜单窗口。
// 菜单不用原生菜单（无法跟随页面深浅主题），改用无边框小窗自绘（FR-10）。
// 收起策略：低级鼠标钩子（WH_MOUSE_LL）检测「点击菜单外」即收起——
// 不依赖焦点事件（弹出瞬间主窗口 webview 抢焦点会让菜单闪没），
// 也不要求菜单窗口必须拿到焦点（否则 focus 失败时菜单永远关不掉）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, PhysicalPosition, WebviewUrl, WebviewWindow,
};

// 3:4 竖向比例（102:136，紧凑）。宽度低于 SM_CXMINTRACK（≈136）时，创建期
// WM_GETMINMAXINFO 早于 WM_NCCREATE、tao 约束尚未挂上，走 DefWindowProc 的
// 系统默认值会把初始宽度强制加宽（实测 102→136）；创建后程序化 set_size
// （SetWindowPos）不受交互式最小跟踪限制，故创建完补一次精确设尺（见下）。
const MENU_W: f64 = 102.0;
const MENU_H: f64 = 136.0;

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    // 图标缺失（打包配置错误）不该 panic 掉整个应用——传播为错误，
    // 让 main 的启动失败对话框告知用户，而非无声退出
    let icon = app
        .default_window_icon()
        .ok_or_else(|| tauri::Error::AssetNotFound("缺少内置窗口图标".into()))?
        .clone();

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("DeepSeekChat")
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button, button_state, position, .. } = event {
                if button_state != MouseButtonState::Up {
                    return;
                }
                let app = tray.app_handle();
                match button {
                    // 左键单击：显示/隐藏主窗口切换
                    MouseButton::Left => toggle_main(app),
                    // 右键：弹出主题化菜单。收起守卫刚触发后的短暂静默期内不重开
                    // （钩子吞掉的菜单外点击恰落在托盘图标上时，UP 仍会回调到这里，
                    // 若立即重开会形成「开→收→开」振荡，体感即右键点不开）
                    MouseButton::Right => {
                        if dismiss_cooldown_elapsed() {
                            if let Err(e) = open_menu(app, position) {
                                log::error!("弹出托盘菜单失败: {e}");
                            }
                        } else {
                            // 冷却静默期内不弹（防开/收振荡），留痕便于排障
                            log::debug!("右键落在收起冷却期内，忽略本次弹出");
                        }
                    }
                    _ => {}
                }
            }
        })
        .build(app)?;
    log::info!("托盘已创建");
    Ok(())
}

/// 左键：主窗口可见且未最小化 → 隐藏；否则唤起
fn toggle_main(app: &AppHandle) {
    match app.get_webview_window("main") {
        Some(win) => {
            let visible = win.is_visible().unwrap_or(false);
            let minimized = win.is_minimized().unwrap_or(false);
            if visible && !minimized {
                crate::actions::hide_main(app);
            } else {
                crate::actions::show_main(app);
            }
        }
        None => crate::actions::show_main(app),
    }
}

/// 确保菜单窗口存在且健康（FR-24 同源思路）：崩溃/重建后 menu 窗的 webview 可能
/// 已死（窗口句柄失效或 webview 加载失败显示错误页）——弹出前校验，失效即销毁重建。
fn ensure_menu_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if let Some(w) = app.get_webview_window("menu") {
        // 健康检查：窗口句柄可用（is_visible 能取到值）才算活
        if w.is_visible().is_ok() {
            return Ok(w);
        }
        log::warn!("托盘菜单窗口已失效，销毁重建");
        if let Err(e) = w.destroy() {
            // 销毁失败必须留痕：残留窗口占着 "menu" 标签，随后的创建会因
            // 标签冲突失败，本次弹出失败（下次点击重试）
            log::error!("销毁失效菜单窗口失败: {e}");
        }
    }
    create_menu_window(app)
}

/// 启动时预创建菜单窗口（失败仅告警，右键时再尝试）
pub fn precreate_menu_window(app: &AppHandle) {
    if app.get_webview_window("menu").is_some() {
        return;
    }
    if let Err(e) = create_menu_window(app) {
        log::warn!("预创建托盘菜单窗口失败（将在右键时重试）: {e}");
    }
}

fn create_menu_window(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    // 与主窗口使用完全相同的浏览器参数（WebView2 共享用户数据目录的硬约束）
    let browser_args = crate::actions::browser_args_for(app);
    let win = tauri::WebviewWindowBuilder::new(app, "menu", WebviewUrl::App("menu.html".into()))
        .title("DeepSeekChat 菜单")
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .visible(false)
        .additional_browser_args(&browser_args)
        .inner_size(MENU_W, MENU_H)
        // 显式清零最小尺寸约束：否则后期收到 WM_GETMINMAXINFO 时 tao 会
        // 回退到系统默认 ptMinTrackSize（SM_CXMINTRACK≈136）钳制宽度
        .min_inner_size(0.0, 0.0)
        .build()?;
    // 创建期的钳制（见常量注释）发生在 build() 内部，补一次精确设尺保住 102 宽
    let _ = win.set_size(tauri::LogicalSize::new(MENU_W, MENU_H));
    log::info!("托盘菜单窗口已创建");
    Ok(win)
}

/// QA：在屏幕固定位置显示菜单窗（仅 debug 构建）
#[cfg(debug_assertions)]
pub fn show_menu_debug(app: &AppHandle) {
    if let Err(e) = open_menu(app, PhysicalPosition::new(600.0, 500.0)) {
        log::error!("菜单调试显示失败: {e}");
    }
}

/// 收起守卫触发时刻（冷却窗口判定用）
static LAST_DISMISS: OnceLock<Mutex<Option<std::time::Instant>>> = OnceLock::new();

/// 距上次守卫收起是否已过冷却期（350ms）：钩子吞掉菜单外点击后，Explorer 收不到
/// DOWN，但托盘图标的 UP 回调仍可能送达（窗口less 区域行为差异）——此间隔内重开
/// 会与守卫收起形成振荡。
fn dismiss_cooldown_elapsed() -> bool {
    let cell = LAST_DISMISS.get_or_init(|| Mutex::new(None));
    match cell.lock() {
        Ok(g) => match *g {
            Some(t) => t.elapsed() >= std::time::Duration::from_millis(350),
            None => true,
        },
        Err(_) => true,
    }
}

fn note_dismissed() {
    let cell = LAST_DISMISS.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = cell.lock() {
        *g = Some(std::time::Instant::now());
    }
}

/// 在托盘光标处弹出菜单（贴边防出屏）
fn open_menu(app: &AppHandle, cursor: PhysicalPosition<f64>) -> tauri::Result<()> {
    let menu = ensure_menu_window(app)?;

    // 按光标所在显示器的缩放换算菜单物理尺寸，并钳制在屏内。
    // monitor_from_point 只查一次：两次调用不仅冗余，第二次失败时还会
    // 出现「尺寸按缩放换算、位置却用光标原值且不钳制」的混合态
    // （菜单出现在光标右下而非左上）
    let monitor = app.monitor_from_point(cursor.x, cursor.y).ok().flatten();
    let scale = monitor.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);
    let (mut x, mut y) = (cursor.x, cursor.y);
    let (w, h) = (MENU_W * scale, MENU_H * scale);
    if let Some(m) = &monitor {
        let mp = m.position();
        let ms = m.size();
        x = cursor.x - w; // 右对齐光标
        y = cursor.y - h - 8.0; // 光标上方
        if x < mp.x as f64 {
            x = mp.x as f64;
        }
        if y < mp.y as f64 {
            y = mp.y as f64; // 托盘在顶部等场景：贴屏顶
        }
        let max_x = mp.x as f64 + ms.width as f64 - w;
        let max_y = mp.y as f64 + ms.height as f64 - h;
        x = x.min(max_x);
        y = y.min(max_y);
    }

    menu.set_position(PhysicalPosition::new(x as i32, y as i32))?;
    // 布防矩形与窗口实际物理范围一致：菜单窗按目标屏缩放 set_size，
    // 混合 DPI 下（上次在 2x 屏、这次在 1x 屏）窗口物理尺寸可能仍是旧屏
    // 换算值，按旧值布防会把菜单真实区域的点击误判为「菜单外」而吞掉。
    let target = tauri::PhysicalSize::new(w as u32, h as u32);
    if menu.outer_size().map(|s| s != target).unwrap_or(true) {
        let _ = menu.set_size(target);
    }
    // 先布防再显示：show 与钩子安装之间的窗口期里，点击菜单外既不会被吞
    // 也不会触发收起（守卫盲区）；布防只改共享状态 + 启动泵线程，先做无害
    arm_popup_guard(app, (x as i32, y as i32, (x + w) as i32, (y + h) as i32));
    menu.show()?;
    menu.set_focus()?;
    log::info!("托盘菜单已弹出 ({x:.0},{y:.0})");
    Ok(())
}

// ---------------------------------------------------------------------------
// 弹层收起守卫：低级鼠标钩子（点击菜单外即收起）
//
// 设计约束：钩子回调跑在系统输入路径上（每个鼠标事件同步调用），超过
// LowLevelHooksTimeout 会被 Windows 静默摘钩。因此回调内**绝不取锁**：
// 布防矩形与泵线程 id 全部用原子变量，回调只做原子读 + 一次 PostThreadMessageW。
// 锁（POPUP）只在布防/撤防/泵收尾等非热路径上使用，且只保护 PopupState 的
// 生命周期（AppHandle 归属与 take 语义），不参与点击判定热路径。
// ---------------------------------------------------------------------------

struct PopupState {
    app: AppHandle,
}

static POPUP: OnceLock<Mutex<Option<PopupState>>> = OnceLock::new();
static DISMISS: AtomicBool = AtomicBool::new(false);
/// 布防矩形（物理像素，l/t/r/b）。GUARD_ACTIVE 置位前不得写入。
static RECT_L: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static RECT_T: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static RECT_R: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static RECT_B: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// 泵线程 id（0 = 未知/未就绪）。钩子回调读它来投递 WM_QUIT。
static PUMP_TID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// 守卫是否生效（矩形已发布且泵线程存在）。回调先查它，避免在
/// 泵线程还没起来时把点击吞掉却无人负责收起。
static GUARD_ACTIVE: AtomicBool = AtomicBool::new(false);

// 命名常量（替代裸魔数，FFI 边界可审计）
/// WM_LBUTTONDOWN / WM_RBUTTONDOWN / WM_NCLBUTTONDOWN / WM_NCRBUTTONDOWN
const MOUSE_DOWN_MSGS: [u32; 4] = [0x0201, 0x0204, 0x00A1, 0x00A4];
const WM_QUIT: u32 = 0x0012;

fn popup_cell() -> &'static Mutex<Option<PopupState>> {
    POPUP.get_or_init(|| Mutex::new(None))
}

/// 弹出菜单时布防；已在防区则更新矩形
fn arm_popup_guard(app: &AppHandle, rect: (i32, i32, i32, i32)) {
    // 先写矩形再置 GUARD_ACTIVE（Release 序）：回调看到 active 时矩形必然已发布
    RECT_L.store(rect.0, Ordering::Release);
    RECT_T.store(rect.1, Ordering::Release);
    RECT_R.store(rect.2, Ordering::Release);
    RECT_B.store(rect.3, Ordering::Release);

    let cell = popup_cell();
    let spawn = {
        let mut guard = match cell.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        match guard.as_mut() {
            // 已布防（泵线程活着）：矩形已更新，直接激活
            Some(_) => {
                GUARD_ACTIVE.store(true, Ordering::Release);
                false
            }
            None => {
                *guard = Some(PopupState { app: app.clone() });
                true
            }
        }
    };
    if spawn {
        // 先置 active 再 spawn 会让回调抢在泵线程/PUMP_TID 就绪前吞点击——
        // 那时 PostThreadMessageW(0) 无处投递，点击被吞却没人收起菜单。
        // active 由泵线程把 PUMP_TID 写好后自行置位（见 run_popup_guard）。
        std::thread::Builder::new()
            .name("menu-popup-guard".into())
            .spawn(run_popup_guard)
            .map_err(|e| log::warn!("弹层守卫线程启动失败: {e}"))
            .ok();
    }
}

/// 收起菜单（菜单项点击 / Esc / 程序性隐藏）：撤防 + 隐藏窗口
pub fn hide_menu(app: &AppHandle) {
    disarm_popup_guard();
    if let Some(menu) = app.get_webview_window("menu") {
        let _ = menu.hide();
    }
}

fn disarm_popup_guard() {
    // 顺序关键：先摘 active（回调立即放行，不再吞点击），再清 DISMISS、停泵。
    // 反过来会有一瞬 DISMISS=false 但回调仍活跃：钩子吞点击 → 置 DISMISS=true
    // → 泵收尾读到 true 又 note_dismissed，把一次正常收起记成冷却起点。
    GUARD_ACTIVE.store(false, Ordering::Release);
    DISMISS.store(false, Ordering::SeqCst);
    let tid = PUMP_TID.load(Ordering::Acquire);
    if tid != 0 {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                tid,
                WM_QUIT,
                windows::Win32::Foundation::WPARAM(0),
                windows::Win32::Foundation::LPARAM(0),
            );
        }
    }
}

fn run_popup_guard() {
    const WH_MOUSE_LL: u32 = 14;
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, MSLLHOOKSTRUCT, SetWindowsHookExW, UnhookWindowsHookEx, MSG,
    };

    unsafe extern "system" fn mouse_hook_proc(
        code: i32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if code >= 0 && GUARD_ACTIVE.load(Ordering::Acquire) && MOUSE_DOWN_MSGS.contains(&(wparam.0 as u32)) {
            let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            let (x, y) = (info.pt.x, info.pt.y);
            let l = RECT_L.load(Ordering::Acquire);
            let t = RECT_T.load(Ordering::Acquire);
            let r = RECT_R.load(Ordering::Acquire);
            let b = RECT_B.load(Ordering::Acquire);
            if !(x >= l && x < r && y >= t && y < b) {
                // 菜单外点击：记意图 + 通知泵线程退出（钩子摘除与隐藏窗口由泵收尾做）。
                // 热路径上只有原子写 + 一次 PostThreadMessageW，无锁。
                DISMISS.store(true, Ordering::SeqCst);
                let tid = PUMP_TID.load(Ordering::Acquire);
                if tid != 0 {
                    unsafe {
                        let _ =
                            windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                                tid,
                                WM_QUIT,
                                WPARAM(0),
                                LPARAM(0),
                            );
                    }
                }
                return LRESULT(1); // 吞掉这次菜单外点击（与原生菜单一致）
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    let hook = unsafe {
        let hmod = GetModuleHandleW(None).unwrap_or_default();
        SetWindowsHookExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOWS_HOOK_ID(WH_MOUSE_LL as i32),
            Some(mouse_hook_proc),
            Some(hmod.into()),
            0,
        )
    };
    let Ok(hook) = hook else {
        // 安装失败：撤防，菜单仅能通过菜单项/Esc 关闭（保守退化路径）。
        // 只在状态仍归本线程时清掉（arm 失败后调用方可能已重试）。
        log::warn!("低级鼠标钩子安装失败，菜单收起退化为菜单项触发");
        GUARD_ACTIVE.store(false, Ordering::Release);
        PUMP_TID.store(0, Ordering::Release);
        if let Ok(mut g) = popup_cell().lock() {
            g.take();
        }
        return;
    };
    let tid = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
    PUMP_TID.store(tid, Ordering::Release);
    // 泵与 tid 就绪后才激活守卫：布防→安装→发布 tid→激活，回调无空窗
    GUARD_ACTIVE.store(true, Ordering::Release);

    // 消息泵：驱动钩子回调；收到 WM_QUIT 退出
    let mut msg = MSG::default();
    loop {
        let r = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if !r.as_bool() || msg.message == WM_QUIT {
            break;
        }
    }
    let _ = unsafe { UnhookWindowsHookEx(hook) };

    // 撤防收尾：先摘 active（回调放行），再按 DISMISS 决定是否隐藏菜单。
    // take() 前 active 已 false，布防方若在这之后重开菜单会走全新布防流程，
    // 不会被本线程的收尾 hide() 误伤（hide 只在 DISMISS=true 即真菜单外点击时执行）。
    GUARD_ACTIVE.store(false, Ordering::Release);
    PUMP_TID.store(0, Ordering::Release);
    let st = popup_cell().lock().ok().and_then(|mut g| g.take());
    if DISMISS.swap(false, Ordering::SeqCst) {
        if let Some(st) = st {
            if let Some(menu) = st.app.get_webview_window("menu") {
                let _ = menu.hide();
            }
            note_dismissed();
            log::info!("托盘菜单因点击菜单外而收起");
        }
    }
}
