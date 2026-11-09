Unicode true
!include "MUI2.nsh"
!ifndef VERSION
  !define VERSION "0.0.0"
!endif
Name "Power Control ${VERSION}"
OutFile "..\target\release\Power-Control-${VERSION}-x64-setup.exe"
InstallDir "$PROGRAMFILES64\Power Control"
RequestExecutionLevel admin
SetCompressor /SOLID lzma

!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Section "Power Control" SEC_MAIN
    SetShellVarContext all
    SetOutPath "$INSTDIR"
    File "..\target\release\power-controle.exe"
    CreateDirectory "$SMPROGRAMS\Power Control"
    CreateShortcut "$SMPROGRAMS\Power Control\Power Control.lnk" "$INSTDIR\power-controle.exe"
    WriteUninstaller "$INSTDIR\Uninstall.exe"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PowerControl" "DisplayName" "Power Control"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PowerControl" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PowerControl" "InstallLocation" "$INSTDIR"
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PowerControl" "NoModify" 1
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PowerControl" "NoRepair" 1
SectionEnd

Section "Uninstall"
    MessageBox MB_OKCANCEL|MB_ICONINFORMATION "Before uninstalling, disable Start with Windows and quit Power Control using its tray menu. Settings and BIOS credentials are retained." IDOK proceed
    Abort
    proceed:
    SetShellVarContext all
    Delete "$SMPROGRAMS\Power Control\Power Control.lnk"
    RMDir "$SMPROGRAMS\Power Control"
    Delete "$INSTDIR\power-controle.exe"
    Delete "$INSTDIR\Uninstall.exe"
    RMDir "$INSTDIR"
    DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\PowerControl"
SectionEnd
