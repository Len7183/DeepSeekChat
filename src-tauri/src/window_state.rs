// 窗口尺寸/位置记忆（FR-01）：自行读写 data\window-state.json。
// 不使用官方 window-state 插件（其默认存储位置破坏便携语义）。
// 坐标越界（多显示器拔插）时回退默认位置。

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Window};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    /// 外框位置（物理像素）
    pub x: f64,
    pub y: f64,
    /// 内区尺寸（逻辑像素，随 DPI 正确缩放）
    pub width: f64,
    pub height: f64,
    /// 保存时的 DPI 缩放（用于越界校验时换算物理尺寸）
    pub scale: f64,
    /// 是否最大化
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 1200.0,
            height: 800.0,
            scale: 1.0,
            maximized: false,
        }
    }
}

pub fn load() -> Option<WindowState> {
    let text = std::fs::read_to_string(crate::paths::window_state_file()).ok()?;
    match serde_json::from_str::<WindowState>(&text) {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!("window-state.json 解析失败: {e}");
            None
        }
    }
}

/// 从窗口捕获当前几何快照。
/// 最大化时仅更新 maximized 标记，保留上次正常态几何（还原后位置正确）。
pub fn save_snapshot(window: &Window) {
    let Ok(maximized) = window.is_maximized() else {
        return;
    };

    let mut state = load().unwrap_or_default();

    if maximized {
        state.maximized = true;
    } else {
        let Ok(pos) = window.outer_position() else { return };
        let Ok(size) = window.inner_size() else { return };
        let Ok(scale) = window.scale_factor() else { return };
        let logical = size.to_logical::<f64>(scale);
        // 最小化时系统可能给出异常坐标，跳过
        if pos.x < -32000 || pos.y < -32000 {
            return;
        }
        state.x = pos.x as f64;
        state.y = pos.y as f64;
        state.width = logical.width;
        state.height = logical.height;
        state.scale = scale;
        state.maximized = false;
    }

    if let Err(e) = crate::config::atomic_write(
        &crate::paths::window_state_file(),
        &serde_json::to_vec_pretty(&state).unwrap_or_default(),
    ) {
        log::warn!("保存窗口状态失败: {e}");
    }
}

/// 校验保存的状态是否落屏；返回应采用的初始几何（None = 用默认位置居中）
pub fn validated(state: &WindowState, app: &AppHandle) -> Option<WindowState> {
    let monitors = app.available_monitors().ok()?;
    if monitors.is_empty() {
        return None;
    }
    let scale = if state.scale >= 0.5 && state.scale <= 5.0 { state.scale } else { 1.0 };
    let w = state.width * scale;
    let h = state.height * scale;
    let (x, y) = (state.x, state.y);

    // 任一显示器与之有正面积交集即认为可见
    let on_screen = monitors.iter().any(|m| {
        let mp = m.position();
        let ms = m.size();
        let mx = mp.x as f64;
        let my = mp.y as f64;
        let mw = ms.width as f64;
        let mh = ms.height as f64;
        let ix = (x + w).min(mx + mw) - x.max(mx);
        let iy = (y + h).min(my + mh) - y.max(my);
        ix > 1.0 && iy > 1.0
    });

    if on_screen {
        Some(state.clone())
    } else {
        log::warn!("窗口状态坐标越界，回退默认位置");
        None
    }
}
