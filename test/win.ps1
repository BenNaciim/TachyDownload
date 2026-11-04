# Win32 helpers for the end-to-end tests of the Tauri shell (the window's native menu bar cannot be reached from the
# page). Usage:
#   win.ps1 dump                 the menu bar as JSON: [{ label, checked, items: [...] }]
#   win.ps1 click "<label>"      chooses the entry with that label (what a click in the menu does)
#   win.ps1 close                asks the main window to close (the close button)
#   win.ps1 close-confirm        asks the other (confirmation) window to close without an answer
#   win.ps1 visible              prints True or False: is the main window shown
param([Parameter(Mandatory = $true)][string]$Action, [string]$Label = '')
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Text;
public class TachyWin {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc p, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern IntPtr GetMenu(IntPtr h);
  [DllImport("user32.dll")] public static extern int GetMenuItemCount(IntPtr m);
  [DllImport("user32.dll")] public static extern IntPtr GetSubMenu(IntPtr m, int i);
  [DllImport("user32.dll")] public static extern uint GetMenuItemID(IntPtr m, int i);
  [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetMenuString(IntPtr m, uint pos, StringBuilder s, int max, uint flags);
  [DllImport("user32.dll")] public static extern uint GetMenuState(IntPtr m, uint id, uint flags);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint msg, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
}
"@

# The main window: a Tauri window of the app's process that has a menu bar.
$mainPid = (Get-Process tachy-download -ErrorAction Stop | Select-Object -First 1).Id
$script:main = [IntPtr]::Zero
[void][TachyWin]::EnumWindows({
    param($h, $l)
    $procId = 0
    [void][TachyWin]::GetWindowThreadProcessId($h, [ref]$procId)
    $cls = New-Object System.Text.StringBuilder 64
    [void][TachyWin]::GetClassName($h, $cls, 64)
    if ($procId -eq $mainPid -and $cls.ToString() -eq 'Tauri Window' -and [TachyWin]::GetMenu($h) -ne [IntPtr]::Zero) {
        $script:main = $h
        return $false
    }
    return $true
  }, [IntPtr]::Zero)
if ($script:main -eq [IntPtr]::Zero) { Write-Error 'main window not found'; exit 1 }

function Read-Menu($menu) {
    $out = @()
    for ($i = 0; $i -lt [TachyWin]::GetMenuItemCount($menu); $i++) {
        $sb = New-Object System.Text.StringBuilder 256
        [void][TachyWin]::GetMenuString($menu, $i, $sb, 256, 0x400)
        $label = ($sb.ToString() -split "`t")[0]
        $state = [TachyWin]::GetMenuState($menu, $i, 0x400)
        $sub = [TachyWin]::GetSubMenu($menu, $i)
        $out += [pscustomobject]@{
            label   = $label
            checked = [bool]($state -band 8)
            id      = [TachyWin]::GetMenuItemID($menu, $i)
            items   = $(if ($sub -ne [IntPtr]::Zero) { @(Read-Menu $sub) } else { @() })
        }
    }
    return $out
}

function Find-Item($items, $label) {
    foreach ($i in $items) {
        if ($i.label -eq $label) { return $i }
        $r = Find-Item $i.items $label
        if ($r) { return $r }
    }
    return $null
}

switch ($Action) {
    'dump' { ConvertTo-Json -InputObject @(Read-Menu ([TachyWin]::GetMenu($script:main))) -Depth 10 -Compress }
    'click' {
        $item = Find-Item (Read-Menu ([TachyWin]::GetMenu($script:main))) $Label
        if (-not $item) { Write-Error "no entry $Label"; exit 1 }
        [void][TachyWin]::PostMessage($script:main, 0x111, [IntPtr][int]$item.id, [IntPtr]::Zero) # WM_COMMAND
    }
    'close' { [void][TachyWin]::PostMessage($script:main, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) } # WM_CLOSE
    'close-confirm' {
        [void][TachyWin]::EnumWindows({
            param($h, $l)
            $procId = 0
            [void][TachyWin]::GetWindowThreadProcessId($h, [ref]$procId)
            $cls = New-Object System.Text.StringBuilder 64
            [void][TachyWin]::GetClassName($h, $cls, 64)
            if ($procId -eq $mainPid -and $cls.ToString() -eq 'Tauri Window' -and $h -ne $script:main -and [TachyWin]::IsWindowVisible($h)) {
                [void][TachyWin]::PostMessage($h, 0x10, [IntPtr]::Zero, [IntPtr]::Zero)
            }
            return $true
          }, [IntPtr]::Zero)
    }
    'visible' { [TachyWin]::IsWindowVisible($script:main) }
}
