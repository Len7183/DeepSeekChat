﻿# diag-windows.ps1 - enumerate app windows (visibility/rect) + PrintWindow capture menu
param([string]$Out = (Join-Path $PSScriptRoot "shot-menu-pw.png"))
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WD {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr hdc, uint flags);
    public struct RECT { public int L, T, R, B; }
    public static uint TargetPid;
    public static System.Collections.Generic.List<string> Rows = new System.Collections.Generic.List<string>();
    public static IntPtr MenuHwnd = IntPtr.Zero;
    public static bool Callback(IntPtr h, IntPtr l) {
        uint p; GetWindowThreadProcessId(h, out p);
        if (p == WD.TargetPid) {
            var sb = new StringBuilder(256); GetWindowText(h, sb, 256);
            var r = new RECT(); GetWindowRect(h, out r);
            bool vis = IsWindowVisible(h);
            int ex = GetWindowLong(h, -20); // GWL_EXSTYLE
            string title = sb.ToString();
            Rows.Add(string.Format("hwnd=0x{0:X} vis={1} rect=({2},{3})-({4},{5}) ex=0x{6:X} title='{7}'",
                h.ToInt64(), vis, r.L, r.T, r.R, r.B, ex, title));
            if (title.IndexOf("menu", StringComparison.OrdinalIgnoreCase) >= 0 ||
                title.IndexOf("菜单") >= 0) { MenuHwnd = h; }
        }
        return true;
    }
}
"@
$proc = Get-Process | Where-Object { $_.ProcessName -like 'DeepSeekChat*' } | Select-Object -First 1
if (-not $proc) { "process not found"; exit 1 }
[WD]::TargetPid = [uint32]$proc.Id
[WD]::EnumWindows([WD+EnumProc]{ param($h, $l) [WD]::Callback($h, $l) }, [IntPtr]::Zero) | Out-Null
[WD]::Rows | ForEach-Object { Write-Host $_ }
if ([WD]::MenuHwnd -ne [IntPtr]::Zero) {
    $r = New-Object WD+RECT
    [WD]::GetWindowRect([WD]::MenuHwnd, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $h = $r.B - $r.T
    if ($w -gt 0 -and $h -gt 0) {
        $bmp = New-Object System.Drawing.Bitmap($w, $h)
        $g = [System.Drawing.Graphics]::FromImage($bmp)
        $hdc = $g.GetHdc()
        # PW_RENDERFULLCONTENT = 2 (含 WebView2 合成内容)
        [WD]::PrintWindow([WD]::MenuHwnd, $hdc, 2) | Out-Null
        $g.ReleaseHdc($hdc)
        $bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
        $g.Dispose(); $bmp.Dispose()
        Write-Host "PrintWindow saved: $Out ($w x $h)"
    }
} else { Write-Host "menu hwnd not found" }
