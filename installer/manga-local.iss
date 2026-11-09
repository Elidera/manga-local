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
Name: "ocr"; Description: "Установить модули распознавания текста Windows — корейский, японский, китайский и английский (нужны права администратора; загрузка из Windows Update — 5–20 минут, уже установленные пропускаются)"

[Files]
Source: "{#Dist}\manga-local.exe"; DestDir: "{app}"; Components: app; Flags: ignoreversion
Source: "{#Dist}\config.default.toml"; DestDir: "{app}"; Components: app; Flags: ignoreversion
Source: "{#Dist}\README.md"; DestDir: "{app}"; Components: app; Flags: ignoreversion
Source: "{#Dist}\LICENSE"; DestDir: "{app}"; Components: app; Flags: ignoreversion
Source: "{#Dist}\LLAMA_VERSION.txt"; DestDir: "{app}"; Components: app; Flags: ignoreversion
Source: "{#Dist}\glossary\*"; DestDir: "{app}\glossary"; Components: app; Flags: ignoreversion recursesubdirs
Source: "{#Dist}\bench\*"; DestDir: "{app}\bench"; Components: app; Flags: ignoreversion recursesubdirs
Source: "{#Dist}\fonts\*"; DestDir: "{app}\fonts"; Components: app; Flags: ignoreversion recursesubdirs
Source: "{#Dist}\extension\*"; DestDir: "{app}\extension"; Components: app; Flags: ignoreversion recursesubdirs
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

// Windows OCR language components (ko-KR, ja-JP, zh-CN, en-US).
// They come from Windows Update and can take 5-20 minutes, so an elevated hidden
// PowerShell does the work while the wizard shows which language is being
// downloaded and how long it has been. Already installed languages are skipped.
function OcrLangName(Code: String): String;
begin
  if Code = 'ko-KR' then Result := 'корейский'
  else if Code = 'ja-JP' then Result := 'японский'
  else if Code = 'zh-CN' then Result := 'китайский'
  else Result := 'английский';
end;

// About one second, while the wizard window stays responsive (Exec pumps messages).
procedure WaitASecond();
var
  R: Integer;
begin
  Exec(ExpandConstant('{sys}\ping.exe'), '-n 2 127.0.0.1', '', SW_HIDE, ewWaitUntilTerminated, R);
end;

procedure InstallOcr();
var
  ResultCode, Waited, Failed: Integer;
  Script, Progress: String;
  S: AnsiString;
  Started, Done: Boolean;
begin
  Script := ExpandConstant('{tmp}\ocr-langs.ps1');
  Progress := ExpandConstant('{tmp}\ocr-progress.txt');
  SaveStringToFile(Script,
    'param([string]$Progress)' + #13#10 +
    '$failed = 0; $i = 0' + #13#10 +
    'foreach ($l in @(''ko-KR'', ''ja-JP'', ''zh-CN'', ''en-US'')) {' + #13#10 +
    '  $i++; $n = "Language.OCR~~~$l~0.0.1.0"' + #13#10 +
    '  Set-Content -Path $Progress -Value "$i $l" -Encoding ASCII' + #13#10 +
    '  try { if ((Get-WindowsCapability -Online -Name $n).State -ne ''Installed'') { Add-WindowsCapability -Online -Name $n -ErrorAction Stop | Out-Null } } catch { }' + #13#10 +
    '  if ((Get-WindowsCapability -Online -Name $n).State -ne ''Installed'') { $failed++ }' + #13#10 +
    '}' + #13#10 +
    'Set-Content -Path $Progress -Value "done $failed" -Encoding ASCII' + #13#10, False);
  DeleteFile(Progress);
  WizardForm.StatusLabel.Caption := 'Установка распознавания текста Windows…';
  WizardForm.FilenameLabel.Caption := 'Windows попросит разрешение администратора.';
  if not ShellExec('runas', 'powershell.exe',
      '-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File "' + Script + '" "' + Progress + '"',
      '', SW_HIDE, ewNoWait, ResultCode) then
  begin
    Log('OCR: elevation refused or failed, code ' + IntToStr(ResultCode));
    WizardForm.FilenameLabel.Caption := '';
    if not WizardSilent then
      MsgBox('Модули распознавания текста Windows не установлены: не получено разрешение администратора.' + #13#10 + #13#10 +
             'Manga Local покажет в панели, как установить их вручную (Параметры → Время и язык → Язык).', mbInformation, MB_OK);
    exit;
  end;

  WizardForm.ProgressGauge.Style := npbstMarquee;
  Waited := 0;
  Started := False;
  Done := False;
  Failed := 0;
  while not Done do
  begin
    WaitASecond();
    Waited := Waited + 1;
    if LoadStringFromFile(Progress, S) then
    begin
      Started := True;
      S := Trim(S);
      if Copy(S, 1, 4) = 'done' then
      begin
        Done := True;
        Failed := StrToIntDef(Trim(Copy(S, 5, 10)), 1);
      end
      else if Length(S) >= 7 then
        WizardForm.StatusLabel.Caption := 'Скачивание распознавания текста Windows: ' +
          OcrLangName(Copy(S, 3, 5)) + ' (' + Copy(S, 1, 1) + ' из 4)…';
    end;
    if not Done then
    begin
      if (not Started) and (Waited > 180) then break;  // the script never started
      if Waited > 45 * 60 then break;
      WizardForm.FilenameLabel.Caption := 'Загрузка из Windows Update, обычно 5–20 минут. Окно не зависло. Прошло: ' +
        IntToStr(Waited div 60) + ' мин.';
    end;
  end;
  WizardForm.ProgressGauge.Style := npbstNormal;
  WizardForm.FilenameLabel.Caption := '';

  if not Done then
  begin
    Log('OCR: gave up waiting after ' + IntToStr(Waited) + ' s');
    if not WizardSilent then
      MsgBox('Распознавание текста Windows устанавливается дольше обычного. Установка продолжится сама в фоне.' + #13#10 + #13#10 +
             'Если перевод не заработает, перезагрузите компьютер — Manga Local покажет в панели, чего не хватает.', mbInformation, MB_OK);
  end
  else if Failed > 0 then
  begin
    Log('OCR: ' + IntToStr(Failed) + ' language(s) not installed');
    if not WizardSilent then
      MsgBox('Не удалось установить распознавание текста для ' + IntToStr(Failed) + ' из 4 языков.' + #13#10 + #13#10 +
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
