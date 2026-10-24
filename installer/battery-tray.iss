; Per-user installer (battery-tray-setup.exe), built with Inno Setup 6.
;
; Local build, after `cargo build --release`:
;   iscc installer\battery-tray.iss      -> target\installer\battery-tray-setup.exe
; CI (.github/workflows/installer.yml) also passes /DAppVersion, /DBinDir and /O.

#ifndef AppVersion
  #define AppVersion "0.0.0-dev"
#endif
#ifndef BinDir
  #define BinDir "..\target\release"
#endif

[Setup]
; Upgrades find the existing installation by this id: never change it.
AppId={{54BABCB1-103C-4180-B39D-CD5166B3EF2B}
AppName=Battery Tray
AppVersion={#AppVersion}
AppPublisher=Q-efx
AppPublisherURL=https://github.com/Q-efx/peripheral-battery-tray
AppSupportURL=https://github.com/Q-efx/peripheral-battery-tray/issues
AppUpdatesURL=https://github.com/Q-efx/peripheral-battery-tray/releases
UninstallDisplayName=Battery Tray
UninstallDisplayIcon={app}\battery-tray.exe
; The app only touches HKCU, so it installs per user without a UAC prompt,
; into %LOCALAPPDATA%\Programs\Battery Tray.
PrivilegesRequired=lowest
DefaultDirName={autopf}\Battery Tray
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
; An upgrade closes the running tray app (it has no unsaved state) before replacing it.
CloseApplications=force
WizardStyle=modern
OutputDir=..\target\installer
OutputBaseFilename=battery-tray-setup
Compression=lzma2
SolidCompression=yes

[Tasks]
Name: autostart; Description: "Start with Windows"

[Files]
Source: "{#BinDir}\battery-tray.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\battery-cli.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Battery Tray"; Filename: "{app}\battery-tray.exe"

[Registry]
; The same value src/autostart.rs writes, so the tray menu shows "Start with Windows" checked.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "BatteryTray"; ValueData: """{app}\battery-tray.exe"""; Tasks: autostart
; Uninstall removes it even when it was switched on later from the tray menu.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "BatteryTray"; Flags: uninsdeletevalue
; Preferences (src/settings.rs).
Root: HKCU; Subkey: "Software\BatteryTray"; Flags: uninsdeletekey

[Run]
Filename: "{app}\battery-tray.exe"; Description: "Start Battery Tray now"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im battery-tray.exe"; Flags: runhidden; RunOnceId: "StopTray"
