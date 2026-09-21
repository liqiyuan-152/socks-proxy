#define AppName "Socks Proxy"
#define AppVersion "0.1.0"
#define AppPublisher "Socks Proxy"
#define AppExeName "socks-proxy.exe"

[Setup]
AppId={{7F461940-79C4-4A26-A1B2-B1A8AC3E11D2}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
DefaultDirName={autopf}\Socks Proxy
DefaultGroupName=Socks Proxy
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
OutputDir=output
OutputBaseFilename=socks-proxy-{#AppVersion}-windows-x64-setup
UninstallDisplayIcon={app}\{#AppExeName}
CloseApplications=no
RestartApplications=no

[Files]
Source: "package\socks-proxy.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "package\sing-box.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "package\user-guide.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "package\dependency-licenses.md"; DestDir: "{app}\licenses"; Flags: ignoreversion
Source: "package\licenses\*"; DestDir: "{app}\licenses\sing-box"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "package\source\*"; DestDir: "{app}\source\sing-box"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Socks Proxy"; Filename: "{app}\{#AppExeName}"; WorkingDir: "{app}"
Name: "{group}\使用与恢复说明"; Filename: "{app}\docs\user-guide.md"

[UninstallRun]
Filename: "{cmd}"; Parameters: "/C taskkill /F /IM {#AppExeName} >NUL 2>&1"; Flags: runhidden waituntilterminated; RunOnceId: "StopApplication"
Filename: "{app}\{#AppExeName}"; Parameters: "--recover-direct"; Flags: runhidden waituntilterminated; RunOnceId: "RecoverDirect"

[Code]
procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
  ExistingApp: String;
begin
  if CurStep = ssInstall then
  begin
    ExistingApp := ExpandConstant('{app}\{#AppExeName}');
    if FileExists(ExistingApp) then
    begin
      Exec(ExpandConstant('{cmd}'), '/C taskkill /F /IM {#AppExeName} >NUL 2>&1', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      if not Exec(ExistingApp, '--recover-direct', ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ResultCode) or (ResultCode <> 0) then
        RaiseException('无法在升级前恢复网络，请先按恢复文档切换全局直连。');
    end;
  end;
end;
