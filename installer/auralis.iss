; Auralis Windows installer (Inno Setup 6).
;
; Per-user install that never asks for administrator rights. CI compiles it in
; .github/workflows/package.yml after the release build:
;
;   ISCC.exe /DAppVersion=1.0.0-rc.1 /DBuildNumber=42 ^
;            /DOutputBaseName=auralis-v1.0.0-rc.1-build.42-windows-x86_64-setup ^
;            /Odist installer\auralis.iss
;
; Local builds can omit every define; they fall back to development values below.
; The script expects target\release\auralis.exe to exist (cargo build --release).

#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif
#ifndef BuildNumber
  #define BuildNumber ""
#endif
#ifndef OutputBaseName
  #define OutputBaseName "auralis-windows-x86_64-setup"
#endif

#define RepoRoot AddBackslash(SourcePath) + ".."
#define AppExe "auralis.exe"

; Windows file version resources need four numeric parts: the Cargo MAJOR.MINOR.PATCH
; (prerelease label dropped) plus the CI build number, or 0 for local and tag builds.
#if Pos("-", AppVersion) > 0
  #define NumericBase Copy(AppVersion, 1, Pos("-", AppVersion) - 1)
#else
  #define NumericBase AppVersion
#endif
#if BuildNumber == ""
  #define NumericVersion NumericBase + ".0"
  #define DisplayVersion AppVersion
#else
  #define NumericVersion NumericBase + "." + BuildNumber
  #define DisplayVersion AppVersion + "+build." + BuildNumber
#endif

[Setup]
; Fixed identity: never change AppId, or upgrades and uninstall entries break.
AppId={{155C1A84-3464-4347-A635-E01C55EA48F1}
AppName=Auralis
AppVersion={#AppVersion}
AppVerName=Auralis {#DisplayVersion}
AppPublisher=somyacodes07
AppPublisherURL=https://github.com/somyacodes07/Auralis
AppSupportURL=https://github.com/somyacodes07/Auralis/issues
AppUpdatesURL=https://github.com/somyacodes07/Auralis/releases
AppCopyright=Copyright (c) 2026 Somyajeet Singh. Licensed under MIT OR Apache-2.0.
VersionInfoVersion={#NumericVersion}
VersionInfoProductVersion={#NumericVersion}
VersionInfoTextVersion={#DisplayVersion}
VersionInfoProductTextVersion={#DisplayVersion}
VersionInfoDescription=Auralis Setup

; Per-user install without elevation. With PrivilegesRequired=lowest, {autopf} resolves
; to {userpf}, which is %LOCALAPPDATA%\Programs, so the app lands in
; %LOCALAPPDATA%\Programs\Auralis. No override to an all-users install is offered.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Auralis
DisableProgramGroupPage=yes
DisableDirPage=auto
UsePreviousAppDir=yes
; Tasks are not remembered between installs: the sign-in checkbox instead mirrors the
; current HKCU Run value (see InitializeWizard), so an upgrade never re-enables a setting
; the user turned off inside the app.
UsePreviousTasks=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0

; PrepareToInstall (below) asks a running Auralis to quit first. Restart Manager stays on as
; a safety net for any other process holding the files, and is told not to relaunch them.
CloseApplications=yes
RestartApplications=no

UninstallDisplayName=Auralis
UninstallDisplayIcon={app}\{#AppExe}
OutputDir={#RepoRoot}\dist
OutputBaseFilename={#OutputBaseName}
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
#if FileExists(RepoRoot + "\assets\auralis.ico")
SetupIconFile={#RepoRoot}\assets\auralis.ico
#endif
; Optional wizard artwork. Each size is used only when its file exists; without any of
; them Inno Setup's built-in images are used.
#define ImageDir RepoRoot + "\assets\installer\"
#define WizardImages ""
#define WizardSmallImages ""
#sub AddWizardImage
  #if FileExists(ImageDir + "wizard-image" + ImageSuffix + ".bmp")
    #if WizardImages != ""
      #define WizardImages WizardImages + ","
    #endif
    #define WizardImages WizardImages + ImageDir + "wizard-image" + ImageSuffix + ".bmp"
  #endif
  #if FileExists(ImageDir + "wizard-small" + ImageSuffix + ".bmp")
    #if WizardSmallImages != ""
      #define WizardSmallImages WizardSmallImages + ","
    #endif
    #define WizardSmallImages WizardSmallImages + ImageDir + "wizard-small" + ImageSuffix + ".bmp"
  #endif
#endsub
#define ImageSuffix ""
#expr AddWizardImage
#define ImageSuffix "-125"
#expr AddWizardImage
#define ImageSuffix "-150"
#expr AddWizardImage
#define ImageSuffix "-200"
#expr AddWizardImage
#if WizardImages != ""
WizardImageFile={#WizardImages}
#endif
#if WizardSmallImages != ""
WizardSmallImageFile={#WizardSmallImages}
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "startup"; Description: "Start Auralis when I sign in"; GroupDescription: "Startup:"; Flags: unchecked

[Files]
Source: "{#RepoRoot}\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\THIRD_PARTY_LICENSES.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; Shortcuts use the icon embedded in auralis.exe.
Name: "{autoprograms}\Auralis"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"
Name: "{autodesktop}\Auralis"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Parameters: "gui"; WorkingDir: "{app}"; Description: "Launch Auralis"; Flags: nowait postinstall skipifsilent
; In-app updates run this installer silently with /relaunch=minimized or /relaunch=studio
; (src/update.rs) so Auralis starts again once the upgrade is done.
Filename: "{app}\{#AppExe}"; Parameters: "{code:RelaunchArguments}"; WorkingDir: "{app}"; Flags: nowait; Check: ShouldRelaunch

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  RunValueName = 'Auralis';

// Runs a short PowerShell command hidden. Returns False if PowerShell could not be started.
function RunPowerShell(const Command: String; var ResultCode: Integer): Boolean;
begin
  Result := Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    '-NoLogo -NoProfile -NonInteractive -Command "' + Command + '"',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

// PowerShell pipeline selecting only auralis.exe processes started from ExePath, so a
// portable copy running from another folder is left alone.
function ProcessFilter(const ExePath: String): String;
var
  Quoted: String;
begin
  Quoted := ExePath;
  StringChangeEx(Quoted, '''', '''''', True);
  Result := 'Get-Process -Name auralis -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq ''' + Quoted + ''' }';
end;

// True while an auralis.exe started from ExePath is running.
function IsAuralisRunning(const ExePath: String): Boolean;
var
  ResultCode: Integer;
begin
  Result := RunPowerShell('if (' + ProcessFilter(ExePath) + ') { exit 0 } else { exit 1 }', ResultCode)
    and (ResultCode = 0);
end;

// Stop the installed Auralis before its files are replaced or removed.
//
// 1. "auralis.exe quit" asks the running Studio to quit and restore the desktop wallpaper.
//    It exits 0 when nothing is running. Builds that predate the subcommand print usage and
//    exit non-zero; that is ignored.
// 2. If that copy is still running after a short wait, it is force-terminated. This skips
//    the app's own cleanup, but Windows destroys its wallpaper host windows with the process
//    and Explorer repaints the static wallpaper, and it is the only way to release the files
//    when the app does not respond. Only processes started from the install folder are
//    stopped. If PowerShell cannot be started, taskkill is used instead, limited to the
//    current user's auralis.exe processes.
procedure StopAuralis(const ExePath: String);
var
  ResultCode, Waited: Integer;
begin
  if not FileExists(ExePath) then
    Exit;

  Exec(ExePath, 'quit', ExtractFileDir(ExePath), SW_HIDE, ewWaitUntilTerminated, ResultCode);

  Waited := 0;
  while IsAuralisRunning(ExePath) and (Waited < 5000) do
  begin
    Sleep(250);
    Waited := Waited + 250;
  end;

  if not RunPowerShell(ProcessFilter(ExePath) + ' | Stop-Process -Force', ResultCode) then
    Exec(ExpandConstant('{sys}\taskkill.exe'),
      '/F /IM {#AppExe} /FI "USERNAME eq ' + GetUserNameString + '"',
      '', SW_HIDE, ewWaitUntilTerminated, ResultCode);

  // Give Windows a moment to release the files of a terminated process.
  Waited := 0;
  while IsAuralisRunning(ExePath) and (Waited < 3000) do
  begin
    Sleep(250);
    Waited := Waited + 250;
  end;
end;

// True for a silent install started by the in-app updater, which asks to be started again.
function ShouldRelaunch: Boolean;
var
  Mode: String;
begin
  Mode := ExpandConstant('{param:relaunch|}');
  Result := WizardSilent and ((Mode = 'minimized') or (Mode = 'studio'));
end;

function RelaunchArguments(Param: String): String;
begin
  if ExpandConstant('{param:relaunch|}') = 'minimized' then
    Result := 'gui --minimized'
  else
    Result := 'gui';
end;
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopAuralis(ExpandConstant('{app}\{#AppExe}'));
  Result := '';
end;

// Sign-in startup. The value must match what the in-app "Launch at startup" toggle writes
// (src/platform/windows.rs: value "Auralis" under HKCU\...\Run, data `"<exe>" gui --minimized`)
// so the toggle and the installer always agree.
var
  StartupWasEnabled: Boolean;
  StartupTaskSynced: Boolean;

procedure InitializeWizard();
begin
  StartupWasEnabled := RegValueExists(HKEY_CURRENT_USER, RunKey, RunValueName);
  StartupTaskSynced := False;
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  // Show the checkbox in its current state the first time the page appears.
  if (CurPageID = wpSelectTasks) and (not StartupTaskSynced) then
  begin
    if StartupWasEnabled then
      WizardSelectTasks('startup');
    StartupTaskSynced := True;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep <> ssPostInstall then
    Exit;
  if WizardIsTaskSelected('startup') then
    RegWriteStringValue(HKEY_CURRENT_USER, RunKey, RunValueName,
      '"' + ExpandConstant('{app}\{#AppExe}') + '" gui --minimized')
  else if StartupTaskSynced and StartupWasEnabled then
    // The user saw the box checked and cleared it. Silent installs without /TASKS=startup
    // never reach this branch, so they leave the existing setting alone.
    RegDeleteValue(HKEY_CURRENT_USER, RunKey, RunValueName);
end;

procedure DeleteIfExists(const Path: String);
begin
  if FileExists(Path) then
    DeleteFile(Path);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ConfigDir, DataDir: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    StopAuralis(ExpandConstant('{app}\{#AppExe}'));
    // Remove the sign-in entry whether it came from this installer or the in-app toggle.
    RegDeleteValue(HKEY_CURRENT_USER, RunKey, RunValueName);
  end;

  // Settings and logs are kept unless the user explicitly says otherwise. A silent
  // uninstall never deletes them. User media is never touched: settings only store paths.
  if (CurUninstallStep = usPostUninstall) and (not UninstallSilent) then
  begin
    if MsgBox('Also delete your Auralis settings and logs?' + #13#10#13#10 +
              'Your video files are not affected either way.',
              mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
    begin
      // AppConfig::default_config_path(): %APPDATA%\Auralis\auralis_config.json, plus the
      // temporary file written while saving and the backup kept after a corrupt load.
      ConfigDir := ExpandConstant('{userappdata}\Auralis');
      DeleteIfExists(ConfigDir + '\auralis_config.json');
      DeleteIfExists(ConfigDir + '\auralis_config.tmp');
      DeleteIfExists(ConfigDir + '\auralis_config.json.corrupt.bak');
      RemoveDir(ConfigDir);

      DataDir := ExpandConstant('{localappdata}\Auralis');
      DelTree(DataDir + '\logs', True, True, True);
      RemoveDir(DataDir);
    end;
  end;
end;
