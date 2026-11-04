# Installs the installer silently into a throwaway folder, checks what it put there, runs the browser-driven tests
# against the installed program, then uninstalls and checks that nothing is left behind but the user's data.
#
#   powershell -File tools\check-installer.ps1 "target\release\bundle\nsis\Tachy Download_2.0.0_x64-setup.exe"
#
# It adds and removes the shortcuts and the uninstall entry of the current user, like a real install. The data folder
# (%APPDATA%\Tachy Download) is only read by the post-install step (--setup-tools: nothing to fetch when the tools
# are already there) and is never deleted.
param([Parameter(Mandatory = $true)][string]$Installer)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$Installer = (Resolve-Path $Installer).Path
$dir = Join-Path $env:TEMP 'tachy-install-test'
if (Test-Path $dir) { Remove-Item $dir -Recurse -Force }
$failures = @()
function Check($what, $ok) {
  if ($ok) { "ok    $what" } else { "FAIL  $what"; $script:failures += $what }
}

$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Tachy Download'
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Tachy Download.lnk'
$desktop = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Tachy Download.lnk'

"== install"
$size = [math]::Round((Get-Item $Installer).Length / 1MB, 2)
"installer: $size MiB"
$t = Measure-Command { Start-Process $Installer -ArgumentList "/S /D=$dir" -Wait }
"installed in $([math]::Round($t.TotalSeconds, 1)) s"
$exe = Join-Path $dir 'tachy-download.exe'
Check 'program installed' (Test-Path $exe)
Check 'extension next to the program (resources\extension)' (Test-Path (Join-Path $dir 'resources\extension\manifest.json'))
Check 'uninstaller installed' (Test-Path (Join-Path $dir 'uninstall.exe'))
Check 'uninstall entry registered' (Test-Path $uninstallKey)
Check 'Start menu shortcut' (Test-Path $startMenu)
Check 'desktop shortcut' (Test-Path $desktop)

"== the installed program passes the browser-driven tests"
Push-Location $root
$env:TACHY_BIN = $exe
& node --test --test-concurrency=1 test/ui.test.js test/shell.test.js 2>&1 | Select-Object -Last 14
$testsFailed = $LASTEXITCODE -ne 0
Remove-Item Env:TACHY_BIN
Pop-Location
Check 'ui.test.js and shell.test.js against the installed program' (-not $testsFailed)
Get-Process tachy-download, msedgewebview2 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep 2

"== update over a running program (what the app's updater runs: /S /UPDATE /R)"
# The relaunch gets a throwaway data folder (/ARGS), so it never touches the real downloads list.
$data = Join-Path $env:TEMP 'tachy-install-test-data'
New-Item -ItemType Directory -Force $data | Out-Null
$running = Start-Process $exe -ArgumentList "--data-dir=`"$data`"", '--hidden' -PassThru
Start-Sleep 4
# No -Wait on Start-Process: it would also wait for the relaunched program, which never ends.
$updater = Start-Process $Installer -ArgumentList "/S /UPDATE /R /ARGS `"--data-dir=```"$data```" --hidden`"" -PassThru
$updater.WaitForExit(120000) | Out-Null
Start-Sleep 6
$after = @(Get-Process tachy-download -ErrorAction SilentlyContinue | Where-Object { $_.Id -ne $running.Id -and $_.Path -eq $exe })
Check 'the old program was stopped' ($null -eq (Get-Process -Id $running.Id -ErrorAction SilentlyContinue))
Check 'the program was relaunched after the update' ($after.Count -ge 1)
Check 'no renamed leftover next to the program' (-not (Test-Path "$exe.old"))
Check 'still one uninstall entry' (Test-Path $uninstallKey)
Get-Process tachy-download, msedgewebview2 -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep 2
Remove-Item $data -Recurse -Force -ErrorAction SilentlyContinue

"== uninstall"
Start-Process (Join-Path $dir 'uninstall.exe') -ArgumentList "/S _?=$dir" -Wait
Remove-Item (Join-Path $dir 'uninstall.exe') -Force -ErrorAction SilentlyContinue
Check 'program removed' (-not (Test-Path $exe))
Check 'uninstall entry removed' (-not (Test-Path $uninstallKey))
Check 'Start menu shortcut removed' (-not (Test-Path $startMenu))
Check 'desktop shortcut removed' (-not (Test-Path $desktop))
Check 'user data kept' (Test-Path (Join-Path $env:APPDATA 'Tachy Download'))
if (Test-Path $dir) { Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue }

if ($failures.Count) { "`n$($failures.Count) check(s) failed"; exit 1 }
"`nall checks passed"
