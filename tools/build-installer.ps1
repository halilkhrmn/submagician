# Builds release binaries and the Windows installer (target\installer\submagician-setup-<version>.exe)
# and the portable zip (target\dist). Downloads ffmpeg (gyan.dev "essentials", checked against its
# published SHA-256) into target\ffmpeg the first time. Requires Inno Setup 6:
#   winget install JRSoftware.InnoSetup
# Provider keys come from SUBMAGICIAN_OPENSUBTITLES_API_KEY / SUBMAGICIAN_SUBDL_API_KEY.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches.Groups[1].Value
cargo build --release -p submagician -p submagician-cli
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$ff = 'target\ffmpeg'
if (-not (Test-Path "$ff\ffmpeg.exe")) {
    New-Item -ItemType Directory $ff -Force | Out-Null
    $zip = "$ff\ffmpeg.zip"
    $url = 'https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip'
    Invoke-WebRequest "$url.sha256" -OutFile "$zip.sha256" -UseBasicParsing
    Invoke-WebRequest $url -OutFile $zip -UseBasicParsing
    $expected = (Get-Content "$zip.sha256" -Raw).Trim().Split()[0].ToLower()
    $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) { throw "ffmpeg download damaged: $actual, expected $expected" }
    Expand-Archive $zip -DestinationPath "$ff\x" -Force
    $bin = Get-ChildItem "$ff\x" -Recurse -Filter ffmpeg.exe | Select-Object -First 1
    Copy-Item $bin.FullName, (Join-Path $bin.DirectoryName 'ffprobe.exe') $ff
    $license = Get-ChildItem "$ff\x" -Recurse -Filter LICENSE* | Select-Object -First 1
    "ffmpeg and ffprobe: the gyan.dev release essentials build ($url), GPL 3.0." +
        " Source: https://ffmpeg.org/download.html and https://www.gyan.dev/ffmpeg/builds/`r`n`r`n" +
        (Get-Content $license.FullName -Raw) | Set-Content "$ff\FFMPEG-LICENSE.txt"
    Remove-Item "$ff\x", $zip, "$zip.sha256" -Recurse -Force
}

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 not found (winget install JRSoftware.InnoSetup)' }
& $iscc "/DAppVersion=$version" installer\submagician.iss
if ($LASTEXITCODE -ne 0) { throw 'ISCC failed' }

# Portable: the same files in a zip (no updates from inside the app).
$dist = 'target\dist'
New-Item -ItemType Directory $dist -Force | Out-Null
Copy-Item "target\installer\submagician-setup-$version.exe" $dist
$stage = 'target\portable\SubMagician'
New-Item -ItemType Directory $stage -Force | Out-Null
Copy-Item target\release\submagician.exe, target\release\submagician-cli.exe, "$ff\ffmpeg.exe", "$ff\ffprobe.exe", "$ff\FFMPEG-LICENSE.txt" $stage
Copy-Item LICENSE "$stage\LICENSE.txt"
Compress-Archive -Path $stage -DestinationPath "$dist\SubMagician-$version-windows-x64-portable.zip" -Force
Get-ChildItem $dist | Select-Object Name, @{ n = 'MB'; e = { [math]::Round($_.Length / 1MB, 1) } }
