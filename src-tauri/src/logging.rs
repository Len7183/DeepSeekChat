// 滚动日志（FR-14）：写入数据目录 logs\，按大小轮转，磁盘占用上限约 5MB。
// 注意脱敏：任何日志调用不得包含 Cookie、请求头、带 token 的完整 URL。

use tauri_plugin_log::{RotationStrategy, Target, TargetKind, TimezoneStrategy};

/// 保留的轮转文件数（1MB × 4 ≈ 5MB 上限）
const KEEP_FILES: usize = 4;
/// 单文件上限 1MB
const MAX_FILE_SIZE: u128 = 1_000_000;

pub fn plugin() -> tauri_plugin_log::Builder {
    let dir = crate::paths::logs_dir();
    #[cfg_attr(not(debug_assertions), allow(unused_mut))]
    let mut targets = vec![Target::new(TargetKind::Folder {
        path: dir.clone(),
        file_name: Some("deepseekchat".into()),
    })];
    // 开发构建额外输出到控制台，便于调试
    #[cfg(debug_assertions)]
    {
        targets.push(Target::new(TargetKind::Stdout));
    }
    #[cfg_attr(not(debug_assertions), allow(unused_mut))]
    let mut builder = tauri_plugin_log::Builder::new()
        .targets(targets)
        .level(log::LevelFilter::Info)
        .max_file_size(MAX_FILE_SIZE)
        // 默认是 UTC，比本机时间差整小时数，排障时对不上操作时间
        .timezone_strategy(TimezoneStrategy::UseLocal)
        .rotation_strategy(RotationStrategy::KeepAll);

    #[cfg(debug_assertions)]
    {
        builder = builder.level(log::LevelFilter::Debug);
    }

    builder
}

/// 启动时清理旧轮转日志：仅保留最新 KEEP_FILES 个。
/// 跳过当前正在写的活跃文件（logger 已在 plugin setup 阶段打开
/// `deepseekchat.log`，删除打开中的文件后插件会继续写向已解除链接的
/// 句柄，日志静默消失——Windows 的 FILE_SHARE_DELETE 让 remove 成功，
/// 但新日志再也写不进任何可见文件）
pub fn prune_old_logs() {
    let dir = crate::paths::logs_dir();
    let mut files: Vec<_> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                // 活跃文件（无后缀数字的 deepseekchat.log）由 logger 持有打开句柄，
                // 删除它会让后续日志写向已解除链接的句柄——永远跳过
                !(name == "deepseekchat.log")
                    && name.starts_with("deepseekchat")
                    && name.ends_with(".log")
            })
            .filter_map(|e| {
                let meta = e.metadata().ok()?;
                let modified = meta.modified().ok()?;
                Some((e.path(), modified))
            })
            .collect(),
        Err(e) => {
            // 目录不可读 = 磁盘上限静默失效，必须留痕（哪怕落不进文件，stderr 可见）
            eprintln!("读取日志目录失败，无法清理旧日志: {e}");
            return;
        }
    };
    if files.len() <= KEEP_FILES {
        return;
    }
    files.sort_by_key(|(_, modified)| *modified);
    files.reverse(); // 新的在前
    let mut removed = 0usize;
    for (path, _) in files.into_iter().skip(KEEP_FILES) {
        match std::fs::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(e) => log::warn!("清理旧日志失败 {}: {e}", path.display()),
        }
    }
    // 单个失败不阻断：部分清理仍执行（I/O 竞态下个别文件可能恰好被占用）
    // 空闲监视线程定期复扫：长运行会话（自启常驻）持续产生轮转文件，
    // 仅启动清理一次会让 ~5MB 磁盘上限在实际使用中无界增长
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
        prune_old_logs();
    });
    log::info!("日志目录已清理（保留 {KEEP_FILES} 个，删除 {removed} 个）");
}
