# qa-windows.ps1 - passive UI QA (PrintWindow capture / minimize-blur test)
# ASCII-only on purpose (PS5.1 reads BOM-less UTF-8 as ANSI).
# Modes:
#   menu          - capture the tray-menu window (visible, width < 400 px)
#   settings-blur - minimize the settings window (450..940 px wide) and check it auto-closed
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('menu', 'settings-blur', 'shot-settings', 'settings-watch')][string]$Mode,
    [int]$ProcId = 0,
    [string]$Out = (Join-Path $PSScriptRoot 'shot.png')
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Dpi {
    [DllImport("user32.dll")] public static extern bool SetProcessDpiAwarenessContext(IntPtr ctx);
    public static void MakeAware() { SetProcessDpiAwarenessContext(new IntPtr(-4)); }
}
"@
[Dpi]::MakeAware() | Out-Null
Add-Type @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class QA {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("kernel32.dll")] public static extern uint GetTickCount();
    [StructLayout(LayoutKind.Sequential)] public struct LASTINPUTINFO { public uint cbSize; public uint dwTime; }
    [DllImport("user32.dll")] public static extern bool GetLastInputInfo(ref LASTINPUTINFO plii);
    public static uint LastInputTick() {
        var li = new LASTINPUTINFO();
        li.cbSize = (uint)Marshal.SizeOf(typeof(LASTINPUTINFO));
        return GetLastInputInfo(ref li) ? li.dwTime : 0;
    }
    public static uint PidOf(IntPtr h) { uint p; GetWindowThreadProcessId(h, out p); return p; }
    public struct RECT { public int L, T, R, B; }
    public static uint TargetPid;
    public static List<IntPtr> Found = new List<IntPtr>();
    public static bool Callback(IntPtr h, IntPtr l) {
        uint p; GetWindowThreadProcessId(h, out p);
        if (p == QA.TargetPid && IsWindowVisible(h)) { Found.Add(h); }
        return true;
    }
    public static string Desc(IntPtr h) {
        var sb = new StringBuilder(128); GetWindowText(h, sb, 128);
        var r = new RECT(); GetWindowRect(h, out r);
        return string.Format("hwnd=0x{0:X} alive={1} vis={2} rect=({3},{4})-({5},{6}) size={7}x{8} titleLen={9}",
            h.ToInt64(), IsWindow(h), IsWindowVisible(h), r.L, r.T, r.R, r.B, r.R - r.L, r.B - r.T, sb.Length);
    }
}
"@

function Get-AppWindows([uint32]$pidTarget) {
    [QA]::Found.Clear()
    [QA]::TargetPid = $pidTarget
    [QA]::EnumWindows([QA+EnumProc]{ param($h, $l) [QA]::Callback($h, $l) }, [IntPtr]::Zero) | Out-Null
    return [QA]::Found.ToArray()
}

function Save-Shot([IntPtr]$h, [string]$path) {
    $r = New-Object QA+RECT
    [QA]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    if ($w -le 0 -or $ht -le 0) { Write-Host "skip shot $($w)x$ht"; return }
    $bmp = New-Object System.Drawing.Bitmap($w, $ht)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    [QA]::PrintWindow($h, $hdc, 2) | Out-Null   # PW_RENDERFULLCONTENT
    $g.ReleaseHdc($hdc)
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()
    Write-Host "saved: $path ($w x $ht)"
}

if ($ProcId -eq 0) { throw "ProcId required" }

$wins = Get-AppWindows ([uint32]$ProcId)
Write-Host "== before ($($wins.Count) visible windows) =="
$settingsHwnd = [IntPtr]::Zero
$menuHwnd = [IntPtr]::Zero
foreach ($h in $wins) {
    Write-Host ([QA]::Desc($h))
    $r = New-Object QA+RECT
    [QA]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    if ($Mode -eq 'menu' -and $w -ge 100 -and $w -lt 400) { $menuHwnd = $h }
    if (($Mode -eq 'settings-blur' -or $Mode -eq 'shot-settings') -and $w -ge 450 -and $w -lt 940 -and $ht -ge 350) { $settingsHwnd = $h }
}

if ($Mode -eq 'menu') {
    if ($menuHwnd -ne [IntPtr]::Zero) { Save-Shot $menuHwnd $Out }
    else { Write-Host "MENU WINDOW NOT FOUND" }
}
elseif ($Mode -eq 'shot-settings') {
    if ($settingsHwnd -ne [IntPtr]::Zero) { Save-Shot $settingsHwnd $Out }
    else { Write-Host "SETTINGS WINDOW NOT FOUND" }
}
elseif ($Mode -eq 'settings-watch') {
    # Observe only: wait for the settings window, then report who took the
    # foreground when it disappears and whether any user input happened.
    $deadline = (Get-Date).AddSeconds(15)
    $sh = [IntPtr]::Zero
    while ((Get-Date) -lt $deadline) {
        $wins = Get-AppWindows ([uint32]$ProcId)
        foreach ($h in $wins) {
            $r = New-Object QA+RECT
            [QA]::GetWindowRect($h, [ref]$r) | Out-Null
            $w = $r.R - $r.L
            if ($w -ge 450 -and $w -lt 940) { $sh = $h }
        }
        if ($sh -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 200
    }
    if ($sh -eq [IntPtr]::Zero) { Write-Host "SETTINGS NEVER APPEARED"; exit 2 }
    $inAtFound = [QA]::LastInputTick()
    Write-Host ("settings found hwnd=0x{0:X} inputTick={1}" -f $sh.ToInt64(), $inAtFound)
    $gone = $false
    $watchEnd = (Get-Date).AddSeconds(25)
    while ((Get-Date) -lt $watchEnd) {
        Start-Sleep -Milliseconds 150
        if (-not [QA]::IsWindow($sh) -or -not [QA]::IsWindowVisible($sh)) { $gone = $true; break }
    }
    $fg = [QA]::GetForegroundWindow()
    $fgName = "none"
    $fgPid = [QA]::PidOf($fg)
    if ($fgPid -ne 0) { $fgName = (Get-Process -Id $fgPid -ErrorAction SilentlyContinue).ProcessName }
    $inAtGone = [QA]::LastInputTick()
    Write-Host ("RESULT gone={0} fgName={1} fgPid={2} inputDeltaMs={3}" -f $gone, $fgName, $fgPid, ([int]($inAtGone - $inAtFound)))
}
