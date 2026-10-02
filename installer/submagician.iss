; SubMagician installer (Inno Setup 6). Build with tools\build-installer.ps1.
; Installs for the current user only (no admin rights): %LOCALAPPDATA%\Programs\SubMagician,
; with ffmpeg and ffprobe next to the app, and a Start menu shortcut.
; In-app updates run it with /VERYSILENT /CLOSEAPPLICATIONS /RELAUNCH=1 (crates/core/src/update.rs).

#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#define AppName "SubMagician"
#define AppExe "submagician.exe"
#define CliExe "submagician-cli.exe"

[Setup]
AppId={{9C4E2A71-5B3D-4F0E-8E62-7A1D3C5B9F20}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=SubMagician contributors
AppPublisherURL=https://github.com/halilkhrmn/submagician
AppSupportURL=https://github.com/halilkhrmn/submagician/issues
DefaultDirName={autopf}\{#AppName}
DisableDirPage=yes
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\target\installer
OutputBaseFilename=submagician-setup-{#AppVersion}
SetupIconFile=..\crates\app\assets\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; The app (and a running worker) lock their files during an update.
CloseApplications=force
RestartApplications=no
LicenseFile=..\LICENSE

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "turkish"; MessagesFile: "compiler:Languages\Turkish.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\{#CliExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\ffmpeg\ffmpeg.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\ffmpeg\ffprobe.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\ffmpeg\FFMPEG-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
; After a normal install: the "Launch SubMagician" box. After an in-app update (silent): start it again.
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\{#AppExe}"; Flags: nowait; Check: RelaunchAfterUpdate

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/IM {#AppExe} /F"; Flags: runhidden; RunOnceId: "StopApp"
Filename: "{sys}\taskkill.exe"; Parameters: "/IM {#CliExe} /F"; Flags: runhidden; RunOnceId: "StopWorker"
; "Find subtitles with SubMagician" in Explorer's menu (Settings → File manager).
Filename: "{sys}\reg.exe"; Parameters: "delete HKCU\Software\Classes\Directory\shell\SubMagician /f"; Flags: runhidden; RunOnceId: "MenuFolder"
Filename: "{sys}\reg.exe"; Parameters: "delete HKCU\Software\Classes\Directory\Background\shell\SubMagician /f"; Flags: runhidden; RunOnceId: "MenuBackground"
Filename: "{sys}\reg.exe"; Parameters: "delete HKCU\Software\Classes\SystemFileAssociations\video\shell\SubMagician /f"; Flags: runhidden; RunOnceId: "MenuVideo"

[UninstallDelete]
; Player plugins (they would fail without the app).
Type: files; Name: "{userappdata}\mpv\scripts\submagician.lua"
Type: files; Name: "{userappdata}\mpv.net\scripts\submagician.lua"
Type: files; Name: "{userappdata}\vlc\lua\extensions\submagician.lua"

[Code]
function RelaunchAfterUpdate: Boolean;
begin
  Result := WizardSilent and (ExpandConstant('{param:RELAUNCH|0}') = '1');
end;

// The app and its worker lock their files: stop them before copying.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM {#AppExe} /F', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM {#CliExe} /F', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;
