// 内存工程（FR-09，验收级）：
// 指标：前台（可见聚焦、无交互空闲 5 分钟）≤180MB，默认 GPU 开启即须达标；
//       托盘挂起（隐藏+挂起+修剪后稳定 60 秒）≤80MB。
// 手段：
//   1) 事件驱动修剪：窗口失焦 / 空闲阈值 / 隐藏后 / 销毁后，对进程树内全部进程
//      （外壳自身 + 全部 WebView2 子进程）执行 EmptyWorkingSet（回收专用工作集）；
//   2) 隐藏后 TrySuspend 真挂起（WebView2 内核暂停，渲染/定时器全停），可见时自动恢复；
//   3) 长时间隐藏（>30 分钟）销毁 webview，唤起时重建（Cookie 在磁盘，登录不丢）。

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};

/// 空闲修剪阈值：无用户交互 5 分钟
const IDLE_PRUNE_SECS: u64 = 300;
/// 隐藏后执行挂起+修剪的延迟（等渲染器安静）
const HIDE_SUSPEND_DELAY_MS: u64 = 1500;
/// 长挂起阈值：隐藏超过 30 分钟销毁 webview
const DESTROY_AFTER_HIDE_SECS: u64 = 1800;

#[derive(Default)]
pub struct MemoryState {
    /// 最后一次用户活跃时刻（主窗口页面心跳上报）
    pub last_activity: Mutex<Option<Instant>>,
    /// 主窗口隐藏时刻（None = 可见或未创建）
    pub hidden_since: Mutex<Option<Instant>>,
    /// webview 是否已因长挂起被销毁
    pub destroyed: Mutex<bool>,
    /// 本段空闲是否已修剪过（用户重新活跃后重新布防）
    pub idle_pruned: Mutex<bool>,
    /// 隐藏事件代数：每次 hide 递增，唤醒的挂起/销毁线程用它校验
    /// 「自己观察到的隐藏段」是否仍是当前状态段（防止 hide→show→hide
    /// 快速切换时多个休眠线程同时醒来各自行动）
    pub hide_generation: std::sync::atomic::AtomicU64,
}

/// 心跳（注入脚本 dschat://heartbeat）
pub fn note_activity(app: &AppHandle) {
    if let Some(state) = app.try_state::<MemoryState>() {
        // 锁毒化只意味着某次持有方 panic，数据本身无害——统一恢复而非放弃
        if let Ok(mut t) = state.last_activity.lock() {
            *t = Some(Instant::now());
        }
        // 用户重新活跃：下一段空闲重新布防修剪
        if let Ok(mut done) = state.idle_pruned.lock() {
            *done = false;
        }
    }
}

/// 距最后用户活跃的秒数；**从未收到心跳返回 0（视为活跃）**。
/// 心跳缺失是信号故障而非「用户挂机」——把缺失当最大空闲会在注入失败时
/// 对正在使用的应用连续修剪（fail-open 到危险动作），保守取 0 更安全。
fn idle_secs(app: &AppHandle) -> u64 {
    if let Some(state) = app.try_state::<MemoryState>() {
        if let Ok(t) = state.last_activity.lock() {
            return match *t {
                Some(t) => t.elapsed().as_secs(),
                None => 0,
            };
        }
    }
    0
}

// ---------------------------------------------------------------------------
// 工作集修剪
// ---------------------------------------------------------------------------

/// 修剪本应用进程树（DeepSeekChat.exe 及其全部 WebView2 子进程）的工作集。
/// 返回修剪的进程数。
#[cfg(windows)]
fn prune_tree() -> usize {
    match unsafe { trim_working_sets() } {
        Ok(n) => n,
        Err(e) => {
            log::warn!("修剪进程树失败: {e}");
            0
        }
    }
}

#[cfg(not(windows))]
fn prune_tree() -> usize {
    0
}

#[cfg(windows)]
unsafe fn trim_working_sets() -> Result<usize, String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SET_QUOTA,
    };

    let my_pid = GetCurrentProcessId();

    // 枚举系统全部进程 (pid, ppid)
    let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).map_err(|e| e.to_string())?;
    let mut all: Vec<(u32, u32)> = Vec::new();
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    if Process32FirstW(snapshot, &mut entry).is_ok() {
        loop {
            all.push((entry.th32ProcessID, entry.th32ParentProcessID));
            if Process32NextW(snapshot, &mut entry).is_err() {
                break;
            }
        }
    }
    let _ = CloseHandle(snapshot);

    // 闭包求出本进程全部后代（含自身）
    let mut tree: HashSet<u32> = HashSet::new();
    tree.insert(my_pid);
    loop {
        let mut grew = false;
        for (pid, ppid) in &all {
            if tree.contains(ppid) && tree.insert(*pid) {
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }

    let mut trimmed = 0usize;
    let mut failed = 0usize;
    for pid in tree {
        if pid == 0 || pid == 4 {
            continue; // 空闲进程与 System
        }
        // 打不开的进程（权限不足 / 已退出）直接跳过
        if let Ok(handle) = OpenProcess(PROCESS_SET_QUOTA | PROCESS_QUERY_INFORMATION, false, pid) {
            // 只把真正成功的修剪计入 trimmed：日志口径 = 实际回收的工作集数，
            // 失败单独计数（整体权限失败时日志能看出来，而不是虚报成功）
            if EmptyWorkingSet(handle).is_ok() {
                trimmed += 1;
            } else {
                failed += 1;
            }
            let _ = CloseHandle(handle);
        }
    }
    if failed > 0 {
        log::debug!("工作集修剪：{failed} 个进程调用失败（权限/状态）");
    }
    Ok(trimmed)
}

// ---------------------------------------------------------------------------
// 触发时机
// ---------------------------------------------------------------------------

/// 窗口失焦触发修剪
pub fn prune_on_focus_loss(_app: &AppHandle) {
    prune_and_log("失焦");
}

/// 空闲阈值触发（idle 线程调用）。
/// 一段空闲只修剪一次：工作集修剪后页面按需换页，反复修剪只是重复枚举进程；
/// 用户重新活跃（心跳）后 `idle_pruned` 复位，下一段空闲重新布防。
fn prune_on_idle(app: &AppHandle) -> bool {
    if idle_secs(app) < IDLE_PRUNE_SECS {
        return false;
    }
    let Some(state) = app.try_state::<MemoryState>() else {
        return false;
    };
    {
        let mut done = match state.idle_pruned.lock() {
            Ok(g) => g,
            // 毒化恢复：时间戳数据无害，放弃会让空闲修剪从此静默失效
            Err(e) => e.into_inner(),
        };
        if *done {
            return false;
        }
        *done = true;
    }
    prune_and_log("空闲");
    true
}

fn prune_and_log(reason: &str) {
    let n = prune_tree();
    if n > 0 {
        log::info!("工作集修剪完成（{reason}，{n} 个进程）");
    }
}

/// 主窗口已隐藏：延迟挂起 + 修剪（FR-09 托盘挂起路径）。
/// 代数校验：休眠线程醒来后先确认 hide_generation 未变（中途 show 过会递增），
/// 且窗口此刻仍隐藏——两个条件都满足才执行挂起/修剪，消除堆积线程竞态。
pub fn on_main_hidden(app: &AppHandle) {
    let generation = if let Some(state) = app.try_state::<MemoryState>() {
        if let Ok(mut t) = state.hidden_since.lock() {
            *t = Some(Instant::now());
        }
        state.hide_generation.fetch_add(1, Ordering::Relaxed) + 1
    } else {
        return;
    };

    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(HIDE_SUSPEND_DELAY_MS));
        // 代数已变（中间发生过 show/hide）→ 本线程的观察已过期，让位给最新线程
        if current_generation(&handle) != generation {
            return;
        }
        let Some(win) = handle.get_webview_window("main") else { return };
        if win.is_visible().unwrap_or(true) {
            return;
        }
        match suspend_main(&handle) {
            Ok(()) => log::info!("WebView2 已挂起（TrySuspend）"),
            Err(e) => log::warn!("TrySuspend 不可用，降级纯修剪路径: {e}"),
        }
        prune_and_log("隐藏后");
    });
}

/// 当前隐藏代数（供休眠线程过期校验）
fn current_generation(app: &AppHandle) -> u64 {
    app.try_state::<MemoryState>()
        .map(|s| s.hide_generation.load(Ordering::Relaxed))
        .unwrap_or(u64::MAX)
}

/// 主窗口重新显示：清状态（WebView2 可见时自动恢复，无需显式 Resume）。
/// 递增代数让仍在休眠的旧挂起/销毁线程全部过期。
pub fn on_main_shown(app: &AppHandle) {
    if let Some(state) = app.try_state::<MemoryState>() {
        if let Ok(mut t) = state.hidden_since.lock() {
            *t = None;
        }
        if let Ok(mut d) = state.destroyed.lock() {
            *d = false;
        }
        state.hide_generation.fetch_add(1, Ordering::Relaxed);
    }
}

/// 经平台通道调用 ICoreWebView2::TrySuspend（FR-09）。
/// TrySuspend 为异步完成回调；webview2-com 的 `wait_for_async_operation` 内部已用
/// mpsc + 消息泵同步等待，本函数只负责把结果从主线程回传，并用 `recv_timeout` 兜底。
#[cfg(windows)]
fn suspend_main(app: &AppHandle) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{self, RecvTimeoutError};
    use std::sync::Arc;

    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2_3;
    use webview2_com::TrySuspendCompletedHandler;
    use windows::core::Interface;

    let Some(win) = app.get_webview_window("main") else {
        return Err("主窗口不存在".into());
    };

    let (tx, rx) = mpsc::channel::<(Result<(), String>, bool)>();
    // is_success：调用成功不等于真的挂上了（官网长连接会被拒绝）
    let suspended = Arc::new(AtomicBool::new(false));
    let suspended_cb = suspended.clone();

    win.with_webview(move |platform| {
        let outcome = (|| -> Result<(), String> {
            let core = unsafe { platform.controller().CoreWebView2() }
                .map_err(|e| format!("CoreWebView2 获取失败: {e}"))?;
            let core3: ICoreWebView2_3 =
                core.cast().map_err(|e| format!("接口转换失败: {e}"))?;

            unsafe {
                TrySuspendCompletedHandler::wait_for_async_operation(
                    Box::new(move |handler| {
                        core3.TrySuspend(&handler)
                            .map_err(webview2_com::Error::WindowsError)
                    }),
                    Box::new(move |error_code, is_success| {
                        suspended_cb.store(is_success, Ordering::Relaxed);
                        error_code
                    }),
                )
                .map_err(|e| format!("TrySuspend 等待失败: {e}"))?;
            }
            Ok(())
        })();
        let _ = tx.send((outcome, suspended.load(Ordering::Relaxed)));
    })
    .map_err(|e| format!("with_webview 调度失败: {e}"))?;

    // with_webview 是异步调度到主线程，等待其回传。
    // 超时只结束等待，不取消已排队的调用；调用自身仍会先核对窗口是否隐藏。
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok((Ok(()), true)) => Ok(()),
        Ok((Ok(()), false)) => Err("TrySuspend 未真正挂起（is_success=false）".into()),
        Ok((Err(e), _)) => Err(e),
        Err(RecvTimeoutError::Timeout) => Err("with_webview 超时未执行".into()),
        Err(RecvTimeoutError::Disconnected) => Err("with_webview 回调未回传结果".into()),
    }
}

// ---------------------------------------------------------------------------
// 空闲监视线程
// ---------------------------------------------------------------------------

/// 启动空闲监视：每 60 秒检查空闲修剪与长挂起销毁（FR-09）
pub fn spawn_idle_monitor(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(60));

        let Some(win) = app.get_webview_window("main") else { continue };

        // 隐藏态：处理长挂起销毁
        if !win.is_visible().unwrap_or(true) {
            let (hidden_for, already) = match app.try_state::<MemoryState>() {
                Some(state) => {
                    let hidden_for = state
                        .hidden_since
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .map(|t| t.elapsed().as_secs())
                        .unwrap_or(0);
                    let already = *state.destroyed.lock().unwrap_or_else(|e| e.into_inner());
                    (hidden_for, already)
                }
                None => (0, false),
            };
            if hidden_for >= DESTROY_AFTER_HIDE_SECS && !already {
                if let Some(state) = app.try_state::<MemoryState>() {
                    if let Ok(mut d) = state.destroyed.lock() {
                        *d = true;
                    }
                }
                // 销毁前终验：is_visible 与 destroyed 标记之间窗口可能刚被唤起
                // （on_main_shown 会清标记并递增代数，但存在窗口）——
                // 重新核对可见性，可见即跳过本次销毁（标记回滚）
                if win.is_visible().unwrap_or(false) {
                    if let Some(state) = app.try_state::<MemoryState>() {
                        if let Ok(mut d) = state.destroyed.lock() {
                            *d = false;
                        }
                    }
                    continue;
                }
                let _ = win.destroy();
                log::info!("隐藏超过 {DESTROY_AFTER_HIDE_SECS}s，已销毁 webview（唤起时重建，登录态在磁盘）");
                // 销毁后再修剪一次残余进程
                prune_and_log("销毁后");
            }
            continue;
        }

        // 可见态：空闲修剪
        let _ = prune_on_idle(&app);
    });
}
