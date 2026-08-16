#define MyAppName "ggst-clipper"
#define MyAppVersion "0.1.4"
#define MyAppPublisher "Naoya"
#define GuiExeName "ggst-clipper.exe"
#define CuiExeName "ggst-clipper-cui.exe"

[Setup]
; NOTE: The value of AppId uniquely identifies this application.
; Do not use the same AppId value in installers for other applications.
AppId={{5D0E8A8F-C9B6-4A73-A3B7-F8B3A8C7F002}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
UninstallDisplayName={#MyAppName} v{#MyAppVersion}
UninstallDisplayIcon={app}\icon.ico
DefaultDirName={autopf}\{#MyAppName}
DisableProgramGroupPage=yes
; 成果物(Setup.exe)の出力先 (installerディレクトリ直下のOutputフォルダ)
OutputDir=Output
OutputBaseFilename=ggst-clipper-setup
Compression=lzma
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\icon.ico

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
; ビルドされたGUIとCUIの実行ファイルを指定 (相対パス)
Source: "..\target\release\{#GuiExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\{#CuiExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\icon.ico"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; スタートメニューとデスクトップにGUIツールのショートカットを作成
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#GuiExeName}"; IconFilename: "{app}\icon.ico"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#GuiExeName}"; Tasks: desktopicon; IconFilename: "{app}\icon.ico"


[Run]
; インストール完了後にGUIツールを起動するオプション
Filename: "{app}\{#GuiExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}"

[Code]
var
  DeleteAppData: Boolean;

function CmdLineParamExists(const Value: string): Boolean;
var
  I: Integer;  
begin
  Result := False;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), Value) = 0 then
    begin
      Result := True;
      Exit;
    end;
end;

function InitializeUninstall(): Boolean;
var
  UninstallForm: TForm;
  PromptLabel: TLabel;
  DataCheckBox: TNewCheckBox;
  YesButton, NoButton: TNewButton;
  UninstallCode: Integer;
  Args: string;
begin
  // カスタムパラメータで再起動された場合（内部プロセス）
  if CmdLineParamExists('/RESTART') then
  begin
    if CmdLineParamExists('/DELETEAPPDATA') then DeleteAppData := True;
    Result := True;
    Exit;
  end;

  // 通常のサイレントアンインストールの場合はそのまま実行
  if UninstallSilent then
  begin
    DeleteAppData := False; 
    Result := True;
    Exit;
  end;

  UninstallForm := TForm.Create(nil);
  try
    UninstallForm.Caption := 'Uninstall';
    UninstallForm.ClientWidth := ScaleX(450);
    UninstallForm.ClientHeight := ScaleY(150);
    UninstallForm.Position := poScreenCenter;

    PromptLabel := TLabel.Create(UninstallForm);
    PromptLabel.Parent := UninstallForm;
    PromptLabel.Caption := 'Are you sure you want to completely remove {#MyAppName} and all of its components?';
    PromptLabel.Left := ScaleX(20);
    PromptLabel.Top := ScaleY(20);
    PromptLabel.Width := UninstallForm.ClientWidth - ScaleX(40);
    PromptLabel.WordWrap := True;

    DataCheckBox := TNewCheckBox.Create(UninstallForm);
    DataCheckBox.Parent := UninstallForm;
    DataCheckBox.Caption := 'Also delete application data such as configuration files and history';
    DataCheckBox.Left := ScaleX(20);
    DataCheckBox.Top := ScaleY(60);
    DataCheckBox.Width := UninstallForm.ClientWidth - ScaleX(40);

    YesButton := TNewButton.Create(UninstallForm);
    YesButton.Parent := UninstallForm;
    YesButton.Caption := '&Yes';
    YesButton.ModalResult := mrYes;
    YesButton.Default := True;
    YesButton.Width := ScaleX(80);
    YesButton.Left := UninstallForm.ClientWidth - ScaleX(180);
    YesButton.Top := UninstallForm.ClientHeight - ScaleY(40);

    NoButton := TNewButton.Create(UninstallForm);
    NoButton.Parent := UninstallForm;
    NoButton.Caption := '&No';
    NoButton.ModalResult := mrNo;
    NoButton.Cancel := True;
    NoButton.Width := ScaleX(80);
    NoButton.Left := UninstallForm.ClientWidth - ScaleX(90);
    NoButton.Top := UninstallForm.ClientHeight - ScaleY(40);

    if UninstallForm.ShowModal() = mrYes then
    begin
      Args := '/SILENT /RESTART';
      if DataCheckBox.Checked then Args := Args + ' /DELETEAPPDATA';
      // カスタム引数を付けてサイレントアンインストーラーを新しく起動
      Exec(ExpandConstant('{uninstallexe}'), Args, '', SW_SHOW, ewNoWait, UninstallCode);
    end;
  finally
    UninstallForm.Free;
  end;
  
  // 元のプロセスはキャンセル扱いにして終了し、標準の確認ダイアログを出させない
  Result := False;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  AppDataDir: string;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    if DeleteAppData then
    begin
      AppDataDir := ExpandConstant('{userappdata}\{#MyAppName}');
      if DirExists(AppDataDir) then
      begin
        DelTree(AppDataDir, True, True, True);
      end;
    end;
    if CmdLineParamExists('/RESTART') then
    begin
      MsgBox('Uninstall completed successfully.', mbInformation, MB_OK);
    end;
  end;
end;
