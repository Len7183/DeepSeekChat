# measure-memory.ps1 —— DeepSeekChat 内存指标验收脚本（FR-09）
#
# 口径：DeepSeekChat.exe 及按父进程链归属的全部 msedgewebview2.exe 子进程的
#       专用工作集（Private Working Set）合计，排除其他应用的 WebView2 进程。
#
# 用法：
#   .\measure-memory.ps1                          # 测一次当前值（-Mode any 不判达标）
#   .\measure-memory.ps1 -Mode foreground         # 对照前台指标 180MB
#   .\measure-memory.ps1 -Mode tray               # 对照托盘挂起指标 80MB
#   .\measure-memory.ps1 -Mode tray -Monitor 60   # 每 5 秒采样持续 60 秒
#
# -ProcessName 按前缀匹配（默认 deepseekchat），因此对 cargo 产物 deepseekchat.exe、
# 安装版 DeepSeekChat.exe、绿色版 DeepSeekChat-v0.1.0-portable.exe 都成立。
#
# 测量方法（验收口径）：
#   前台：主窗口可见聚焦，页面加载完成后无用户交互空闲 5 分钟后测量；
#   托盘：窗口隐藏、挂起/修剪完成后稳定 60 秒后测量。
param(
    [ValidateSet('any', 'foreground', 'tray')][string]$Mode = 'any',
    [double]$ForegroundThresholdMB = 180,
    [double]$TrayThresholdMB = 80,
    [int]$Monitor = 0,
    [string]$ProcessName = 'deepseekchat'
)

$ErrorActionPreference = 'Stop'

function Get-ProcessTree {
    # 返回：@{ AppPid=...; Pids=@(...); Rows=@(...); TotalMB=... }；找不到主进程返回 $null。
    # 本函数只返回该哈希表——机器可读行由脚本末尾输出。函数里混写 Write-Output 会让
    # 调用方拿到的「哈希表」变成「字符串 + 哈希表」的数组（靠成员枚举碰巧仍能取值）。
    $all = Get-CimInstance Win32_Process |
        Select-Object ProcessId, ParentProcessId, Name

    # 名称按前缀匹配（-like 不区分大小写）：cargo 产物与安装版是 deepseekchat.exe /
    # DeepSeekChat.exe，而 README 推荐的绿色版产物名是 DeepSeekChat-v0.1.0-portable.exe。
    # 通配符转义：参数值里的 *[?]\ 会改变匹配语义（'app[1]' 永远匹配不上）
    $escaped = $ProcessName.Replace('\', '\\').Replace('[', '`[').Replace(']', '`]').Replace('*', '`*').Replace('?', '`?')
    $apps = @($all | Where-Object { $_.Name -like "$escaped*" })
    if ($apps.Count -eq 0) { return $null }

    # 多个主进程时全部纳入（单实例下应只有一个）
    $roots = @($apps | Select-Object -ExpandProperty ProcessId)

    # 父进程链闭包：后代集合（含根）
    $descendants = New-Object System.Collections.Generic.HashSet[uint32]
    foreach ($r in $roots) { [void]$descendants.Add([uint32]$r) }
    do {
        $grew = $false
        foreach ($p in $all) {
            if ($descendants.Contains([uint32]$p.ParentProcessId) -and -not $descendants.Contains([uint32]$p.ProcessId)) {
                [void]$descendants.Add([uint32]$p.ProcessId)
                $grew = $true
            }
        }
    } while ($grew)

    # 专用工作集（按 PID）
    $perf = Get-CimInstance Win32_PerfFormattedData_PerfProc_Process |
        Where-Object { $descendants.Contains([uint32]$_.IDProcess) } |
        Select-Object IDProcess, Name, @{ n = 'PrivateMB'; e = { [math]::Round($_.WorkingSetPrivate / 1MB, 2) } }

    $rows = @($perf | Sort-Object PrivateMB -Descending)
    # 空集（进程在快照与性能查询之间退出）Sum 为 $null，Round 后变 0——
    # 这是「测不到」不是「0MB」，保留 $null 让调用方按缺失处理
    $sumObj = @($rows) | Measure-Object -Sum PrivateMB
    $total = if ($null -ne $sumObj.Sum) { [math]::Round($sumObj.Sum, 2) } else { $null }

    return @{ AppPid = $roots; Pids = $descendants; Rows = $rows; TotalMB = $total }
}

function Show-Snapshot {
    $tree = Get-ProcessTree
    if ($null -eq $tree) {
        Write-Host "未找到进程 $ProcessName.exe（应用未运行？）" -ForegroundColor Yellow
        return $null
    }
    $webviewRows = @($tree.Rows | Where-Object { $_.Name -like 'msedgewebview2*' })
    $appRows     = @($tree.Rows | Where-Object { $_.Name -notlike 'msedgewebview2*' })

    Write-Host ("进程树: {0} 个进程（主进程 {1}，WebView2 {2}）" -f $tree.Rows.Count, ($tree.AppPid -join ','), $webviewRows.Count)
    $tree.Rows | ForEach-Object {
        "{0,8}  {1,9:N2} MB  {2}" -f $_.IDProcess, $_.PrivateMB, $_.Name
    } | Write-Host
    Write-Host ("合计专用工作集: {0:N2} MB" -f $tree.TotalMB) -ForegroundColor Cyan

    if ($Mode -ne 'any') {
        $threshold = if ($Mode -eq 'foreground') { $ForegroundThresholdMB } else { $TrayThresholdMB }
        $ok = $tree.TotalMB -le $threshold
        $label = if ($Mode -eq 'foreground') { "前台 ≤${ForegroundThresholdMB}MB" } else { "托盘挂起 ≤${TrayThresholdMB}MB" }
        if ($ok) {
            Write-Host ("[{0}] PASS —— {1:N2}MB ≤ {2}MB" -f $label, $tree.TotalMB, $threshold) -ForegroundColor Green
        } else {
            Write-Host ("[{0}] FAIL —— {1:N2}MB > {2}MB" -f $label, $tree.TotalMB, $threshold) -ForegroundColor Red
        }
    }
    return $tree.TotalMB
}

# 机器可读输出必须写在脚本主体：放进函数会被 `$null = Show-Snapshot` 一起丢掉，
# stdout 上永远没有 TOTAL_MB 行，qa-memory.ps1 的数值采集就会一直拿到 -1。
# 人类可读报表走 Write-Host（不进成功流），与之互不干扰。
# 数值固定用 InvariantCulture 输出：逗号小数区域（de/fr/ru）会把 TOTAL_MB=123,45
# 写出来，下游正则 [0-9.]+ 截断成 123，静默丢失精度
if ($Monitor -gt 0) {
    $end = (Get-Date).AddSeconds($Monitor)
    while ((Get-Date) -lt $end) {
        $total = Show-Snapshot
        if ($null -ne $total) { Write-Output ("TOTAL_MB={0}" -f $total.ToString([System.Globalization.CultureInfo]::InvariantCulture)) }
        Write-Host "---"
        Start-Sleep -Seconds 5
    }
} else {
    $total = Show-Snapshot
    if ($null -ne $total) { Write-Output ("TOTAL_MB={0}" -f $total.ToString([System.Globalization.CultureInfo]::InvariantCulture)) }
}
