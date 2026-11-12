; Windows installer for Unbing (Inno Setup 6).
; Built by the release workflow:
;   ISCC /DAppVersion=1.2.3 /DSourceExe=path\to\unbing.exe /DOutputDir=dist packaging\windows\unbing.iss
;
; Installs per user (no admin prompt) into %LOCALAPPDATA%\Programs\Unbing.
; Settings (%APPDATA%\unbing) and downloaded pictures (Pictures\Unbing) are kept on uninstall.
; The daily change is a "Run" registry value written by the app itself (Settings →
; "Change wallpaper every day"); the uninstaller removes it.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\..\target\release\unbing.exe"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
AppId={{B3D57A0E-8C1F-4E6B-9A2D-7F40C61E95A8}
AppName=Unbing
AppVersion={#AppVersion}
AppVerName=Unbing {#AppVersion}
AppPublisher=Rawz06 Studio
AppPublisherURL=https://github.com/Rawz06-Studio/Unbing
AppSupportURL=https://github.com/Rawz06-Studio/Unbing/issues
AppUpdatesURL=https://github.com/Rawz06-Studio/Unbing/releases
DefaultDirName={autopf}\Unbing
DefaultGroupName=Unbing
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\Unbing.exe
UninstallDisplayName=Unbing
OutputDir={#OutputDir}
OutputBaseFilename=Unbing-Setup-x86_64
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
ShowLanguageDialog=auto

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "Unbing.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Unbing"; Filename: "{app}\Unbing.exe"
Name: "{autodesktop}\Unbing"; Filename: "{app}\Unbing.exe"; Tasks: desktopicon

[Registry]
; Nothing is written here (ValueType: none): this only removes, on uninstall,
; the startup entry the app may have created.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Unbing"; Flags: uninsdeletevalue dontcreatekey

[UninstallRun]
; Stop the background process before its file is removed.
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM Unbing.exe"; Flags: runhidden; RunOnceId: "StopUnbing"

[Run]
Filename: "{app}\Unbing.exe"; Description: "{cm:LaunchProgram,Unbing}"; Flags: nowait postinstall skipifsilent
