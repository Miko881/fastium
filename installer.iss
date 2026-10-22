; Inno Setup Script for Fastium Browser
#define MyAppName "Fastium"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "Fastium"
#define MyAppExeName "fastium.exe"

[Setup]
AppId={{D37F2C54-71F0-4C9D-9481-9FB3B2CA555F}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DisableProgramGroupPage=yes
LicenseFile=LICENSE
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=dist
OutputBaseFilename=FastiumSetup
SetupIconFile=assets\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "polish"; MessagesFile: "compiler:Languages\Polish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "target\x86_64-pc-windows-msvc\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[Code]
var
  ErrorCode: Integer;

function IsWebView2Installed(): Boolean;
var
  Dummy: String;
begin
  // 1. Sprawdzenie katalogow instalacji WebView2 w Windows 10 i 11
  if DirExists(ExpandConstant('{commonpf32}\Microsoft\EdgeWebView\Application')) or
     DirExists(ExpandConstant('{commonpf64}\Microsoft\EdgeWebView\Application')) or
     DirExists('C:\Program Files (x86)\Microsoft\EdgeWebView\Application') or
     DirExists('C:\Program Files\Microsoft\EdgeWebView\Application') then
  begin
    Result := True;
    Exit;
  end;

  // 2. Sprawdzenie kluczy rejestru Uninstall dla Microsoft EdgeWebView
  if RegKeyExists(HKEY_LOCAL_MACHINE, 'SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Microsoft EdgeWebView') or
     RegKeyExists(HKEY_LOCAL_MACHINE, 'SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Microsoft EdgeWebView') or
     RegKeyExists(HKEY_CURRENT_USER, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\Microsoft EdgeWebView') then
  begin
    Result := True;
    Exit;
  end;

  // 3. Sprawdzenie kluczy aktualizacji EdgeUpdate
  if RegQueryStringValue(HKEY_LOCAL_MACHINE, 'SOFTWARE\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BCD-66C352E4E4C8}', 'pv', Dummy) or
     RegQueryStringValue(HKEY_LOCAL_MACHINE, 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BCD-66C352E4E4C8}', 'pv', Dummy) or
     RegQueryStringValue(HKEY_CURRENT_USER, 'Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BCD-66C352E4E4C8}', 'pv', Dummy) then
  begin
    Result := True;
    Exit;
  end;

  // 4. Sprawdzenie domyslnego Microsoft Edge (jako awaryjny fallback dla silnika)
  if DirExists('C:\Program Files (x86)\Microsoft\Edge\Application') or
     DirExists('C:\Program Files\Microsoft\Edge\Application') then
  begin
    Result := True;
    Exit;
  end;

  Result := False;
end;

function InitializeSetup(): Boolean;
begin
  Result := True;
  if not IsWebView2Installed() then
  begin
    if MsgBox('Fastium wymaga środowiska Microsoft Edge WebView2 Runtime, które nie zostało wykryte w Twoim systemie.' + #13#10 +
              'Większość nowszych systemów Windows 10 i 11 ma je zainstalowane fabrycznie.' + #13#10#13#10 +
              'Czy chcesz otworzyć stronę pobierania WebView2?', mbConfirmation, MB_YESNO) = IDYES then
    begin
      ShellExec('open', 'https://go.microsoft.com/fwlink/p/?LinkId=2124703', '', '', SW_SHOW, ewNoWait, ErrorCode);
    end;
  end;
end;
