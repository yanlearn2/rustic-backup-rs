; Rustic Backup 安装程序脚本
; 使用 Inno Setup 编译

#define MyAppName "Rustic Backup"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "rustic-backup"
#define MyAppExeName "rustic-backup.exe"

[Setup]
AppId={{B8F3A2C1-4D7E-4F2A-9C1B-6E5D8A7F3C2D}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=dist
OutputBaseFilename=RusticBackup-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesInstallIn64BitMode=x64
SetupIconFile=assets\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
AppCopyright=MIT License

[Languages]
Name: "chinesesimp"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "附加图标:"; Flags: unchecked
Name: "addtopath"; Description: "添加到系统环境变量 PATH"; GroupDescription: "附加任务:"; Flags: unchecked

[Files]
Source: "dist\rustic-backup.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "dist\config.example.json"; DestDir: "{app}"; Flags: ignoreversion onlyifdoesntexist
Source: "dist\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "dist\bin\README.txt"; DestDir: "{app}\bin"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Parameters: "setup"; IconFilename: "{app}\{#MyAppExeName}"
Name: "{group}\卸载 {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Parameters: "setup"; IconFilename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "启动配置向导"; Parameters: "setup"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
Type: filesandordirs; Name: "{app}\repo"
Type: filesandordirs; Name: "{app}\logs"
Type: filesandordirs; Name: "{app}\bin"
Type: files; Name: "{app}\config.json"

[Code]
const
    EnvironmentKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';

procedure AddToPath();
var
    Path: string;
begin
    if not RegQueryStringValue(HKEY_LOCAL_MACHINE, EnvironmentKey, 'Path', Path) then
        RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path);

    if Pos(Lowercase(ExpandConstant('{app}')), Lowercase(Path)) = 0 then
    begin
        Path := Path + ';' + ExpandConstant('{app}');
        RegWriteStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path);
    end;
end;

procedure RemoveFromPath();
var
    Path: string;
    AppPath: string;
begin
    if RegQueryStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path) then
    begin
        AppPath := ExpandConstant('{app}');
        StringChange(Path, ';' + AppPath, '');
        StringChange(Path, AppPath + ';', '');
        StringChange(Path, AppPath, '');
        RegWriteStringValue(HKEY_CURRENT_USER, 'Environment', 'Path', Path);
    end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
    if CurStep = ssPostInstall then
    begin
        if IsTaskSelected('addtopath') then
            AddToPath();
    end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
    if CurUninstallStep = usPostUninstall then
        RemoveFromPath();
end;
