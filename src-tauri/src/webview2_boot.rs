// WebView2 运行时检测与引导（FR-11）：
// 启动时检测 Evergreen Runtime；缺失时原生对话框征得同意 → 释放内嵌的微软官方
// Bootstrapper（约 2MB，随 exe 分发）在线安装；无网络时给出官方下载页与手动指引。

use std::path::PathBuf;

/// EdgeUpdate 客户端 GUID（微软官方检测口径）
const WEBVIEW2_CLIENT_ID: &str = "{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
const OFFICIAL_DOWNLOAD_PAGE: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

/// 内嵌的微软官方 Bootstrapper
const BOOTSTRAPPER: &[u8] = include_bytes!("../resources/MicrosoftEdgeWebview2Setup.exe");

/// 引导结果，供 setup 阶段补记日志（FR-14）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeOutcome {
    /// 本机已有 Runtime，未做任何引导
    AlreadyPresent,
    /// 本机缺失，用户同意后由内嵌 Bootstrapper 装好
    InstalledByBootstrap,
}

/// 确保 WebView2 Runtime 可用；不可用且用户未完成安装时退出进程。
/// 必须在创建任何 WebView 之前调用。
///
/// 注意：本函数跑在 logger 安装之前（`tauri::Builder` 尚未 build），此处 `log::*`
/// 不会落盘，故本模块不写日志——过程与失败原因全部由原生对话框当面传达，
/// 「曾引导安装过」这一事实经 [`RuntimeOutcome`] 交给 `main.rs` 在 setup 里补记。
pub fn ensure_runtime() -> RuntimeOutcome {
    if runtime_installed() {
        return RuntimeOutcome::AlreadyPresent;
    }

    let online = is_online();
    if online {
        let yes = message_box(
            "DeepSeekChat 需要 Microsoft WebView2 Runtime\n\n本机未检测到 WebView2 Runtime，是否立即下载并安装？\n（使用微软官方引导器，约 2MB，需联网）",
            true,
        );
        if yes {
            if install_with_bootstrapper() {
                if runtime_installed() {
                    return RuntimeOutcome::InstalledByBootstrap;
                }
                message_box(
                    "安装程序已执行，但仍未检测到 WebView2 Runtime。\n请重启应用；若仍失败请手动安装。",
                    false,
                );
            } else {
                message_box("安装失败，请通过官方页面手动安装。", false);
                open_download_page();
            }
            std::process::exit(1);
        }
        message_box("未安装 WebView2 Runtime，应用无法启动。", false);
        std::process::exit(1);
    } else {
        // 无网络：提供官方下载页链接与手动安装指引
        let text = format!(
            "DeepSeekChat 需要 Microsoft WebView2 Runtime\n\n本机未检测到 WebView2 Runtime，且当前无网络连接。\n\n请联网后手动下载安装 WebView2 Runtime：\n{}\n\n安装完成后重新启动本应用即可。",
            OFFICIAL_DOWNLOAD_PAGE
        );
        let _ = message_box(&text, true);
        open_download_page();
        std::process::exit(1);
    }
}

/// 检测 Evergreen Runtime（微软官方口径：EdgeUpdate Clients 键 pv 值）
fn runtime_installed() -> bool {
    #[cfg(windows)]
    {
        unsafe { query_runtime_registry().is_some() }
    }
    #[cfg(not(windows))]
    {
        true
    }
}

#[cfg(windows)]
unsafe fn query_runtime_registry() -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegGetValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ,
    };

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let candidates: [(HKEY, &str); 3] = [
        (HKEY_LOCAL_MACHINE, "SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients"),
        (HKEY_LOCAL_MACHINE, "SOFTWARE\\Microsoft\\EdgeUpdate\\Clients"),
        (HKEY_CURRENT_USER, "SOFTWARE\\Microsoft\\EdgeUpdate\\Clients"),
    ];

    for (root, base) in candidates {
        let subkey = format!("{base}\\{WEBVIEW2_CLIENT_ID}");
        let subkey_w = w(&subkey);
        let value_w = w("pv");
        let mut buf = [0u16; 128];
        let mut len = (buf.len() * std::mem::size_of::<u16>()) as u32;
        let status = RegGetValueW(
            root,
            PCWSTR(subkey_w.as_ptr()),
            PCWSTR(value_w.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        );
        if status.is_err() {
            continue;
        }
        let chars = (len as usize) / std::mem::size_of::<u16>();
        let pv = String::from_utf16_lossy(&buf[..chars.min(buf.len())])
            .trim_end_matches('\0')
            .to_string();
        if !pv.is_empty() && pv != "0.0.0.0" {
            return Some(pv);
        }
    }
    None
}

/// 简单联网检查：解析微软更新域名
fn is_online() -> bool {
    use std::net::ToSocketAddrs;
    ("go.microsoft.com", 443)
        .to_socket_addrs()
        .map(|mut it| it.next().is_some())
        .unwrap_or(false)
}

/// 释放内嵌 Bootstrapper 到 %TEMP% 并静默安装（等待完成）。
/// 失败原因不回传日志——本模块运行在 logger 之前（见 [`ensure_runtime`]），
/// 调用方以对话框告知用户。
fn install_with_bootstrapper() -> bool {
    let Some(temp_dir) = std::env::var_os("TEMP").map(PathBuf::from) else {
        return false;
    };
    let setup_path = temp_dir.join("MicrosoftEdgeWebview2Setup.exe");
    if std::fs::write(&setup_path, BOOTSTRAPPER).is_err() {
        return false;
    }

    #[cfg(windows)]
    {
        let exit_code = unsafe { run_setup_and_wait(&setup_path) };
        exit_code == Some(0)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
unsafe fn run_setup_and_wait(setup: &std::path::Path) -> Option<i32> {
    use windows::core::{PWSTR, PCWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Threading::{
        CreateProcessW, GetExitCodeProcess, WaitForSingleObject, CREATE_NO_WINDOW,
        INFINITE, PROCESS_INFORMATION, STARTUPINFOW,
    };

    let cmdline = format!("\"{}\" /silent /install", setup.display());
    let mut cmdline_w: Vec<u16> = cmdline.encode_utf16().chain(std::iter::once(0)).collect();
    let si = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut pi = PROCESS_INFORMATION::default();

    let ok = CreateProcessW(
        PCWSTR::null(),
        Some(PWSTR(cmdline_w.as_mut_ptr())),
        None,
        None,
        false,
        CREATE_NO_WINDOW,
        None,
        None,
        &si,
        &mut pi,
    );
    if ok.is_err() {
        return None; // 失败原因由调用方的对话框告知（本模块早于 logger，见 ensure_runtime）
    }

    let _ = WaitForSingleObject(HANDLE(pi.hProcess.0), INFINITE);
    let mut code: u32 = 0;
    let _ = GetExitCodeProcess(HANDLE(pi.hProcess.0), &mut code);
    let _ = CloseHandle(HANDLE(pi.hProcess.0));
    let _ = CloseHandle(HANDLE(pi.hThread.0));
    Some(code as i32)
}

/// 运行期原生提示框（OK）：webview 不可用或不可信路径上的用户可感知反馈，
/// 复用启动期消息框样式（标题固定 DeepSeekChat）
pub fn alert_box(text: &str) {
    let _ = message_box(text, false);
}

/// 原生消息框（WebView2 尚未就绪，不能用网页 UI）
fn message_box(text: &str, yes_no: bool) -> bool {
    #[cfg(windows)]
    unsafe {
        use windows::core::HSTRING;
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, IDYES, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_YESNO,
        };
        let flags = if yes_no { MB_YESNO | MB_ICONWARNING } else { MB_OK | MB_ICONWARNING }
            | MB_SETFOREGROUND;
        let result = MessageBoxW(
            None,
            &HSTRING::from(text),
            &HSTRING::from("DeepSeekChat"),
            flags,
        );
        result == IDYES
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        false
    }
}

fn open_download_page() {
    #[cfg(windows)]
    unsafe {
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let url = format!("{}\0", OFFICIAL_DOWNLOAD_PAGE);
        let url_w: Vec<u16> = url.encode_utf16().collect();
        let verb_w: Vec<u16> = "open\0".encode_utf16().collect();
        ShellExecuteW(
            None,
            PCWSTR(verb_w.as_ptr()),
            PCWSTR(url_w.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        );
    }
}
