#define MyAppName "GGST Clip"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "Your Name"
#define GuiExeName "ggst-clip-gui.exe"
#define CuiExeName "ggst-clip-rust.exe"

[Setup]
; NOTE: The value of AppId uniquely identifies this application.
; Do not use the same AppId value in installers for other applications.
AppId={{5D0E8A8F-C9B6-4A73-A3B7-F8B3A8C7F002}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DisableProgramGroupPage=yes
; 成果物(Setup.exe)の出力先 (installerディレクトリ直下のOutputフォルダ)
OutputDir=Output
OutputBaseFilename=GGST-Clip-Setup
Compression=lzma
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
; ビルドされたGUIとCUIの実行ファイルを指定 (相対パス)
Source: "..\target\release\{#GuiExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\{#CuiExeName}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; スタートメニューとデスクトップにGUIツールのショートカットを作成
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#GuiExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#GuiExeName}"; Tasks: desktopicon

[Run]
; インストール完了後にGUIツールを起動するオプション
Filename: "{app}\{#GuiExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent
