// IPC 命令：设置读写（FR-05）、数据目录与项目主页（FR-06）、重试导航（FR-12）、托盘菜单动作（FR-03）

use serde::Deserialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_opener::OpenerExt as _;

use crate::{actions, AppState};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigPatch {
    pub close_action: Option<crate::config::CloseAction>,
    pub low_memory_mode: Option<bool>,
    pub autostart: Option<bool>,
}

/// 托盘菜单动作（menu.html 的 data-action）。枚举而非裸字符串：
/// 未知动作在反序列化时即被拒绝，而不是漏进运行时匹配
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MenuAction {
    Hide,
    Open,
    Settings,
    Quit,
}

#[tauri::command]
pub fn get_config(app: AppHandle) -> Result<crate::config::Config, String> {
    // 锁毒化返回捏造的默认值会让设置页显示假状态（且与 set_config 的
    // 报错口径不一致）——改为传播错误，UI 弹 toast 告知
    app.state::<AppState>()
        .config
        .lock()
        .map(|c| c.clone())
        .map_err(|e| format!("读取配置失败（锁已中毒）: {e}"))
}

/// 配置保存串行锁：并发的 set_config 若共用同一个 .tmp 文件会互相覆盖，
/// 这里把「改内存 + 落盘」整段串行化（仅设置窗口会调用，代价可忽略）。
static CONFIG_SAVE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 部分更新设置并落盘；autostart 同步注册表（Run 键带 --silent-start）。
/// 顺序：注册表 → 落盘 → 发布内存。任一步失败都让两处存储与内存态保持一致。
#[tauri::command]
pub fn set_config(app: AppHandle, patch: ConfigPatch) -> Result<crate::config::Config, String> {
    let _guard = CONFIG_SAVE_LOCK
        .lock()
        .map_err(|_| "配置保存锁已中毒".to_string())?;

    // 0) 候选配置先行构建：若 config 锁已中毒，此时注册表尚未被改动，
    //    直接报错返回不产生任何存储分歧（此前注册表先写、此处才失败会留下
    //    「注册表已改而磁盘/内存未改」且无回滚的窗口）
    let candidate = {
        let state = app.state::<AppState>();
        let config = state.config.lock().map_err(|e| e.to_string())?;
        let mut candidate = config.clone();
        if let Some(v) = patch.close_action {
            candidate.close_action = v;
        }
        if let Some(v) = patch.low_memory_mode {
            candidate.low_memory_mode = v;
        }
        if let Some(v) = patch.autostart {
            candidate.autostart = v;
        }
        candidate
    }; // 锁在此释放：落盘 IO 不阻塞 close_is_hide / browser_args_for 等取锁线程

    // 1) 自启：注册表为准（先读真实状态，避免幂等重复写），失败则整体返回，内存态不动。
    //    读取失败必须传播：若以 false 兜底且目标是关，会跳过 disable 却把
    //    autostart=false 发布到磁盘/内存——注册表仍开启而三处存储不一致
    let autostart_prev = if let Some(v) = patch.autostart {
        let autolaunch = app.autolaunch();
        let current = autolaunch
            .is_enabled()
            .map_err(|e| format!("读取开机自启状态失败: {e}"))?;
        if current != v {
            if v {
                autolaunch.enable().map_err(|e| format!("开启自启失败: {e}"))?;
            } else {
                autolaunch.disable().map_err(|e| format!("关闭自启失败: {e}"))?;
            }
            log::info!("开机自启已{}", if v { "开启" } else { "关闭" });
        }
        Some(current)
    } else {
        None
    };

    // 3) 落盘失败：回滚注册表，内存态保持原值（UI 回弹到真实状态）
    if let Err(e) = crate::config::save(&candidate) {
        if let Some(prev) = autostart_prev {
            let autolaunch = app.autolaunch();
            let rollback = if prev { autolaunch.enable() } else { autolaunch.disable() };
            match rollback {
                Ok(()) => log::warn!("配置保存失败，已回滚开机自启状态: {e}"),
                Err(re) => {
                    log::error!("配置保存失败且自启回滚失败: {e} / {re}");
                    // 尽力重读注册表真实状态并写回内存/磁盘，消除「报错说可能
                    // 已改动、实际状态永远无人知晓」的悬空分歧
                    let actual = app
                        .autolaunch()
                        .is_enabled()
                        .unwrap_or(candidate.autostart);
                    let mut reconciled = candidate.clone();
                    reconciled.autostart = actual;
                    if crate::config::save(&reconciled).is_ok() {
                        let state = app.state::<AppState>();
                        let mut guard = match state.config.lock() {
                            Ok(g) => g,
                            Err(_) => return Err(format!(
                                "保存失败: {e}；开机自启实际状态：{}（内存同步失败）",
                                if actual { "开启" } else { "关闭" }
                            )),
                        };
                        *guard = reconciled.clone();
                    }
                    return Err(format!(
                        "保存失败: {e}；开机自启实际状态：{}（已尽量同步）",
                        if actual { "开启" } else { "关闭" }
                    ));
                }
            }
        }
        return Err(e);
    }

    // 4) 落盘成功后才发布到内存。锁毒化时不再静默返回：磁盘已是新值，
    //    内存却保持旧值会让 UI 显示与重启后实际配置不一致——显式报错提示
    let published = {
        let state = app.state::<AppState>();
        let mut config = state
            .config
            .lock()
            .map_err(|e| format!("配置已保存但发布到内存失败（锁已中毒）: {e}"))?;
        *config = candidate;
        config.clone()
    };
    Ok(published)
}

/// 当前数据目录（设置窗口底部展示，FR-06）
#[tauri::command]
pub fn get_data_dir() -> String {
    crate::paths::data_dir().display().to_string()
}

/// 打开数据目录（资源管理器）
#[tauri::command]
pub fn open_data_dir(app: AppHandle) -> Result<(), String> {
    let dir = crate::paths::data_dir();
    app.opener()
        .open_path(dir.display().to_string(), None::<&str>)
        .map_err(|e| format!("打开数据目录失败: {e}"))?;
    log::info!("已打开数据目录");
    Ok(())
}

/// 打开项目主页（设置窗口底部「关于DeepSeekChat」，系统浏览器）。
/// 日志脱敏：只记动作，不记完整 URL。
#[tauri::command]
pub fn open_project_home(app: AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(crate::PROJECT_HOME_URL, None::<&str>)
        .map_err(|e| format!("打开项目主页失败: {e}"))?;
    log::info!("已打开项目主页");
    Ok(())
}

/// 从错误页重试回到官网（FR-12）。
/// async：避免在主线程 webview 回调内执行窗口操作（WebView2 重入限制）。
#[tauri::command]
pub async fn retry_navigation(app: AppHandle) -> Result<(), String> {
    let Some(win) = app.get_webview_window("main") else {
        return Err("主窗口不存在".into());
    };
    let url = tauri::Url::parse(crate::DEEPSEEK_URL).map_err(|e| e.to_string())?;
    log::info!("错误页重试：重新导航官网");
    win.navigate(url).map_err(|e| format!("导航失败: {e}"))
}

/// 托盘菜单动作（来自 menu.html）。
/// async：设置窗在此命令内懒创建——同步命令运行在主线程 webview 回调里，
/// 此时创建 WebView2 控制器会报 ERROR_INVALID_STATE（与托盘回调同类问题），
/// async 命令在工作线程执行、由事件循环干净地完成创建。
#[tauri::command]
pub async fn menu_action(app: AppHandle, action: MenuAction) -> Result<(), String> {
    // 点击后先收起菜单窗（撤防弹层守卫 + 隐藏）
    crate::tray::hide_menu(&app);
    match action {
        MenuAction::Hide => Ok(()), // Esc 收起，无其他动作
        MenuAction::Open => {
            actions::show_main(&app);
            Ok(())
        }
        MenuAction::Settings => {
            let result = actions::open_settings(&app);
            if let Err(e) = &result {
                log::error!("打开设置窗口失败: {e}");
            }
            result.map_err(|e| e.to_string())
        }
        MenuAction::Quit => {
            log::info!("托盘菜单退出");
            app.exit(0);
            Ok(())
        }
    }
}
