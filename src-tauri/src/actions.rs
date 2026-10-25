// 窗口与应用级动作：主窗口创建/唤起等公共逻辑

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Manager, WebviewWindow};

use crate::{
    config::{CloseAction, Config},
    AppState, DEEPSEEK_URL,
};

/// Tauri 默认 WebView2 浏览器参数（additional_browser_args 会整体替换默认值，需显式保留）
pub const DEFAULT_BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";

/// 由配置计算浏览器参数（FR-08 省内存模式追加禁 GPU）。
/// 只在启动时调用一次：省内存模式改动后需重启才生效，运行期不得随设置变化，
/// 否则后创建的窗口会带上与既有窗口不同的 Environment 参数。
pub fn browser_args_from_config(cfg: &Config) -> String {
    let mut args = String::from(DEFAULT_BROWSER_ARGS);
    if cfg.low_memory_mode {
        args.push_str(" --disable-gpu --renderer-process-limit=1 --process-per-site");
    }
    args
}

/// 本次运行固定的浏览器参数（启动快照）。
/// WebView2 约束：共享同一用户数据目录的所有 Environment 参数必须一致，
/// 故主窗口、设置窗、托盘菜单窗（含销毁后重建的主窗口）都必须取同一结果。
pub fn browser_args_for(app: &AppHandle) -> String {
    app.state::<AppState>().browser_args.clone()
}

/// 创建主窗口（恢复窗口状态；visible=false 用于开机自启静默进托盘）
pub fn create_main(app: &AppHandle, visible: bool) -> tauri::Result<WebviewWindow> {
    // DEEPSEEK_URL 是编译期常量、当前必为合法 URL，但本函数位于托盘点击/二次启动/
    // 崩溃自愈等事件路径上，解析失败走传播而非 panic（进程退出对用户表现为「点不开」）
    let url = tauri::Url::parse(DEEPSEEK_URL)
        .map_err(|e| tauri::Error::AssetNotFound(format!("官网地址解析失败: {e}")))?;
    let saved = crate::window_state::load().and_then(|s| crate::window_state::validated(&s, app));
    let restore_maximized = saved.as_ref().map(|s| s.maximized).unwrap_or(false);

    // 浏览器参数：默认保留 Tauri 默认项；省内存模式（FR-08）追加禁 GPU 与进程精简
    let browser_args = browser_args_for(app);

    let builder = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::External(url))
        .title("DeepSeekChat")
        .min_inner_size(960.0, 600.0)
        .visible(visible)
        .additional_browser_args(&browser_args)
        // 注入官网页的脚本（每次导航后都会执行，叠加生效）：
        // INIT_JS——主题/心跳/风控弹窗（FR-09/10/18）+ 刷新快捷键（FR-21）
        // EXPORT_JS——对话导出悬浮钮与抓取（FR-22）
        .initialization_script(crate::injected::INIT_JS)
        .initialization_script(crate::export::EXPORT_JS)
        // Windows 上 Tauri 默认会替换 WebView2 的拖放处理器（改发 tauri://drag-drop），
        // 页面因此收不到 dataTransfer.files —— 官网的「拖图片进对话」会整个失效。
        // 关掉内部处理器，让页面自己按 HTML5 语义接收文件拖放。
        .disable_drag_drop_handler();
    // 顶层导航策略 + 弹窗/OAuth 策略（FR-13）
    let mut builder = crate::webview_cfg::apply_navigation_policy(app, builder);

    builder = match saved {
        Some(s) => builder.position(s.x, s.y).inner_size(s.width, s.height),
        None => builder.center().inner_size(1200.0, 800.0),
    };

    let win = builder.build()?;
    if restore_maximized {
        let _ = win.maximize();
    }
    // 应用已记录的主题（页面尚未上报时用回退色）
    crate::theme::apply_current(app);
    // 重置挂起/隐藏状态
    crate::memory::on_main_shown(app);
    // 导航失败 → 本地错误页（FR-12）：放在创建流程内，销毁重建后的主窗口同样被监听
    crate::webview_cfg::attach_failure_handler(app, &win);
    let low_mem = browser_args.contains("disable-gpu");
    log::info!("主窗口已创建（visible={visible}，low_mem={low_mem}）");
    Ok(win)
}

/// 唤起主窗口（托盘左键 / 二次启动 / 菜单「打开」）
pub fn show_main(app: &AppHandle) {
    match app.get_webview_window("main") {
        Some(win) => {
            let _ = win.show();
            let _ = win.unminimize();
            let _ = win.set_focus();
            log::info!("主窗口已唤起");
            // 已存在窗口路径在此清理挂起状态；重建路径由 create_main 内部处理
            crate::memory::on_main_shown(app);
        }
        // 长时间挂起后 webview 被销毁的场景：重建主窗口
        None => {
            log::info!("主窗口不存在，重建");
            if let Err(e) = create_main(app, true) {
                log::error!("重建主窗口失败: {e}");
                // 仅记日志会让托盘点击/二次启动表现为「没反应」；弹原生对话框可感知
                crate::webview2_boot::alert_box(&format!(
                    "主窗口重建失败，请重启应用。\n\n{e}"
                ));
            }
        }
    }
}

/// 刷新主窗口（FR-21）：ICoreWebView2::Reload 是内核级重载，保留 SPA 会话语义，
/// 不产生导航历史条目；错误页场景由 error.ts 的 F5 → retry_navigation 走导航路径。
pub fn reload_main(app: &AppHandle) {
    match app.get_webview_window("main") {
        Some(win) => {
            if let Err(e) = win.reload() {
                log::warn!("主窗口刷新失败: {e}");
            } else {
                log::info!("主窗口已刷新");
            }
        }
        // webview 已被长挂起销毁：直接重建（等于全新加载）
        None => {
            if let Err(e) = create_main(app, true) {
                log::error!("重建主窗口失败: {e}");
                crate::webview2_boot::alert_box(&format!(
                    "主窗口重建失败，请重启应用。\n\n{e}"
                ));
            }
        }
    }
}

/// 隐藏主窗口（→托盘）
pub fn hide_main(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
        log::info!("主窗口已隐藏到托盘");
        // 托盘挂起路径：延迟 TrySuspend + 修剪（FR-09）
        crate::memory::on_main_hidden(app);
    }
}

/// 关闭行为是否为「隐藏到托盘」
pub fn close_is_hide(app: &AppHandle) -> bool {
    app.state::<AppState>()
        .config
        .lock()
        .map(|c| c.close_action == CloseAction::Hide)
        .unwrap_or_else(|poisoned| {
            // 锁毒化只意味着某次持有方 panic，数据本身仍一致——恢复真实值
            // 而不是静默回退默认（用户的「直接退出」选择不应被覆盖）
            let c = poisoned.into_inner();
            let hide = c.close_action == CloseAction::Hide;
            log::warn!("配置锁已毒化，恢复读取 close_action（hide={hide}）");
            hide
        })
}

/// 当前前台窗口所属进程 id（0 = 无前台）；用于失焦诊断
fn foreground_pid() -> u32 {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    // SAFETY: GetForegroundWindow / GetWindowThreadProcessId 均为只读查询，
    // 不转移指针所有权；h 为系统返回的合法或空 HWND（空已提前返回 0），
    // pid 经出参写入后读取。
    unsafe {
        let h = GetForegroundWindow();
        if h == HWND::default() {
            return 0;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        pid
    }
}

/// 打开设置窗口（FR-05，独立小窗；失焦即隐藏）。
/// v0.1.0 性能口径：窗口与 webview 常驻（首次创建后不销毁），失焦改为 hide，
/// 再次打开零等待（对比「失焦 close + 重开建 webview」省 300~600ms）。
pub fn open_settings(app: &AppHandle) -> tauri::Result<()> {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
        return Ok(());
    }
    // 与主窗口一致的浏览器参数（WebView2 共享用户数据目录硬约束）
    let browser_args = browser_args_for(app);
    let win = tauri::WebviewWindowBuilder::new(
        app,
        "settings",
        tauri::WebviewUrl::App("settings.html".into()),
    )
    .title("DeepSeekChat 设置")
    .inner_size(522.0, 428.0)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .visible(false)
    .center()
    .additional_browser_args(&browser_args)
    .build()?;
    // 失焦即隐藏（不销毁）：需「拿到过焦点」且「已过创建静默期」，
    // 避开创建瞬间的焦点抖动 / 主窗口 webview 抢焦点导致的误隐藏（与托盘菜单同源问题）
    let app_for_event = app.clone();
    let focus_flag = Arc::new(AtomicBool::new(false));
    let flag = focus_flag.clone();
    let created = std::time::Instant::now();
    win.on_window_event(move |event| match event {
        tauri::WindowEvent::Focused(true) => {
            flag.store(true, Ordering::Relaxed);
        }
        tauri::WindowEvent::Focused(false) => {
            let had = flag.swap(false, Ordering::Relaxed);
            if !had {
                return;
            }
            // 先判定再取前台 pid：无未消费焦点时省一次 Win32 调用
            let age = created.elapsed();
            let own = foreground_pid() == std::process::id();
            if age < std::time::Duration::from_millis(1500) && own {
                // 创建期抖动：主窗口 webview 抢回焦点（与托盘菜单同源问题）。
                // 不隐藏，重新聚焦夺回；抖动若持续也被 1.5s 窗口限制。
                if let Some(w) = app_for_event.get_webview_window("settings") {
                    log::info!("设置窗口创建期焦点抖动，已重新聚焦");
                    let _ = w.set_focus();
                }
            } else {
                // 其余任意失焦（点击外部应用或本应用主窗口）→ 立即隐藏（常驻，不销毁）
                if let Some(w) = app_for_event.get_webview_window("settings") {
                    log::info!("设置窗口失焦，已隐藏（age={}ms）", age.as_millis());
                    let _ = w.hide();
                }
            }
        }
        _ => {}
    });
    crate::theme::apply_current(app);
    let _ = win.show();
    let _ = win.set_focus();
    log::info!("设置窗口已创建");
    Ok(())
}
