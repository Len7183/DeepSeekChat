// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// DeepSeekChat：DeepSeek 网页版桌面客户端（非官方）
// 模块：数据目录 / 配置 / 窗口状态 / 日志 / 动作

mod actions;
mod commands;
mod config;
mod export;
mod injected;
mod logging;
mod memory;
mod paths;
mod theme;
mod tray;
mod webview2_boot;
mod webview_cfg;
mod window_state;

use std::time::{Duration, Instant};

use tauri::{AppHandle, Listener, Manager, WindowEvent};
use tauri_plugin_autostart::MacosLauncher;

/// 官网地址（主窗口内容）
pub const DEEPSEEK_URL: &str = "https://chat.deepseek.com";
/// 项目主页（设置窗口底部「关于DeepSeekChat」入口）
pub const PROJECT_HOME_URL: &str = "https://github.com/Len7183/DeepSeekChat";

pub struct AppState {
    pub config: std::sync::Mutex<config::Config>,
    /// 窗口状态保存防抖
    pub last_state_save: std::sync::Mutex<Instant>,
    /// 本次运行固定的 WebView2 参数（省内存模式重启生效，FR-08 硬约束）
    pub browser_args: String,
}

/// 防抖时间戳初值：回拨 10 秒，等价于「已到期」，首次移动/缩放立即落盘。
/// 用 `checked_sub` 兜住下溢——`Instant - Duration` 在系统运行不足 10 秒时会 panic，
/// 而本项目 `panic = "abort"`，那样就是启动即崩。
fn initial_save_stamp() -> Instant {
    let now = Instant::now();
    now.checked_sub(Duration::from_secs(10)).unwrap_or(now)
}

fn main() {
    // --silent-start：开机自启触发的启动，静默进入托盘（FR-03）
    let silent_start = std::env::args().any(|a| a == "--silent-start");

    // 数据目录与 WebView2 用户数据目录必须最先就位（FR-06/FR-07）
    paths::init();

    // WebView2 Runtime 检测与引导（FR-11），必须在创建任何 WebView 之前。
    // 此刻 logger 尚未安装，结果留到 setup 里落盘（与 paths::init 同源）
    let runtime_outcome = webview2_boot::ensure_runtime();

    let config = config::load();
    // 本次运行的 WebView2 参数在此定格：省内存模式改动后需重启才生效（FR-08）
    let browser_args = actions::browser_args_from_config(&config);

    tauri::Builder::default()
        // 单实例必须是第一个注册的插件（FR-02，WebView2 用户数据目录硬约束）
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            log::info!("检测到二次启动，唤起已运行实例");
            actions::show_main(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--silent-start"]),
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(logging::plugin().build())
        .manage(AppState {
            config: std::sync::Mutex::new(config),
            last_state_save: std::sync::Mutex::new(initial_save_stamp()),
            browser_args,
        })
        .manage(theme::ThemeState::default())
        .manage(memory::MemoryState::default())
        .setup(move |app| {
            logging::prune_old_logs();
            log::info!("DeepSeekChat 启动（silent_start={silent_start}）");
            // 只记初始化结果，实际路径在设置页查看，避免日志暴露本机用户名。
            log::info!("数据目录已初始化");
            if runtime_outcome == webview2_boot::RuntimeOutcome::InstalledByBootstrap {
                log::info!("WebView2 Runtime 曾缺失，已由内嵌引导程序安装");
            }

            // 官网页事件监听（注入脚本经 plugin:event|emit 上报，远程源 ACL 唯一通路）
            let h_theme = app.handle().clone();
            app.listen_any("dschat://theme", move |event| {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                    let dark = v.get("dark").and_then(|d| d.as_bool()).unwrap_or(false);
                    let bg = v.get("bg").and_then(|b| b.as_str()).unwrap_or("");
                    // 入口白名单：仅接受 ASCII 的 #RRGGBB。任意字符串透传会让下游
                    // theme::parse_hex 的按字节切片（s[0..2]）在 6 字节非 ASCII 输入
                    // （如 "日日"）上 panic——panic = "abort" 下远程页面可杀死进程。
                    let bg_ok = bg.len() == 7
                        && bg.starts_with('#')
                        && bg[1..].bytes().all(|b| b.is_ascii_hexdigit());
                    if bg_ok {
                        theme::set_page_theme(&h_theme, dark, bg);
                    }
                }
            });
            let h_heart = app.handle().clone();
            app.listen_any("dschat://heartbeat", move |_event| {
                memory::note_activity(&h_heart);
            });
            // 刷新（FR-21）：注入脚本拦截 F5/Ctrl+R 后经事件通道请求
            let h_reload = app.handle().clone();
            app.listen_any("dschat://reload", move |_event| {
                actions::reload_main(&h_reload);
            });
            // 全屏翻转（FR-23，F11）
            let h_fs = app.handle().clone();
            app.listen_any("dschat://fullscreen", move |_event| {
                webview_cfg::toggle_fullscreen(&h_fs);
            });
            // 对话导出（FR-22）：页面抓取结果经事件通道落盘（FR-20 唯一通路）
            let h_export = app.handle().clone();
            app.listen_any("dschat://export", move |event| {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                    export::handle_export_event(&h_export, &v);
                }
            });
            // 注入脚本的日志通道（风控弹窗自动关闭的记录与 QA 探针回传）
            app.listen_any("dschat://log", move |event| {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                    if let Some(msg) = v.get("msg").and_then(|m| m.as_str()) {
                        if msg == "risk-dialog-dismissed" {
                            log::info!("已自动关闭官网环境提醒弹窗");
                        } else {
                            // QA 回传仅供开发排障，生产日志不写任意网页载荷。
                            #[cfg(debug_assertions)]
                            log::debug!("页面日志: {msg}");
                        }
                    }
                }
            });

            // 手动启动显示主窗口；开机自启静默进托盘（FR-03）
            // 导航失败监听从 create_main 内部注册（重建路径同样覆盖，见 FR-12）
            actions::create_main(app.app_handle(), !silent_start)?;

            // 托盘常驻（FR-03）
            tray::setup(app.app_handle())?;
            // 预创建托盘菜单窗口：不与主窗口争抢启动期——主 webview 先建先加载，
            // 菜单窗延迟 3 秒后台预建（右键早于它的极端场景由 ensure_menu_window 兜底）
            {
                let h = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(3));
                    tray::precreate_menu_window(&h);
                });
            }

            // 空闲监视线程：空闲修剪 + 长挂起销毁（FR-09）
            memory::spawn_idle_monitor(app.handle().clone());

            // --settings：开发时直接打开设置窗口
            #[cfg(debug_assertions)]
            if std::env::args().any(|a| a == "--settings") {
                actions::open_settings(app.app_handle())?;
            }

            // --menu-test：在屏幕固定位置显示托盘菜单窗（仅开发诊断）
            #[cfg(debug_assertions)]
            if std::env::args().any(|a| a == "--menu-test") {
                tray::show_menu_debug(app.handle());
            }

            // DSCHAT_CMD_TEST=1：8 秒后分别从「菜单窗口（本地页，用户真实路径）」和
            // 「主窗口（远程页）」invoke menu_action('settings')，结果经 dschat://log 回传
            #[cfg(debug_assertions)]
            if std::env::var("DSCHAT_CMD_TEST").as_deref() == Ok("1") {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(8));
                    // 占位符取 __TAG__ 而非 TAG：脚本里出现其它含 TAG 的子串时不会被误替换
                    let probe = r#"(function(){
  function report(tag, msg){ try { window.__TAURI_INTERNALS__.invoke('plugin:event|emit', { event: 'dschat://log', payload: { msg: 'CMD_TEST@@' + tag + ': ' + msg } }); } catch(e){} }
  try {
    window.__TAURI_INTERNALS__.invoke('menu_action', { action: 'settings' })
      .then(function(){ report(__TAG__, 'settings OK'); })
      .catch(function(e){ report(__TAG__, 'settings ERR: ' + (e && e.message ? e.message : String(e))); });
  } catch(e) { report(__TAG__, 'EXC: ' + e); }
})();"#;
                    for (label, tag) in [("menu", "'MENU'"), ("main", "'MAIN'")] {
                        if let Some(w) = handle.get_webview_window(label) {
                            let js = probe.replace("__TAG__", tag);
                            let _ = w.eval(&js);
                        }
                    }
                });
            }

            // 以下均为 debug 构建专属入口（release 不参与编译）
            #[cfg(debug_assertions)]
            {
                // --error-test：主窗口导航到本地错误页（验证 FR-12 UI 与自动重试回路）
                if std::env::args().any(|a| a == "--error-test") {
                    webview_cfg::show_error_page(app.handle(), 12);
                }
                // --ext-test：导航到外部域名（验证 FR-13 外链走系统浏览器）
                if std::env::args().any(|a| a == "--ext-test") {
                    let ext = tauri::Url::parse("https://example.com/").unwrap();
                    let _ = app.get_webview_window("main").and_then(|w| w.navigate(ext).ok());
                }
                // DSCHAT_DIALOG_PROBE=1：探测官网「使用环境异常」弹窗 DOM 结构
                if std::env::var("DSCHAT_DIALOG_PROBE").as_deref() == Ok("1") {
                    let handle = app.handle().clone();
                    std::thread::spawn(move || {
                        for delay in [10, 20, 35] {
                            std::thread::sleep(std::time::Duration::from_secs(delay));
                            if let Some(w) = handle.get_webview_window("main") {
                                let _ = w.eval(r#"(function(){
  function findDialog() {
    var walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    while (walker.nextNode()) {
      var t = walker.currentNode.textContent || '';
      if (t.indexOf('使用环境异常') >= 0 || t.indexOf('官方产品') >= 0) {
        var el = walker.currentNode.parentElement;
        var chain = [];
        var cur = el;
        for (var i = 0; i < 8 && cur && cur !== document.body; i++) {
          try {
            chain.push(cur.tagName + '.' + String(cur.className).slice(0, 120) + ' | ' + cur.outerHTML.slice(0, 260));
          } catch (e) {}
          cur = cur.parentElement;
        }
        return chain;
      }
    }
    return null;
  }
  var r = findDialog();
  var payload = r ? JSON.stringify(r).slice(0, 2200) : 'NOT_FOUND';
  try {
    window.__TAURI_INTERNALS__.invoke('plugin:event|emit', { event: 'dschat://log', payload: { msg: 'DIALOG_PROBE@@' + payload } });
  } catch (e) {}
})();"#);
                            }
                        }
                    });
                }
            }

            // DSCHAT_EXPORT_PROBE=1：探测官网对话 DOM（FR-22 校准）；
            // 追加 DSCHAT_EXPORT_TEST=md|txt|html|doc|json：18 秒后自动触发一次导出
            #[cfg(debug_assertions)]
            if std::env::var("DSCHAT_EXPORT_PROBE").as_deref() == Ok("1") {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(12));
                    if let Some(w) = handle.get_webview_window("main") {
                        let _ = w.eval(crate::export::EXPORT_PROBE_JS);
                    }
                });
            }
            #[cfg(debug_assertions)]
            if let Ok(fmt) = std::env::var("DSCHAT_EXPORT_TEST") {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(18));
                    if let Some(w) = handle.get_webview_window("main") {
                        // 直接调导出入口：先点开侧栏第一个会话（探针实测 ._546d736 为
                        // 列表项），等 .ds-message 出现后点菜单项；不依赖工具条可见性
                        let _ = w.eval(format!(
                            r#"(function(){{
  function openConv(cb) {{
    if (document.querySelector('.ds-message')) {{ cb(); return; }}
    var items = document.querySelectorAll('._546d736');
    for (var i=0;i<items.length;i++) {{
      var t = (items[i].textContent || '').trim();
      if (t.length > 4 && t.length < 60) {{ items[i].click(); break; }}
    }}
    setTimeout(function() {{ if (document.querySelector('.ds-message')) cb(); else openConv(cb); }}, 2000);
  }}
  openConv(function() {{
    var btns = document.querySelectorAll('#dschat-export-menu .dschat-mi');
    for (var i=0;i<btns.length;i++) {{
      if (btns[i].textContent.indexOf('{fmt}') >= 0) {{ btns[i].click(); return; }}
    }}
  }});
}})();"#
                        ));
                    }
                });
            }

            // debug 探针：DSCHAT_PROBE=1 时向主窗口注入诊断横幅（仅 debug 构建参与编译）
            #[cfg(debug_assertions)]
            if std::env::var("DSCHAT_PROBE").as_deref() == Ok("1") {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(8));
                    if let Some(w) = handle.get_webview_window("main") {
                        let _ = w.eval(
                            r#"(function(){
  var r = { ipc: typeof window.__TAURI_INTERNALS__, inj: !!window.__DSCHAT_INJECTED__ };
  try {
    var els = [document.documentElement, document.body];
    var el = document.body && document.body.firstElementChild;
    for (var i = 0; i < 5 && el; i++) { els.push(el); el = el.firstElementChild; }
    for (var i = 0; i < els.length; i++) {
      var bg = getComputedStyle(els[i]).backgroundColor;
      if (bg && bg.indexOf('rgba') === -1 && bg !== 'transparent') { r.bg = bg; break; }
    }
  } catch (e) { r.err = String(e); }
  var b = document.createElement('div');
  b.style.cssText = 'position:fixed;top:0;left:0;z-index:999999;background:#ff0;color:#000;font:14px monospace;padding:6px;';
  b.textContent = 'PROBE ipc=' + r.ipc + ' inj=' + r.inj + ' bg=' + (r.bg || 'none') + ' err=' + (r.err || '-');
  document.body.appendChild(b);
})();"#,
                        );
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_config,
            commands::set_config,
            commands::get_data_dir,
            commands::open_data_dir,
            commands::open_project_home,
            commands::menu_action,
            commands::retry_navigation,
            theme::get_theme,
        ])
        .on_window_event(|window, event| {
            if window.label() != "main" {
                // 托盘菜单窗的收起由 tray.rs 的低级鼠标钩子负责（点击菜单外即收起），
                // 不再监听焦点事件：弹出瞬间主窗口 webview 抢焦点会让菜单闪没。
                return;
            }
            match event {
                // FR-04：关闭按钮默认隐藏到托盘，设置可改直接退出
                WindowEvent::CloseRequested { api, .. } => {
                    let app: &AppHandle = window.app_handle();
                    crate::window_state::save_snapshot(window);
                    if actions::close_is_hide(app) {
                        api.prevent_close();
                        log::info!("关闭按钮 → 隐藏到托盘");
                        let _ = window.hide();
                        // 托盘挂起路径：延迟 TrySuspend + 修剪（FR-09）
                        memory::on_main_hidden(app);
                    } else {
                        log::info!("关闭按钮 → 直接退出");
                        app.exit(0);
                    }
                }
                // FR-01：窗口尺寸/位置记忆（防抖保存）
                WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                    let app: &AppHandle = window.app_handle();
                    let should_save = {
                        let app_state = app.state::<AppState>();
                        // 锁毒化只是某次持有方 panic，数据（时间戳）本身无害——
                        // 恢复而不是永久放弃：Err(_) => return 会让 FR-01
                        // 位置记忆从此静默失效
                        let mut last = match app_state.last_state_save.lock() {
                            Ok(g) => g,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                        let due = last.elapsed() >= Duration::from_millis(800);
                        if due {
                            *last = Instant::now();
                        }
                        due
                    };
                    if should_save {
                        crate::window_state::save_snapshot(window);
                    }
                }
                WindowEvent::Focused(false) => {
                    crate::window_state::save_snapshot(window);
                    // 失焦修剪（FR-09：触发时机=窗口失焦）
                    let app: &AppHandle = window.app_handle();
                    memory::prune_on_focus_loss(app);
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .unwrap_or_else(|e| {
            // release 下 windows_subsystem="windows" 无控制台，panic=abort 时
            // expect 消息用户不可见；启动失败必须给出原生对话框再退出
            log::error!("DeepSeekChat 启动失败: {e}");
            crate::webview2_boot::alert_box(&format!("DeepSeekChat 启动失败：\n\n{e}"));
            std::process::exit(1);
        });
}
