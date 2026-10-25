# qa-memory.ps1 - FR-09 formal acceptance sequence driver
# 1. focus main window, idle 5.5 min (page heartbeat silent) -> measure foreground
# 2. send WM_CLOSE (hide to tray) -> wait suspend+prune -> stabilize 60s -> measure tray
# 3. relaunch exe (single-instance shows window) -> report window visible
# 路径按脚本自身位置解析（仓库可移动），也可用 -ExePath 显式指定
# 退出码：0=达标 2=度量失败 3=超预算（可被 CI/自动化消费）
param(
    [string]$ExePath
)
$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $ExePath) {
    $ExePath = Join-Path $RepoRoot 'src-tauri\target\release\deepseekchat.exe'
}
# 同会话重复运行不再因 type 已存在而中断（验收常需要反复跑）
if (-not ('QWin' -as [type])) {
    Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class QWin {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
    public static IntPtr Found = IntPtr.Zero;
    public static bool Callback(IntPtr h, IntPtr l) {
        uint p; GetWindowThreadProcessId(h, out p);
        if (p == QWin.TargetPid && IsWindowVisible(h)) {
            var sb = new StringBuilder(256); GetWindowText(h, sb, 256);
            if (sb.ToString() == "DeepSeekChat") { QWin.Found = h; return false; }
        }
        return true;
    }
    public static uint TargetPid;
    public static IntPtr FindMainByPid(uint pid) {
        Found = IntPtr.Zero; TargetPid = pid;
        EnumWindows(Callback, IntPtr.Zero);
        return Found;
    }
}
"@
}

function Get-MainHwnd {
    # 主窗口所属进程才是目标：单实例转发进程存续极短，取「有主窗口者」
    # 而不是任意第一个 deepseekchat 进程
    foreach ($proc in (Get-Process deepseekchat -ErrorAction SilentlyContinue)) {
        $h = [QWin]::FindMainByPid([uint32]$proc.Id)
        if ($h -ne [IntPtr]::Zero) { return $h }
    }
    return [IntPtr]::Zero
}

function Send-WmClose {
    $h = Get-MainHwnd
    if ($h -eq [IntPtr]::Zero) { throw "main window not found" }
    # 投递失败必须显式失败：静默继续会把「窗口根本没隐藏」的无效数据
    # 当成托盘指标记录
    if (-not [QWin]::PostMessage($h, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)) {
        throw "WM_CLOSE 投递失败（窗口可能已关闭）"
    }
}

function Get-Total {
    $out = & (Join-Path $RepoRoot 'measure-memory.ps1') -Mode any
    $line = $out | Where-Object { $_ -like 'TOTAL_MB=*' } | Select-Object -First 1
    if ($line -match 'TOTAL_MB=([0-9.]+)') { return [double]$Matches[1] }
    Write-Warning "度量失败：measure-memory 未输出 TOTAL_MB（进程树缺失？）原始输出: $($out -join ' | ')"
    return -1
}

$fail = @()

Write-Host "== 步骤 1：聚焦主窗口，空闲 5.5 分钟（前台口径）==" -ForegroundColor Yellow
$h = Get-MainHwnd
if ($h -eq [IntPtr]::Zero) { throw "main window not found" }
# 前台激活可能被系统拒绝（后台调用方限制）：不校验就把「未聚焦」测成前台指标
if (-not [QWin]::SetForegroundWindow($h)) {
    Write-Warning "SetForegroundWindow 失败：前台口径可能不成立（请人工点一下窗口后重测）"
}
Start-Sleep -Seconds 330
$fg = Get-Total
if ($fg -lt 0) { exit 2 }
Write-Host ("前台指标（空闲 5.5 分钟后）: {0} MB / 180 MB" -f $fg)
if ($fg -gt 180) { $fail += "foreground=$fg MB > 180" }

Write-Host "== 步骤 2：WM_CLOSE 隐藏到托盘，等待挂起+修剪 + 稳定 60 秒 ==" -ForegroundColor Yellow
Send-WmClose
Start-Sleep -Seconds 10
$trayEarly = Get-Total
Start-Sleep -Seconds 60
$tray = Get-Total
if ($tray -lt 0) { exit 2 }
Write-Host ("托盘指标（挂起+修剪后 10s）: {0} MB" -f $trayEarly)
Write-Host ("托盘指标（稳定 60s 后）: {0} MB / 80 MB" -f $tray)
if ($tray -gt 80) { $fail += "tray=$tray MB > 80" }

Write-Host "== 步骤 3：二次启动唤起（计时恢复）==" -ForegroundColor Yellow
$sw = [System.Diagnostics.Stopwatch]::StartNew()
Start-Process -FilePath $ExePath | Out-Null
$h2 = [IntPtr]::Zero
while ($sw.Elapsed.TotalSeconds -lt 10) {
    Start-Sleep -Milliseconds 200
    $h2 = Get-MainHwnd
    if ($h2 -ne [IntPtr]::Zero) { break }
}
Write-Host ("窗口在 {0:N1}s 内重新可见={1}" -f $sw.Elapsed.TotalSeconds, ($h2 -ne [IntPtr]::Zero))

if ($fail.Count -gt 0) {
    Write-Host "验收未通过：" $fail -ForegroundColor Red
    exit 3
}
Write-Host "== 验收通过，详细日志见 data\logs ==" -ForegroundColor Green
exit 0
