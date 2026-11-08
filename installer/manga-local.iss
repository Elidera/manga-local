; Inno Setup script for Manga Local. Built by CI:
;   ISCC.exe /DAppVersion=0.1.0 /DDist=<path to dist> manga-local.iss
; Installs per-user (no admin rights) into %LOCALAPPDATA%\Programs\MangaLocal.

#ifndef AppVersion
  #define AppVersion "0.2.0"
#endif
#ifndef Dist
  #define Dist "..\dist"
#endif

[Setup]
AppId={{FD9BC92E-7D97-487D-A7E5-BC0335CDE437}
AppName=Manga Local
AppVersion={#AppVersion}
AppPublisher=Manga Local contributors
DefaultDirName={localappdata}\Programs\MangaLocal
DefaultGroupName=Manga Local
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
OutputBaseFilename=MangaLocal-Setup-{#AppVersion}-x64
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\manga-local.exe
SetupIconFile=..\assets\icon.ico
DisableProgramGroupPage=yes
; Close a running copy before replacing files (otherwise the old exe stays locked).
CloseApplications=force
RestartApplications=no

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[Types]
Name: "gpu"; Description: "Видеокарта (Vulkan) — рекомендуется"
Name: "cpu"; Description: "Только процессор (старые ПК без поддержки Vulkan)"

[Components]
Name: "app"; Description: "Manga Local"; Types: gpu cpu; Flags: fixed
Name: "vulkan"; Description: "Нейросетевой движок llama.cpp (Vulkan: NVIDIA / AMD / Intel)"; Types: gpu; Flags: exclusive
Name: "cpu"; Description: "Нейросетевой движок llama.cpp (только CPU)"; Types: cpu; Flags: exclusive

[Tasks]
Name: "desktopicon"; Description: "Ярлык на рабочем столе"
Name: "ocr"; Description: "Установить модули распознавания текста Windows — корейский, японский, китайский и английский (нужны права администратора, ~40 МБ)"

[Files]
Source: "{#Dist}\manga-local.exe"; DestDir: "{app}"; Components: app; Flags: ignoreversion
Source: "{#Dist}\config.default.toml"; DestDir: "{app}"; Components: app; Flags: ignoreversion
Source: "{#Dist}\README.md"; DestDir: "{app}"; Components: app; Flags: ignoreversion
Source: "{#Dist}\LICENSE"; DestDir: "{app}"; Components: app; Flags: ignoreversion
Source: "{#Dist}\LLAMA_VERSION.txt"; DestDir: "{app}"; Components: app; Flags: ignoreversion
Source: "{#Dist}\glossary\*"; DestDir: "{app}\glossary"; Components: app; Flags: ignoreversion recursesubdirs
Source: "{#Dist}\bench\*"; DestDir: "{app}\bench"; Components: app; Flags: ignoreversion recursesubdirs
Source: "{#Dist}\llama-vulkan\*"; DestDir: "{app}\llama"; Components: vulkan; Flags: ignoreversion recursesubdirs
Source: "{#Dist}\llama-cpu\*"; DestDir: "{app}\llama"; Components: cpu; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{group}\Manga Local"; Filename: "{app}\manga-local.exe"
Name: "{group}\Папка данных Manga Local"; Filename: "{localappdata}\MangaLocal"
Name: "{autodesktop}\Manga Local"; Filename: "{app}\manga-local.exe"; Tasks: desktopicon

[Run]
; Windows OCR language components are installed from [Code] (CurStepChanged), so a
; refused UAC prompt or a failed install is reported instead of passing silently.
Filename: "{app}\manga-local.exe"; Description: "Запустить Manga Local"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "taskkill.exe"; Parameters: "/F /IM llama-server.exe"; Flags: runhidden; RunOnceId: "killllama"

[Code]
// Close a running Manga Local before files are replaced; its llama-server child
// exits with it (job object). Without this the old exe stayed locked.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM manga-local.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Sleep(800);
  Result := '';
end;

// Windows OCR language components (ko-KR, ja-JP, zh-CN, en-US): elevated, hidden PowerShell.
// Exit code 0 = all installed (checked afterwards, not just "no error").
procedure InstallOcr();
var
  ResultCode: Integer;
  Params: String;
begin
  Params := '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -Command "' +
    '$ok = $true; ' +
    'foreach ($n in @(''Language.OCR~~~ko-KR~0.0.1.0'', ''Language.OCR~~~ja-JP~0.0.1.0'', ''Language.OCR~~~zh-CN~0.0.1.0'', ''Language.OCR~~~en-US~0.0.1.0'')) { ' +
    'try { Add-WindowsCapability -Online -Name $n -ErrorAction Stop | Out-Null } catch { }; ' +
    'if ((Get-WindowsCapability -Online -Name $n).State -ne ''Installed'') { $ok = $false } }; ' +
    'if ($ok) { exit 0 } else { exit 3 }"';
  WizardForm.StatusLabel.Caption := 'Установка модулей распознавания текста Windows…';
  if not ShellExec('runas', 'powershell.exe', Params, '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
  begin
    Log('OCR: elevation refused or failed, code ' + IntToStr(ResultCode));
    if not WizardSilent then
      MsgBox('Модули распознавания текста Windows не установлены: не получено разрешение администратора.' + #13#10 + #13#10 +
             'Manga Local покажет в панели, как установить их вручную (Параметры → Время и язык → Язык).', mbInformation, MB_OK);
  end
  else if ResultCode <> 0 then
  begin
    Log('OCR: install finished with code ' + IntToStr(ResultCode));
    if not WizardSilent then
      MsgBox('Не удалось установить модули распознавания текста Windows (код ' + IntToStr(ResultCode) + ').' + #13#10 + #13#10 +
             'Часто помогает перезагрузка и повторная установка. Manga Local покажет в панели, как установить их вручную.', mbInformation, MB_OK);
  end
  else
    Log('OCR: language components installed');
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('ocr') then
    InstallOcr();
end;

function InitializeUninstall(): Boolean;
begin
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    if MsgBox('Удалить также скачанные модели, кэш переводов, исправления и настройки?' + #13#10 +
              '(' + ExpandConstant('{localappdata}\MangaLocal') + ')', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
      DelTree(ExpandConstant('{localappdata}\MangaLocal'), True, True, True);
end;
