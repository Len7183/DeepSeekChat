// 数据目录策略（FR-06）：
// 1. 优先 exe 同目录 data\（绿色/安装版口径一致）
// 2. 不可写（如 Program Files）时回退 %APPDATA%\DeepSeekChat\
// 同时在首个 WebView 创建前设置 WEBVIEW2_USER_DATA_FOLDER，保证登录态随数据目录走（FR-07）。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 数据目录（进程内唯一，首次调用时解析并锁定）
pub fn data_dir() -> &'static PathBuf {
    DATA_DIR.get_or_init(resolve)
}

pub fn webview_dir() -> PathBuf {
    data_dir().join("webview")
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

pub fn config_file() -> PathBuf {
    data_dir().join("config.json")
}

pub fn window_state_file() -> PathBuf {
    data_dir().join("window-state.json")
}

/// 初始化：创建子目录并设置 WebView2 用户数据目录。
/// 必须在任何 WebView 创建之前调用。
///
/// 注意：本函数跑在 logger 安装之前（`tauri::Builder` 尚未 build），
/// 此处调用 `log::*` 不会落盘。初始化结果由 `main.rs` 在 setup 里补记，不包含路径（FR-14）。
pub fn init() {
    for sub in [webview_dir(), logs_dir()] {
        if let Err(e) = std::fs::create_dir_all(&sub) {
            eprintln!("创建数据子目录失败: {e}");
        }
    }
    // 便携登录态的关键：把 WebView2 用户数据目录指到 data\webview\
    std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", webview_dir());
}

fn resolve() -> PathBuf {
    // 1) exe 同目录 data\
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_parent) = exe.parent() {
            let candidate = exe_parent.join("data");
            if is_writable_dir(&candidate) {
                return candidate;
            }
            log::warn!("数据目录不可写，回退 %APPDATA%");
        }
    }
    // 2) 回退 %APPDATA%\DeepSeekChat\
    let fallback = std::env::var("APPDATA")
        .map(|p| PathBuf::from(p).join("DeepSeekChat"))
        .unwrap_or_else(|_| PathBuf::from("data"));
    let _ = std::fs::create_dir_all(&fallback);
    fallback
}

/// 判断目录可创建且可写（写探针文件验证）
fn is_writable_dir(dir: &Path) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".write-test");
    match std::fs::write(&probe, b"ok") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}
