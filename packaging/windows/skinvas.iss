; Inno Setup script. Built by CI from dist\Skinvas (see bundle.sh); version comes from SKINVAS_VERSION.
#define AppVersion GetEnv('SKINVAS_VERSION')

[Setup]
AppId={{8F3B1C2E-5D6A-4E7F-9A1B-2C3D4E5F6A7B}
AppName=Skinvas
AppVersion={#AppVersion}
AppPublisher=mezyqq
AppPublisherURL=https://github.com/mezyqq/skinvas
DefaultDirName={autopf}\Skinvas
DefaultGroupName=Skinvas
OutputDir=..\..\dist
OutputBaseFilename=Skinvas-{#AppVersion}-Setup
SetupIconFile=..\..\dist\Skinvas\skinvas.ico
UninstallDisplayIcon={app}\skinvas.ico
LicenseFile=..\..\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequiredOverridesAllowed=dialog

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "uk"; MessagesFile: "compiler:Languages\Ukrainian.isl"
Name: "pl"; MessagesFile: "compiler:Languages\Polish.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\dist\Skinvas\*"; DestDir: "{app}"; Flags: recursesubdirs ignoreversion

[Icons]
Name: "{group}\Skinvas"; Filename: "{app}\bin\skinvas.exe"; IconFilename: "{app}\skinvas.ico"
Name: "{autodesktop}\Skinvas"; Filename: "{app}\bin\skinvas.exe"; IconFilename: "{app}\skinvas.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\bin\skinvas.exe"; Description: "{cm:LaunchProgram,Skinvas}"; Flags: nowait postinstall skipifsilent
