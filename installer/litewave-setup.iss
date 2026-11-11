; Inno Setup script for litewave (portable exe -> normal Windows install)
; Lives in installer/. Build locally: cargo build --release, then compile this file with Inno Setup.

#define AppName "litewave"
#ifndef AppVersion
  #define AppVersion "1.0"
#endif
#define AppExe "litewave.exe"

[Setup]
AppId={{8F3C2B1A-5D47-4E69-9A12-0C4E7B1D2A55}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=litewave contributors
; Per-user install: no administrator (UAC) prompt needed
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
LicenseFile=..\LICENSE
SetupIconFile=..\assets\icon.ico
OutputBaseFilename=litewave-setup
OutputDir=..\dist
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "uninstall-litewave.bat"; DestDir: "{app}"; Flags: ignoreversion

[UninstallDelete]
Type: filesandordirs; Name: "{app}"

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent
; After an in-app update (silent install) start litewave again
Filename: "{app}\{#AppExe}"; Flags: nowait skipifnotsilent

[Code]
// On uninstall, offer to also remove settings, cache and the saved Spotify sign-in.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    if MsgBox('Also delete litewave settings, cache and the saved Spotify sign-in?',
              mbConfirmation, MB_YESNO) = IDYES then
    begin
      DelTree(ExpandConstant('{userappdata}\litewave'), True, True, True);
      DelTree(ExpandConstant('{localappdata}\litewave'), True, True, True);
    end;
  end;
end;
