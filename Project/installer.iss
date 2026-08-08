; Silanes (Rust) Inno Setup 安装脚本：双语 / /VERYSILENT 静默 / 版本比较
; 服务更新程序注册与卸载 / PATH 注册

#define MyAppName "Silanes"
#define MyAppVersion "26.4.1"
#define MyAppPublisher "Copyright (C) 2026 NXRKYMANE SOFTWARE"
#define MyAppURL "https://github.com/NXRKYMANE/Silanes"
#define MyAppExeName "silanes64.exe"
#define MyAppFlavor "rust"

[Setup]
; AppId 唯一标识产品与卸载键，覆盖安装时保持不变
AppId={{A7B4C9D2-3E5F-4A6B-8C7D-9E0F1A2B3C4D}
AppName={#MyAppName} v{#MyAppVersion}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} v{#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
; 安装目录必须与 install_path() 一致（Program Files\Silanes\silanes64.exe），
; 否则 -internal --install-updater 的 require_install_path 校验会失败
DefaultDirName={autopf}\Silanes
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\Publish
OutputBaseFilename=silanes-win-x64-setup-v{#MyAppVersion}
SetupIconFile=..\Misc\images\Proj.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2
SolidCompression=yes
WizardStyle=classic
DisableWelcomePage=no
WizardImageFile=..\Misc\images\Background.bmp
WizardSmallImageFile=..\Misc\images\Rust.bmp
; 覆盖正在运行的 silanes64.exe（覆盖安装场景）时由 Inno 自动关闭进程，避免文件被占用
CloseApplications=yes
RestartApplications=no
UsePreviousLanguage=no
; 强制安装到 Program Files\Silanes（与 install_path() 契约一致），用户不可选目录，避免安装到
; 其它位置导致 -internal --install-updater 的 require_install_path 校验失败
DisableDirPage=yes
DirExistsWarning=no
; 显式安装总是弹语言选择框；静默/自动化安装须显式传 /LANG=（/LANG 优先级最高，传了就不弹框）
ShowLanguageDialog=yes
VersionInfoVersion={#MyAppVersion}.0
VersionInfoProductName={#MyAppName}
VersionInfoProductVersion={#MyAppVersion}.0
VersionInfoCompany=NXRKYMANE SOFTWARE
VersionInfoCopyright={#MyAppPublisher}
VersionInfoDescription=Silanes Installer

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimp"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[CustomMessages]
english.SameVersionPrompt=An identical version (v%1) is already installed. Reinstall?
chinesesimp.SameVersionPrompt=已安装相同版本的 Silanes (v%1)。是否重新安装？
english.DowngradePrompt=A newer version (v%1) is already installed. Downgrade to v{#MyAppVersion}?
chinesesimp.DowngradePrompt=已安装更新的版本 (v%1)。降级到 v{#MyAppVersion}？
english.UpdaterRegisterFail=Failed to register service updater.%n%n%1%n%nAbort: exit setup  |  Retry: try again  |  Ignore: skip and continue
chinesesimp.UpdaterRegisterFail=注册服务更新程序失败。%n%n%1%n%n「终止」退出安装  「重试」重新注册  「忽略」跳过并继续
english.UpdaterRemoveFail=Failed to remove service updater.%n%n%1%n%nAbort: exit uninstall  |  Retry: try again  |  Ignore: skip and continue
chinesesimp.UpdaterRemoveFail=移除服务更新程序失败。%n%n%1%n%n「终止」退出卸载  「重试」重新尝试  「忽略」跳过并继续
english.NoOutput=(no output captured; exit code %1)
chinesesimp.NoOutput=（未捕获到输出；退出码 %1）
english.ViewDoc=View Documentation
chinesesimp.ViewDoc=查看中文文档
english.RebootPrompt=A reboot is required to complete the installation.%n%nReboot now?
chinesesimp.RebootPrompt=需要重启系统才能完成安装。%n%n是否立即重启？
english.InstallCancelled=Installation cancelled.
chinesesimp.InstallCancelled=安装已取消。

[Files]
; [Setup] CloseApplications=yes 已在覆盖运行中的 silanes64.exe 时自动关闭进程
Source: "..\Publish\silanes64.exe"; DestDir: "{app}"; Flags: ignoreversion; AfterInstall: LogFile('{app}\silanes64.exe')
Source: "..\Misc\images\Proj.ico"; DestDir: "{app}"; DestName: "icon.ico"; Flags: ignoreversion; AfterInstall: LogFile('{app}\icon.ico')
Source: "..\Misc\sil.cmd"; DestDir: "{app}"; DestName: "sil.cmd"; Flags: ignoreversion; AfterInstall: LogFile('{app}\sil.cmd')
Source: "..\Docs\*"; DestDir: "{app}\Docs"; Flags: recursesubdirs createallsubdirs ignoreversion; AfterInstall: LogFile('{app}\Docs\')

[Registry]
Root: HKLM; Subkey: "Software\Microsoft\Windows\CurrentVersion\App Paths\silanes64.exe"; ValueType: string; ValueName: ""; ValueData: "{app}\silanes64.exe"; Flags: uninsdeletekey
; 卸载元数据键（供外部查询版本等）；SystemComponent=1 使其不显示在"程序和功能"，
; 不写 UninstallString/DisplayIcon，卸载统一由 Inno 的 {AppId}_is1 条目负责
Root: HKLM; Subkey: "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Silanes"; ValueType: dword; ValueName: "SystemComponent"; ValueData: "1"; Flags: uninsdeletevalue
Root: HKLM; Subkey: "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Silanes"; ValueType: string; ValueName: "DisplayName"; ValueData: "{#MyAppName}"
Root: HKLM; Subkey: "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Silanes"; ValueType: string; ValueName: "DisplayVersion"; ValueData: "{#MyAppVersion}"
Root: HKLM; Subkey: "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Silanes"; ValueType: string; ValueName: "Flavor"; ValueData: "{#MyAppFlavor}"
Root: HKLM; Subkey: "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Silanes"; ValueType: string; ValueName: "Publisher"; ValueData: "{#MyAppPublisher}"
Root: HKLM; Subkey: "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Silanes"; ValueType: string; ValueName: "URLInfoAbout"; ValueData: "{#MyAppURL}"
Root: HKLM; Subkey: "SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Silanes"; ValueType: string; ValueName: "InstallLocation"; ValueData: "{app}"; Flags: uninsdeletekey

[Run]
Filename: "{code:GetDocPath}"; Description: "{cm:ViewDoc}"; Flags: postinstall nowait skipifsilent unchecked shellexec

[Code]
const
  // 注意：Pascal 字符串中 { 不需要写 {{
  UninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{A7B4C9D2-3E5F-4A6B-8C7D-9E0F1A2B3C4D}_is1';
  // NSIS 旧版安装的卸载键（兼容从 NSIS 安装包升级）
  NSISUninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Silanes';

var
  // 安装页与准备页的滚动日志面板（模拟 NSIS 的 DetailPrint）
  LogMemo: TNewMemo;
  PrepLogMemo: TNewMemo;
  // 准备页进度条（Inno 7 已移除 PreparingGauge 属性，自行创建）
  PrepGauge: TNewProgressBar;
  // 按钮行左侧的版权文本
  CopyrightLabel: TNewStaticText;

// ── 安装页日志：追加一行并滚动到底部（写入两个页面）──
// SelLength 清零后再设 SelStart；WM_VSCROLL(SB_BOTTOM) 不依赖焦点，保证滚到底
procedure AddLog(const Msg: String);
begin
  if Msg = '' then
    Exit;
  if LogMemo <> nil then
  begin
    SendMessage(LogMemo.Handle, $000B {EM_SETREDRAW}, 0, 0);
    LogMemo.Lines.Add(Msg);
    LogMemo.SelLength := 0;
    LogMemo.SelStart := Length(LogMemo.Text);
    SendMessage(LogMemo.Handle, $00B7 {EM_SCROLLCARET}, 0, 0);
    SendMessage(LogMemo.Handle, $0115 {WM_VSCROLL}, 7 {SB_BOTTOM}, 0);
    SendMessage(LogMemo.Handle, $000B {EM_SETREDRAW}, 1, 0);
  end;
  if PrepLogMemo <> nil then
  begin
    SendMessage(PrepLogMemo.Handle, $000B {EM_SETREDRAW}, 0, 0);
    PrepLogMemo.Lines.Add(Msg);
    PrepLogMemo.SelLength := 0;
    PrepLogMemo.SelStart := Length(PrepLogMemo.Text);
    SendMessage(PrepLogMemo.Handle, $00B7 {EM_SCROLLCARET}, 0, 0);
    SendMessage(PrepLogMemo.Handle, $0115 {WM_VSCROLL}, 7 {SB_BOTTOM}, 0);
    SendMessage(PrepLogMemo.Handle, $000B {EM_SETREDRAW}, 1, 0);
  end;
end;

// [Files] 条目安装完成后的回调，按 NSIS 的 "Extract: <完整路径>" 格式记录
procedure LogFile(const TargetPath: String);
begin
  AddLog('Extract: ' + ExpandConstant(TargetPath));
end;

// ── 创建滚动日志面板（安装页 + 准备页，均位于进度条下方）──
procedure InitializeWizard;
var
  AnchorTop, LogHeight: Integer;
begin
  // 安装页日志框（ssNone 隐藏滚动条，bsSingle 为 Win10 扁平边框）
  LogMemo := TNewMemo.Create(WizardForm);
  LogMemo.Parent := WizardForm.InstallingPage;
  LogMemo.ReadOnly := True;
  LogMemo.ScrollBars := ssNone;
  LogMemo.BorderStyle := bsSingle;
  LogMemo.WantTabs := True;
  LogMemo.Color := clWhite;
  LogMemo.Font.Name := 'Cascadia Code';
  LogMemo.Font.Size := 9;

  // 以进度条与状态文本中较低者为锚点，确保日志框始终在它们下方
  if WizardForm.StatusLabel.Top > WizardForm.ProgressGauge.Top then
    AnchorTop := WizardForm.StatusLabel.Top + WizardForm.StatusLabel.Height
  else
    AnchorTop := WizardForm.ProgressGauge.Top + WizardForm.ProgressGauge.Height;

  LogHeight := WizardForm.InstallingPage.ClientHeight - AnchorTop - 20;
  if LogHeight < 0 then
    LogHeight := 0;

  LogMemo.SetBounds(
    WizardForm.ProgressGauge.Left,
    AnchorTop + 8,
    WizardForm.ProgressGauge.Width,
    LogHeight);

  // 准备页进度条 + 日志框（与安装页同坐标，切换页面无跳动）
  PrepGauge := TNewProgressBar.Create(WizardForm);
  PrepGauge.Parent := WizardForm.PreparingPage;
  PrepGauge.Style := npbstMarquee;
  PrepGauge.SetBounds(
    WizardForm.ProgressGauge.Left,
    WizardForm.ProgressGauge.Top,
    WizardForm.ProgressGauge.Width,
    WizardForm.ProgressGauge.Height);

  PrepLogMemo := TNewMemo.Create(WizardForm);
  PrepLogMemo.Parent := WizardForm.PreparingPage;
  PrepLogMemo.ReadOnly := True;
  PrepLogMemo.ScrollBars := ssNone;
  PrepLogMemo.BorderStyle := bsSingle;
  PrepLogMemo.WantTabs := True;
  PrepLogMemo.Color := clWhite;
  PrepLogMemo.Font.Name := 'Cascadia Code';
  PrepLogMemo.Font.Size := 9;
  PrepLogMemo.SetBounds(LogMemo.Left, LogMemo.Top, LogMemo.Width, LogMemo.Height);

  // 准备就绪页的目录摘要列表：同样隐藏滚动条并改用扁平边框
  WizardForm.ReadyMemo.ScrollBars := ssNone;
  WizardForm.ReadyMemo.BorderStyle := bsSingle;
  WizardForm.ReadyMemo.Color := clWhite;

  // 按钮行左侧的版权文本（与按钮垂直居中，不覆盖按钮区）
  CopyrightLabel := TNewStaticText.Create(WizardForm);
  CopyrightLabel.Parent := WizardForm;
  CopyrightLabel.Caption := '{#MyAppPublisher}';
  CopyrightLabel.Font.Size := 8;
  CopyrightLabel.Font.Color := clGray;
  CopyrightLabel.AutoSize := True;
  CopyrightLabel.Top := WizardForm.CancelButton.Top + (WizardForm.CancelButton.Height - CopyrightLabel.Height) div 2;
  CopyrightLabel.Left := 35;
end;

// ── 页面切换时补设样式：Inno 7 的页面控件按需创建，懒创建的控件需在此重设 ──
// 进入含日志框的页面时把焦点移到按钮上，避免日志框获得焦点导致光标闪烁
procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpReady then
  begin
    WizardForm.ReadyMemo.ScrollBars := ssNone;
    WizardForm.ReadyMemo.BorderStyle := bsSingle;
    WizardForm.ReadyMemo.Color := clWhite;
  end
  else if CurPageID = wpPreparing then
  begin
    PrepLogMemo.ScrollBars := ssNone;
    PrepLogMemo.BorderStyle := bsSingle;
    // 隐藏准备页内置描述文本，避免被日志框遮挡后从边缘漏字
    WizardForm.PreparingLabel.Visible := False;
    // WM_NEXTDLGCTL(wParam=句柄)：把焦点交给指定控件（Inno 脚本未暴露 SetFocus）
    if WizardForm.NextButton.Visible and WizardForm.NextButton.Enabled then
      SendMessage(WizardForm.Handle, $0028 {WM_NEXTDLGCTL}, WizardForm.NextButton.Handle, 0)
    else if WizardForm.CancelButton.Visible and WizardForm.CancelButton.Enabled then
      SendMessage(WizardForm.Handle, $0028 {WM_NEXTDLGCTL}, WizardForm.CancelButton.Handle, 0);
  end
  else if CurPageID = wpInstalling then
  begin
    LogMemo.ScrollBars := ssNone;
    LogMemo.BorderStyle := bsSingle;
    if WizardForm.NextButton.Visible and WizardForm.NextButton.Enabled then
      SendMessage(WizardForm.Handle, $0028 {WM_NEXTDLGCTL}, WizardForm.NextButton.Handle, 0)
    else if WizardForm.CancelButton.Visible and WizardForm.CancelButton.Enabled then
      SendMessage(WizardForm.Handle, $0028 {WM_NEXTDLGCTL}, WizardForm.CancelButton.Handle, 0);
  end;
end;

// ── 版本比较：V1>V2 → 1, V1=V2 → 0, V1<V2 → -1 ──
function CompareVersions(const V1, V2: String): Integer;
var
  A1, A2: String;
  P1, P2, C1, C2: Integer;
begin
  A1 := V1;
  A2 := V2;
  while True do
  begin
    P1 := Pos('.', A1);
    P2 := Pos('.', A2);
    if P1 = 0 then
      C1 := StrToIntDef(A1, 0)
    else
    begin
      C1 := StrToIntDef(Copy(A1, 1, P1 - 1), 0);
      A1 := Copy(A1, P1 + 1, MaxInt);
    end;
    if P2 = 0 then
      C2 := StrToIntDef(A2, 0)
    else
    begin
      C2 := StrToIntDef(Copy(A2, 1, P2 - 1), 0);
      A2 := Copy(A2, P2 + 1, MaxInt);
    end;
    if C1 < C2 then
    begin
      Result := -1;
      Exit;
    end;
    if C1 > C2 then
    begin
      Result := 1;
      Exit;
    end;
    if P1 = 0 then
    begin
      if P2 = 0 then
        Result := 0
      else if StrToIntDef(A2, 0) > 0 then
        Result := -1
      else
        Result := 0;
      Exit;
    end;
    if P2 = 0 then
    begin
      if StrToIntDef(A1, 0) > 0 then
        Result := 1
      else
        Result := 0;
      Exit;
    end;
  end;
end;

// ── 等待 silanes64.exe 完全退出（最长 MaxSec 秒），避免覆盖运行中的 exe ──
procedure WaitForSilanesExit(MaxSec: Integer);
var
  I, J: Integer;
  Output: TExecOutput;
  ResultCode: Integer;
  Found: Boolean;
begin
  I := 0;
  while I < MaxSec do
  begin
    Found := False;
    if ExecAndCaptureOutput('tasklist.exe', '/FI "IMAGENAME eq silanes64.exe" /FO CSV /NH', '', SW_HIDE, ewWaitUntilTerminated, ResultCode, Output) then
    begin
      for J := 0 to GetArrayLength(Output.StdOut) - 1 do
      begin
        if Pos('silanes64.exe', Output.StdOut[J]) > 0 then
          Found := True;
      end;
    end;
    if not Found then
      Exit;
    Sleep(1000);
    I := I + 1;
  end;
end;

// ── 构建错误详情：合并 stdout/stderr 全部行 + 退出码 + 命令行 ──
function BuildErrorText(const Args: String; const ResultCode: Integer; const Output: TExecOutput): String;
var
  I: Integer;
begin
  Result := '';
  for I := 0 to GetArrayLength(Output.StdErr) - 1 do
    Result := Result + Output.StdErr[I] + #13#10;
  for I := 0 to GetArrayLength(Output.StdOut) - 1 do
    Result := Result + Output.StdOut[I] + #13#10;
  Result := TrimRight(Result);
  if Result = '' then
    Result := FmtMessage(CustomMessage('NoOutput'), [IntToStr(ResultCode)]);
  Result := Result + #13#10 + 'Command: ' + Args;
end;

// ── 静默执行本安装目录的 silanes64.exe（不弹窗），失败时返回错误文本；输出同时追加到日志框 ──
function SilanesExec(const Args: String; var ErrText: String): Boolean;
var
  Output: TExecOutput;
  ResultCode: Integer;
  I: Integer;
begin
  Result := False;
  ErrText := '';
  // 注意：ExecAndCaptureOutput 的 Filename 不能带引号，否则进程启动失败（error 87）
  if ExecAndCaptureOutput(ExpandConstant('{app}\silanes64.exe'), Args, '', SW_HIDE, ewWaitUntilTerminated, ResultCode, Output) and (ResultCode = 0) then
    Result := True
  else
    ErrText := BuildErrorText(Args, ResultCode, Output);

  // 将 silanes 的实际输出逐行显示到 detail 日志框
  for I := 0 to GetArrayLength(Output.StdErr) - 1 do
    if Trim(Output.StdErr[I]) <> '' then
      AddLog(Output.StdErr[I]);
  for I := 0 to GetArrayLength(Output.StdOut) - 1 do
    if Trim(Output.StdOut[I]) <> '' then
      AddLog(Output.StdOut[I]);
end;

// ── 通过 silanes64.exe 执行命令，失败弹「终止 / 重试 / 忽略」并显示完整错误流 ──
function RunSilanesCommand(const Args, FailMsg: String): Boolean;
var
  ErrText: String;
begin
  Result := False;
  while True do
  begin
    if SilanesExec(Args, ErrText) then
    begin
      Result := True;
      Exit;
    end;
    case MsgBox(FmtMessage(FailMsg, [ErrText]), mbError, MB_ABORTRETRYIGNORE) of
      IDABORT:
        begin
          Result := False;
          Exit;
        end;
      IDIGNORE:
        begin
          Result := True;
          Exit;
        end;
      IDOK:  // 静默模式 MsgBox 不显示并返回 IDOK，视为失败退出，避免无限重试
        begin
          Result := False;
          Exit;
        end;
    end;
  end;
end;

// ── 安装目录加入系统 PATH（已存在则跳过），大小写不敏感、条目级精确匹配避免破坏系统 PATH ──
procedure AddToPath;
var
  PathVal, Dir, LowerPath, LowerDir: String;
begin
  Dir := ExpandConstant('{app}');
  // 规范化目录: 去掉尾部 "\"（若有）
  if Copy(Dir, Length(Dir), 1) = '\' then
    Delete(Dir, Length(Dir), 1);

  if not RegQueryStringValue(HKLM, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'Path', PathVal) then
    Exit; // 读取失败（如超长 PATH）: 不修改，避免把读取失败误当"PATH 为空"而清空整个系统 PATH
  if Trim(PathVal) = '' then
  begin
    RegWriteExpandStringValue(HKLM, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'Path', Dir);
    Exit;
  end;

  // 大小写不敏感、条目级精确检查: 给 PATH 与目录首尾都补 ";" 做边界，
  // 避免 "C:\Program Files\Silanes" 与 "C:\Program Files (x86)\Silanes" 之类的子串误判
  LowerPath := ';' + LowerCase(PathVal) + ';';
  LowerDir := ';' + LowerCase(Dir) + ';';
  if Pos(LowerDir, LowerPath) = 0 then
  begin
    RegWriteExpandStringValue(HKLM, 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment', 'Path', PathVal + ';' + Dir);
    // 广播 WM_SETTINGCHANGE 通知环境变更（PostMessage 不阻塞，避免挂起窗口拖死安装）
    PostMessage(65535 {HWND_BROADCAST}, 26 {WM_SETTINGCHANGE}, 0, 0);
  end;
end;

// ── 旧版本确认：升级直接继续；同版本/降级需确认（静默: 降级中止、同版重装、升级继续，对应 NSIS /SD）──
function ShouldProceedWithOldVersion(const OldVer: String): Boolean;
var
  Cmp: Integer;
begin
  Cmp := CompareVersions('{#MyAppVersion}', OldVer);
  if Cmp = 0 then
  begin
    if WizardSilent then
      Result := True
    else
      Result := MsgBox(FmtMessage(CustomMessage('SameVersionPrompt'), [OldVer]), mbConfirmation, MB_YESNO) = IDYES;
  end
  else if Cmp < 0 then
  begin
    if WizardSilent then
      Result := False
    else
      Result := MsgBox(FmtMessage(CustomMessage('DowngradePrompt'), [OldVer]), mbConfirmation, MB_YESNO) = IDYES;
  end
  else
    Result := True;
end;

function InitializeSetup(): Boolean;
begin
  // 静默安装参数（/VERYSILENT /LANG=...）由 updater 直接传入：Inno 在 SYSTEM（Session 0）下
  // 以临时服务模式安装，此处任何 cmd.exe 重入都会递归卡死，故直接继续
  Result := True;
end;

// ── 安装前清理（PrepareToInstall 在文件复制前调用）：运行旧版卸载器（Inno 或 NSIS）清理旧服务与旧文件；
// 静默模式下等待旧 silanes64.exe 退出（避免覆盖运行中文件）；旧安装目录与目标不同时删除旧目录残留
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  OldVer, OldUninst, OldDir: String;
  ResultCode: Integer;
begin
  Result := '';

  // 1. 版本检查与确认（NSI .onInit 行为：升级静默、降级警告默认拒绝、同版询问重装）
  if RegQueryStringValue(HKLM, UninstallKey, 'DisplayVersion', OldVer) or
     RegQueryStringValue(HKLM, NSISUninstallKey, 'DisplayVersion', OldVer) then
  begin
    if not ShouldProceedWithOldVersion(OldVer) then
    begin
      Result := CustomMessage('InstallCancelled');
      Exit;
    end;

    // 2. 运行旧版卸载器（不弹旧 UI），清理旧服务、旧文件与旧注册表键
    if (RegQueryStringValue(HKLM, UninstallKey, 'UninstallString', OldUninst) or
        RegQueryStringValue(HKLM, NSISUninstallKey, 'UninstallString', OldUninst)) and
       (OldUninst <> '') then
    begin
      AddLog('Running old uninstaller: ' + OldUninst);
      if Pos('unins000.exe', LowerCase(OldUninst)) > 0 then
      begin
        Exec(OldUninst, '/VERYSILENT /SUPPRESSMSGBOX /NORESTART', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      end
      else
      begin
        // NSIS 卸载器: /S 静默、_?= 指定旧安装目录（不删自身，与 NSI 原逻辑一致）
        Exec(OldUninst, '/S _?=' + ExtractFilePath(OldUninst), '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
      end;
      // 旧 NSIS 卸载器残留的 uninst.exe（Inno 卸载器名为 unins000.exe，不会覆盖它）
      if FileExists(ExpandConstant('{app}\uninst.exe')) then
        DeleteFile(ExpandConstant('{app}\uninst.exe'));
    end;

    // 3. NSIS 旧键残留清理（卸载器文件已不存在、键却还在时）
    if RegQueryStringValue(HKLM, NSISUninstallKey, 'UninstallString', OldUninst) and
       (OldUninst <> '') and not FileExists(OldUninst) then
      RegDeleteKeyIncludingSubkeys(HKLM, NSISUninstallKey);
  end;

  // 4. 静默模式: 等待旧 silanes64.exe 完全退出（最长 30 秒）
  if WizardSilent then
  begin
    AddLog('Waiting for silanes64.exe to exit...');
    WaitForSilanesExit(30);
  end;

  // 5. 旧安装目录与本次目标不同: 删除旧目录残留（改目录更新时旧 exe/卸载器留在旧目录）
  if RegQueryStringValue(HKLM, UninstallKey, 'InstallLocation', OldDir) or
     RegQueryStringValue(HKLM, NSISUninstallKey, 'InstallLocation', OldDir) then
  begin
    if (OldDir <> '') and (Pos('Silanes', OldDir) > 0) and
       (CompareText(OldDir, ExpandConstant('{app}')) <> 0) then
    begin
      AddLog('Removing old install directory: ' + OldDir);
      DelTree(OldDir, True, True, True);
    end;
  end;

  AddLog('Cleanup done.');
end;

// ── 服务更新程序注册：文件复制与注册表写入完成后调用（ssPostInstall）──
procedure ConfigureService;
begin
  // 注册开机服务更新程序以在下次重启后升级服务宿主
  AddLog('Registering boot-time service updater...');
  if not RunSilanesCommand('-internal --install-updater', CustomMessage('UpdaterRegisterFail')) then
    Abort;
  // 添加到系统 PATH（卸载/升级不删除，与 NSI 一致）
  AddToPath;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if CurStep = ssInstall then
    AddLog('Starting installation...')
  else if CurStep = ssPostInstall then
    ConfigureService
  else if CurStep = ssDone then
  begin
    AddLog('Installation complete.');
    // 非静默: 重启提示（与 NSI 一致，安装完成即询问是否重启）
    if not WizardSilent then
      if MsgBox(CustomMessage('RebootPrompt'), mbConfirmation, MB_YESNO) = IDYES then
        Exec(ExpandConstant('{sys}\shutdown.exe'), '/r /t 0', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  end;
end;

// ── 卸载：移除服务更新程序；失败弹「终止 / 重试 / 忽略」──
function InitializeUninstall: Boolean;
begin
  Result := True;

  // 移除服务更新程序（失败弹窗；Abort → 终止卸载，Ignore → 继续）
  if not RunSilanesCommand('-internal --uninstall-updater', CustomMessage('UpdaterRemoveFail')) then
  begin
    Result := False;
    Exit;
  end;

  // 等待 silanes64.exe 完全退出，避免删除文件时被占用
  WaitForSilanesExit(20);
end;

// ── 完成页文档：按当前语言返回文档路径 ──
function GetDocPath(Param: String): String;
begin
  if ActiveLanguage = 'chinesesimp' then
    Result := ExpandConstant('{app}\Docs\README_CN.html')
  else
    Result := ExpandConstant('{app}\Docs\README_EN.html');
end;
