// WebView 行为配置（FR-12 / FR-13）：
// - 顶层导航策略：DeepSeek 域名与本地页保留在客户端，其余交系统浏览器
// - 新窗口策略：弹窗（含第三方 OAuth 登录）一律允许在应用内打开（WebView2 弹窗
//   与主窗口同浏览器进程、共享 Cookie，登录回调可完成）
// - 导航失败检测：NavigationCompleted 失败 → 跳转本地错误页（FR-12）

use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_opener::OpenerExt as _;

/// 判断是否 DeepSeek 域名
fn is_deepseek(url: &tauri::Url) -> bool {
    url.host_str()
        .map(|h| h == "deepseek.com" || h.ends_with(".deepseek.com"))
        .unwrap_or(false)
}

/// 判断是否应用本地页（错误页等）
fn is_local(url: &tauri::Url) -> bool {
    match url.host_str() {
        // tauri.localhost 是 release 内嵌资源 origin；localhost/127.0.0.1 仅
        // dev 的 Vite 服务需要——release 放行任意本地端口会让页面重定向
        // 到本机其他服务时留在应用内（而非交系统浏览器），故 dev-only
        Some("tauri.localhost") => true,
        Some("localhost") | Some("127.0.0.1") => cfg!(debug_assertions),
        _ => url.scheme() == "tauri" || url.as_str().starts_with("about:"),
    }
}

/// 顶层导航是否允许留在客户端
fn navigation_allowed(url: &tauri::Url) -> bool {
    is_deepseek(url) || is_local(url)
}

/// 错误页地址（dev 走 Vite，release 走内嵌资源）
fn error_page_url() -> tauri::Url {
    if cfg!(debug_assertions) {
        tauri::Url::parse("http://localhost:5173/error.html").expect("错误页地址解析失败")
    } else {
        tauri::Url::parse("http://tauri.localhost/error.html").expect("错误页地址解析失败")
    }
}

/// 主窗口构建期钩子：导航策略 + 新窗口策略（须传入 app 以便外链调用系统浏览器）
pub fn apply_navigation_policy<'a>(
    app: &AppHandle<Wry>,
    builder: tauri::WebviewWindowBuilder<'a, Wry, AppHandle<Wry>>,
) -> tauri::WebviewWindowBuilder<'a, Wry, AppHandle<Wry>> {
    let opener_app = app.clone();
    let opener_for_popup = app.clone();
    builder
        .on_navigation(move |url| {
            // 把文件拖到页面空白处时，WebView2 的原生默认行为是导航到 file://；
            // 这类导航既不是站内内容，也不该丢给系统浏览器，直接拦掉。
            if url.scheme() == "file" {
                log::info!("已拦截本地文件导航（拖放落在非投放区）");
                return false;
            }
            if navigation_allowed(url) {
                true
            } else {
                // FR-13：非 DeepSeek 域名 → 系统默认浏览器（日志脱敏：只记域名）
                let host = url.host_str().unwrap_or("未知来源").to_string();
                let target = url.to_string();
                log::info!("外部导航交给系统浏览器: {host}");
                let h = opener_app.clone();
                std::thread::spawn(move || {
                    let _ = h.opener().open_url(target, None::<&str>);
                });
                false
            }
        })
        .on_new_window(move |url, _features| {
            // FR-13：弹窗保留在应用内的本意是第三方 OAuth（共享 Cookie，回调可完成）。
            // 但 Allow 的弹窗由 WebView2 默认机制创建，其内部导航不经过 on_navigation
            // 白名单——无条件 Allow 等于给 target=_blank 开了绕过导航策略的旁路。
            // 折中：DeepSeek 域与主流 OAuth 提供商放行，其余拒绝并交系统浏览器。
            const OAUTH_HOSTS: &[&str] = &[
                "login.microsoftonline.com",
                "accounts.google.com",
                "github.com",
                "login.weixin.qq.com",
                "open.weixin.qq.com",
                "graph.qq.com",
            ];
            let host = url.host_str().unwrap_or("").to_ascii_lowercase();
            if is_deepseek(&url) || OAUTH_HOSTS.contains(&host.as_str()) {
                log::info!("应用内打开新窗口（OAuth/弹窗）: {host}");
                tauri::webview::NewWindowResponse::Allow
            } else {
                log::info!("弹窗域名不在白名单，交系统浏览器: {host}");
                let target = url.to_string();
                let h = opener_for_popup.clone();
                std::thread::spawn(move || {
                    let _ = h.opener().open_url(target, None::<&str>);
                });
                tauri::webview::NewWindowResponse::Deny
            }
        })
}

/// 附加导航失败监听（NavigationCompleted 失败 → 本地错误页，FR-12）。
/// 归入主窗口创建流程调用：监听绑定在当前 WebView2 实例上，销毁重建后必须重新注册。
pub fn attach_failure_handler(app: &AppHandle, win: &tauri::WebviewWindow) {
    #[cfg(windows)]
    {
        use webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_WEB_ERROR_STATUS;
        use windows::core::BOOL;

        let app_handle = app.clone();
        let app_for_pf = app_handle.clone();
        // 兜底机制注册失败必须醒目：FR-12/FR-24 静默失效 = 错误页/自愈整条链路
        // 消失，不能无声吞掉 with_webview 的 Result
        if let Err(e) = win.with_webview(move |platform| unsafe {
            let core = match platform.controller().CoreWebView2() {
                Ok(c) => c,
                Err(e) => {
                    log::error!("NavigationCompleted 监听注册失败: {e}");
                    return;
                }
            };
            let core_for_cb = core.clone();

            let handler = webview2_com::NavigationCompletedEventHandler::create(
                Box::new(move |_sender, args| {
                    let Some(args) = args else { return Ok(()) };

                    let mut is_success = BOOL::default();
                    if args.IsSuccess(&mut is_success).is_err() {
                        return Ok(());
                    }
                    if is_success.as_bool() {
                        return Ok(());
                    }

                    // 当前来源（日志脱敏：只记域名）；若已停留在错误页则不再触发，避免循环。
                    // 精确判定：解析 URL 后比对 host+path，子串匹配会把带
                    // /error.html 的查询串/远端路径误判为错误页而漏兜底。
                    let mut cur_pwstr = windows::core::PWSTR::null();
                    let current = if core_for_cb.Source(&mut cur_pwstr).is_ok() {
                        take_pwstr(cur_pwstr)
                    } else {
                        String::new()
                    };
                    let on_error_page = tauri::Url::parse(&current)
                        .ok()
                        .map(|u| {
                            u.host_str() == error_page_url().host_str()
                                && u.path().ends_with("/error.html")
                        })
                        .unwrap_or(false);
                    if on_error_page {
                        return Ok(());
                    }
                    let host = tauri::Url::parse(&current)
                        .ok()
                        .and_then(|u| u.host_str().map(str::to_string))
                        .unwrap_or_else(|| "未知来源".to_string());

                    let mut status = COREWEBVIEW2_WEB_ERROR_STATUS(0);
                    let _ = args.WebErrorStatus(&mut status);
                    log::warn!("导航失败: host={host} status={}", status.0);

                    // 事件回调内不宜直接导航，延后处理。TOCTOU 校验：延迟到期时
                    // 重读当前 URL，失败潮（一次断网产生多批 NavigationCompleted）
                    // 或已落在错误页时不再重复跳转。
                    let h = app_handle.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(120));
                        if let Some(w) = h.get_webview_window("main") {
                            let cur = w.url().map(|u| u.to_string()).unwrap_or_default();
                            let on_err = tauri::Url::parse(&cur)
                                .ok()
                                .map(|u| {
                                    u.host_str() == error_page_url().host_str()
                                        && u.path().ends_with("/error.html")
                                })
                                .unwrap_or(false);
                            if on_err {
                                return;
                            }
                        }
                        show_error_page(&h, status.0);
                    });
                    Ok(())
                }),
            );

            if core.add_NavigationCompleted(&handler, std::ptr::null_mut()).is_err() {
                log::error!("add_NavigationCompleted 注册失败——错误页兜底（FR-12）将不生效");
            }

            // 渲染进程崩溃自愈（FR-24）：STATUS_ACCESS_VIOLATION 等内核级崩溃属
            // WebView2 运行时/驱动行为，壳无法阻止，但可以监听 ProcessFailed 自动重建
            // webview 恢复（Cookie 在磁盘，会话由官网 SPA 自行恢复）。
            // 单飞守卫：一次崩溃可能产生多批事件（渲染退出→无响应→帧失败），
            // 无去重会并发多个 5s 后的重建线程互相打架；恢复完成后复位。
            static RECOVERING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            let pf_handler = webview2_com::ProcessFailedEventHandler::create(
                Box::new(move |_sender, args| {
                    use std::sync::atomic::Ordering;
                    let Some(args) = args else { return Ok(()) };
                    let mut kind = webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PROCESS_FAILED_KIND(0);
                    if args.ProcessFailedKind(&mut kind).is_err() {
                        return Ok(());
                    }
                    let k = kind.0;
                    // 只处理渲染进程类失败（0=浏览器进程终止，1=渲染退出，2=渲染无响应）
                    if k != 1 && k != 2 {
                        return Ok(());
                    }
                    if RECOVERING
                        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                        .is_err()
                    {
                        log::info!("崩溃自愈进行中，忽略重复的进程失败事件（kind={k}）");
                        return Ok(());
                    }
                    log::warn!("WebView2 渲染进程失败（kind={k}），5 秒后重建 webview");
                    let h = app_for_pf.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(5));
                        // 到期重校验：若旧窗口已不在且新窗口已建好（别处完成了恢复），不重复
                        // 旧窗口已死：直接销毁残留再走重建路径
                        if let Some(w) = h.get_webview_window("main") {
                            let _ = w.destroy();
                        }
                        let result = crate::actions::create_main(&h, true);
                        RECOVERING.store(false, std::sync::atomic::Ordering::SeqCst);
                        match result {
                            Err(e) => {
                                log::error!("崩溃自愈重建主窗口失败: {e}");
                                crate::webview2_boot::alert_box(&format!(
                                    "主窗口重建失败，请重启应用。\n\n{e}"
                                ));
                            }
                            Ok(_) => log::info!("崩溃自愈：主窗口已重建"),
                        }
                    });
                    Ok(())
                }),
            );
            if core.add_ProcessFailed(&pf_handler, std::ptr::null_mut()).is_err() {
                log::error!("add_ProcessFailed 注册失败——崩溃自愈（FR-24）将不生效");
            }
        }) {
            log::error!("with_webview 失败，错误页与崩溃自愈监听（FR-12/FR-24）未注册: {e}");
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (app, win);
    }
}

/// 跳转到本地错误页（带错误码）
pub fn show_error_page(app: &AppHandle, status_code: i32) {
    let Some(win) = app.get_webview_window("main") else { return };
    let mut url = error_page_url();
    url.set_fragment(Some(&format!("status={status_code}")));
    if let Err(e) = win.navigate(url) {
        log::error!("跳转错误页失败: {e}");
    }
}

/// 取出 WebView2 分配的 UTF-16 字符串并释放（CoTaskMem）
#[cfg(windows)]
fn take_pwstr(p: windows::core::PWSTR) -> String {
    // SAFETY: `p` 来自 WebView2 COM out 参数（add_* 事件按 CoTaskMemAlloc 约定
    // 分配、保证 NUL 结尾的 UTF-16），所有权随调用移交本函数——读取长度、
    // 拷贝为 String 后必须 CoTaskMemFree 释放。若上游契约被破坏，此扫描会
    // 越界；WebView2 官方契约下安全（webview2-com 自身 take_pwstr 即此实现）。
    unsafe {
        if p.0.is_null() {
            return String::new();
        }
        let mut len = 0usize;
        while *p.0.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(p.0, len));
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0.cast()));
        s
    }
}

/// 主窗口全屏翻转（FR-23，F11）
pub fn toggle_fullscreen(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else { return };
    let on = win.is_fullscreen().unwrap_or(false);
    if let Err(e) = win.set_fullscreen(!on) {
        log::warn!("全屏切换失败: {e}");
    } else {
        log::info!("主窗口全屏: {}", if on { "退出" } else { "进入" });
    }
}

