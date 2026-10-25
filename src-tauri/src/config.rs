// 应用设置（FR-05）：仅四项，读写 data\config.json。
// 不使用任何默认存储在 %APPDATA% 的插件，保证便携语义一致。

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloseAction {
    /// 关闭按钮 → 隐藏到托盘（默认）
    Hide,
    /// 关闭按钮 → 直接退出
    Exit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// 关闭按钮行为
    pub close_action: CloseAction,
    /// 省内存模式（禁用 GPU），改动后需重启生效
    pub low_memory_mode: bool,
    /// 开机自启
    pub autostart: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            close_action: CloseAction::Hide,
            low_memory_mode: false,
            autostart: false,
        }
    }
}

/// 读取配置；文件缺失或损坏时返回默认值（不覆盖磁盘，等下次保存时写入）
pub fn load() -> Config {
    let path = crate::paths::config_file();
    match std::fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Config>(&text) {
            Ok(cfg) => cfg,
            Err(e) => {
                log::warn!("config.json 解析失败，使用默认值: {e}");
                Config::default()
            }
        },
        Err(_) => Config::default(),
    }
}

/// 原子保存（先写临时文件再重命名，避免异常退出损坏）
pub fn save(cfg: &Config) -> Result<(), String> {
    let path = crate::paths::config_file();
    atomic_write(&path, &serde_json::to_vec_pretty(cfg).map_err(|e| e.to_string())?)?;
    log::info!("配置已保存");
    Ok(())
}

/// 原子写入工具：同目录写 .tmp 后 rename 覆盖目标。
/// `std::fs::rename` 在 Windows 上走 `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`，
/// 本就能覆盖已存在的目标——先 remove 再 rename 反而把写操作劈成两步，
/// 在两步之间崩溃会直接丢掉旧文件（实测：rename 覆盖返回 ok=true）。
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("写入临时配置文件失败: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("重命名失败: {e}"))?;
    Ok(())
}
