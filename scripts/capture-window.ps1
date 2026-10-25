﻿# Capture a specific top-level window of a process by size filter (for visual QA)
param(
    [string]$ProcessName = "deepseekchat",
    [int]$MaxWidth = 700,
    [string]$Out = (Join-Path $PSScriptRoot "shot.png")
)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Text;
using System.Collections.Generic;
using System.Runtime.InteropServices;
public class WinEnum {
    public delegate bool EnumProc(IntPtr h, IntPtr l);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder sb, int max);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    public struct RECT { public int Left, Top, Right, Bottom; }
    public static List<IntPtr> Found = new List<IntPtr>();
    public static uint TargetPid;
    public static bool Callback(IntPtr h, IntPtr l) {
        uint pid; GetWindowThreadProcessId(h, out pid);
        if (pid == TargetPid && IsWindowVisible(h)) {
            var sb = new StringBuilder(256); GetWindowText(h, sb, 256);
            if (sb.Length > 0) Found.Add(h);
        }
        return true;
    }
    public static List<IntPtr> WindowsOf(uint pid) {
        Found = new List<IntPtr>(); TargetPid = pid;
        EnumWindows(Callback, IntPtr.Zero);
        return Found;
    }
}
"@

$proc = Get-Process $ProcessName -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { "process not found"; exit 1 }

$hwnds = [WinEnum]::WindowsOf([uint32]$proc.Id)
$candidates = @()
foreach ($h in $hwnds) {
    $r = New-Object WinEnum+RECT
    [WinEnum]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.Right - $r.Left; $ht = $r.Bottom - $r.Top
    $sb = New-Object System.Text.StringBuilder 256
    [WinEnum]::GetWindowText($h, $sb, 256) | Out-Null
    if ($w -gt 0 -and $w -le $MaxWidth) {
        $candidates += [pscustomobject]@{ H = $h; W = $w; Ht = $ht; Title = $sb.ToString() }
    }
}
if (-not $candidates) { "no candidate window"; exit 1 }
$win = $candidates | Sort-Object W -Descending | Select-Object -First 1
$h = $win.H
[WinEnum]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 500
$r = New-Object WinEnum+RECT
[WinEnum]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.Right - $r.Left; $ht = $r.Bottom - $r.Top
$bmp = New-Object System.Drawing.Bitmap($w, $ht)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($r.Left, $r.Top, 0, 0, $bmp.Size)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
"saved $Out ($w x $ht) title='" + $win.Title + "'"
