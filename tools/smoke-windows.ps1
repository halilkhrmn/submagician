# Installs the setup silently for this user, checks the files, starts the app, uninstalls.
param([Parameter(Mandatory)] [string] $Setup)
$ErrorActionPreference = 'Stop'
$proc = Start-Process $Setup -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-' -PassThru -Wait
if ($proc.ExitCode -ne 0) { throw "setup exited with $($proc.ExitCode)" }
$dir = "$env:LOCALAPPDATA\Programs\SubMagician"
foreach ($f in 'submagician.exe', 'submagician-cli.exe', 'ffmpeg.exe', 'ffprobe.exe', 'unins000.exe') {
    if (-not (Test-Path "$dir\$f")) { throw "missing $f" }
}
& "$dir\submagician-cli.exe" --version
if ($LASTEXITCODE -ne 0) { throw 'submagician-cli failed' }
& "$dir\ffmpeg.exe" -hide_banner -version | Select-Object -First 1

$app = Start-Process "$dir\submagician.exe" -PassThru
Start-Sleep -Seconds 8
if ($app.HasExited) { throw "the window exited with $($app.ExitCode)" }
Stop-Process $app.Id -Force

$un = Start-Process "$dir\unins000.exe" -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -PassThru -Wait
if ($un.ExitCode -ne 0) { throw "uninstall exited with $($un.ExitCode)" }
Start-Sleep -Seconds 3
if (Test-Path "$dir\submagician.exe") { throw 'files left after uninstall' }
'smoke test passed: windows'
