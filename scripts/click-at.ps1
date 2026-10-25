﻿# click-at.ps1 - simulate a mouse click at physical screen coordinates
param([int]$X = 0, [int]$Y = 0)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class M {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint x, uint y, uint d, UIntPtr e);
}
"@
[M]::SetCursorPos($X, $Y) | Out-Null
Start-Sleep -Milliseconds 120
[M]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)  # LEFTDOWN
Start-Sleep -Milliseconds 60
[M]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)  # LEFTUP
"clicked at $X,$Y"
