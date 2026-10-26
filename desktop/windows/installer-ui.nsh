; Native presentation only. Installation and upgrade logic remains in installer.nsi.
!include nsDialogs.nsh
!define MUI_BGCOLOR "10231E"
!define MUI_TEXTCOLOR "EFECE5"
!define MUI_INSTFILESPAGE_COLORS "EFECE5 1B392E"
!define MUI_CUSTOMFUNCTION_GUIINIT PanitaGuiInit
!define MUI_CUSTOMFUNCTION_UNGUIINIT un.PanitaGuiInit
Var PanitaDpi
Var PanitaDialog
Var PanitaIcon
Var PanitaTitleFont
Var PanitaBrandFont
Var PanitaPath
Var PanitaDesktopCheckbox
Var PanitaDesktopWanted

; Never force-close a launcher that may own live games. Probe the exact file.
!macro PANITA_REQUIRE_IDLE EXECUTABLE
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $R0
  !insertmacro RestartManager_StartSession $R0
  ${If} $R0 == ""
    MessageBox MB_OK|MB_ICONSTOP "No se pudo comprobar si Panita Client está en uso. Cierra el launcher e inténtalo de nuevo." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  !insertmacro RestartManager_RegisterFile $R0 "${EXECUTABLE}"
  ${If} $0 != 0
    !insertmacro RestartManager_EndSession $R0
    MessageBox MB_OK|MB_ICONSTOP "No se pudo comprobar si Panita Client está en uso. Cierra el launcher e inténtalo de nuevo." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  StrCpy $1 0
  StrCpy $2 0
  System::Call 'RSTRTMGR::RmGetList(p R0, *i .r1, *i .r2, p 0, *i .r3) i .r4'
  !insertmacro RestartManager_EndSession $R0
  ${If} $4 != 0
    MessageBox MB_OK|MB_ICONSTOP "Cierra todas las instancias y Panita Client antes de continuar." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  Pop $R0
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend

!macro PANITA_REQUIRE_IDLE_INSTALLATION
  ReadRegStr $OldMainBinaryName SHCTX "${UNINSTKEY}" "MainBinaryName"
  ${If} $OldMainBinaryName != ""
  ${AndIf} $OldMainBinaryName != "panita-desktop.exe"
  ${AndIf} $OldMainBinaryName != "panita-client.exe"
    MessageBox MB_OK|MB_ICONSTOP "La instalación anterior no se reconoce. Conserva sus archivos y revisa la instalación antes de continuar." /SD IDOK
    SetErrorLevel 2
    Abort
  ${EndIf}
  ${If} $OldMainBinaryName != ""
    Push $4
    ReadRegStr $4 SHCTX "${MANUPRODUCTKEY}" ""
    ${If} $4 != ""
      !insertmacro PANITA_REQUIRE_IDLE "$4\$OldMainBinaryName"
    ${EndIf}
    Pop $4
  ${EndIf}
  !insertmacro PANITA_REQUIRE_IDLE "$INSTDIR\${MAINBINARYNAME}.exe"
!macroend

; Coordinates are logical pixels; preserve registers used by upstream pages.
!macro PANITA_RECT HANDLE X Y WIDTH HEIGHT
  Push $0
  Push $1
  Push $2
  Push $3
  IntOp $0 ${X} * $PanitaDpi
  IntOp $0 $0 / 96
  IntOp $1 ${Y} * $PanitaDpi
  IntOp $1 $1 / 96
  IntOp $2 ${WIDTH} * $PanitaDpi
  IntOp $2 $2 / 96
  IntOp $3 ${HEIGHT} * $PanitaDpi
  IntOp $3 $3 / 96
  System::Call 'user32::SetWindowPos(p ${HANDLE}, p 0, i r0, i r1, i r2, i r3, i 4)'
  Pop $3
  Pop $2
  Pop $1
  Pop $0
!macroend

!macro PANITA_PAGE_LAYOUT HANDLE
  SetCtlColors ${HANDLE} EFECE5 10231E
  ShowWindow $mui.Header.Text ${SW_HIDE}
  ShowWindow $mui.Header.SubText ${SW_HIDE}
  ShowWindow $mui.Header.Image ${SW_HIDE}
  ShowWindow $mui.Header.Background ${SW_HIDE}
  ShowWindow $mui.Branding.Text ${SW_HIDE}
  ShowWindow $mui.Branding.Background ${SW_HIDE}
  ShowWindow $mui.Line.Standard ${SW_HIDE}
  ShowWindow $mui.Line.FullWindow ${SW_HIDE}
  Push $0
  GetDlgItem $0 $HWNDPARENT 1036
  ShowWindow $0 ${SW_HIDE}
  Pop $0
  !insertmacro PANITA_RECT ${HANDLE} 24 24 386 490
  ShowWindow $mui.Button.Back ${SW_HIDE}
  !insertmacro PANITA_RECT $mui.Button.Cancel 24 546 104 44
  !insertmacro PANITA_RECT $mui.Button.Next 142 546 268 44
!macroend

!macro PANITA_BRAND
  ${NSD_CreateIcon} 0 0 32u 32u ""
  Pop $0
  SendMessage $0 ${STM_SETICON} $PanitaIcon 0
  ${NSD_CreateLabel} 42u 3u -42u 25u "Panita Client"
  Pop $0
  SetCtlColors $0 EFECE5 10231E
  SendMessage $0 ${WM_SETFONT} $PanitaBrandFont 1
  ${NSD_CreateLabel} 0 43u 100% 14u "Beta · ${VERSION}"
  Pop $0
  SetCtlColors $0 F2CF74 10231E
!macroend

!macro PANITA_HEADING TITLE DESCRIPTION
  ${NSD_CreateLabel} 0 65u 100% 52u "${TITLE}"
  Pop $0
  SendMessage $0 ${WM_SETFONT} $PanitaTitleFont 1
  SetCtlColors $0 EFECE5 10231E
  ${NSD_CreateLabel} 0 123u 100% 40u "${DESCRIPTION}"
  Pop $0
  SetCtlColors $0 C4D0C8 10231E
!macroend

!macro PANITA_UI_FUNCTIONS
  !insertmacro PANITA_GUI_INIT ""
  !insertmacro PANITA_GUI_INIT "un."

  Function PanitaWelcome
    Call SkipIfPassive
    nsDialogs::Create 1018
    Pop $PanitaDialog
    ${If} $PanitaDialog == error
      Abort
    ${EndIf}
    !insertmacro PANITA_PAGE_LAYOUT $PanitaDialog
    !insertmacro PANITA_BRAND
    ReadRegStr $0 SHCTX "${UNINSTKEY}" "DisplayVersion"
    ${If} $0 == ""
      !insertmacro PANITA_HEADING "Todo listo para empezar" "Instala Panita Client y encuentra tu próxima aventura."
      SendMessage $mui.Button.Next ${WM_SETTEXT} 0 "STR:Instalar"
    ${Else}
      !insertmacro PANITA_HEADING "Tu próxima aventura continúa" "Actualiza Panita Client. Tus cuentas, ajustes, instancias y mundos se conservarán."
      SendMessage $mui.Button.Next ${WM_SETTEXT} 0 "STR:Actualizar"
    ${EndIf}
    ${NSD_CreateLabel} 0 174u 100% 32u "Se instalará para tu usuario. No necesitas instalar Java por separado."
    Pop $0
    SetCtlColors $0 C4D0C8 1B392E
    ${NSD_CreateLabel} 0 222u 100% 26u "Carpeta: $INSTDIR"
    Pop $PanitaPath
    SetCtlColors $PanitaPath C4D0C8 10231E
    ${NSD_CreateButton} 0 252u 100% 24u "Cambiar carpeta"
    Pop $0
    ${NSD_OnClick} $0 PanitaChooseDirectory
    ${NSD_CreateCheckbox} 0 282u 100% 16u "Crear acceso directo en el escritorio"
    Pop $PanitaDesktopCheckbox
    ; The themed checkbox ignores custom foreground colors on a dark dialog.
    System::Call 'uxtheme::SetWindowTheme(p $PanitaDesktopCheckbox, w "", w "")'
    SetCtlColors $PanitaDesktopCheckbox C4D0C8 10231E
    ${If} $NoShortcutMode != 1
      ${NSD_Check} $PanitaDesktopCheckbox
    ${EndIf}
    nsDialogs::Show
  FunctionEnd

  Function PanitaWelcomeLeave
    !insertmacro PANITA_REQUIRE_IDLE_INSTALLATION
    ${NSD_GetState} $PanitaDesktopCheckbox $PanitaDesktopWanted
  FunctionEnd

  Function PanitaChooseDirectory
    nsDialogs::SelectFolderDialog "Carpeta de instalación" "$INSTDIR"
    Pop $0
    ${If} $0 != error
      StrCpy $INSTDIR $0
      ${NSD_SetText} $PanitaPath "Carpeta: $INSTDIR"
    ${EndIf}
  FunctionEnd

  Function PanitaFinish
    Call SkipIfPassive
    nsDialogs::Create 1018
    Pop $PanitaDialog
    ${If} $PanitaDialog == error
      Abort
    ${EndIf}
    !insertmacro PANITA_PAGE_LAYOUT $PanitaDialog
    !insertmacro PANITA_BRAND
    !insertmacro PANITA_HEADING "Panita Client está listo" "Ya puedes abrir el launcher y elegir una instancia."
    ${NSD_CreateLabel} 0 174u 100% 32u "Tus aventuras se mantienen separadas. Las actualizaciones de instancias son siempre opcionales."
    Pop $0
    SetCtlColors $0 C4D0C8 1B392E
    SendMessage $mui.Button.Next ${WM_SETTEXT} 0 "STR:Abrir"
    SendMessage $mui.Button.Cancel ${WM_SETTEXT} 0 "STR:Cerrar"
    EnableWindow $mui.Button.Cancel 1
    nsDialogs::Show
  FunctionEnd

  Function PanitaFinishLeave
    Call RunMainBinary
  FunctionEnd

  Function un.PanitaConfirm
    Call un.SkipIfPassive
    nsDialogs::Create 1018
    Pop $PanitaDialog
    ${If} $PanitaDialog == error
      Abort
    ${EndIf}
    !insertmacro PANITA_PAGE_LAYOUT $PanitaDialog
    !insertmacro PANITA_BRAND
    !insertmacro PANITA_HEADING "¿Desinstalar Panita Client?" "Se eliminará el launcher de este equipo."
    ${NSD_CreateLabel} 0 174u 100% 65u "Tus cuentas guardadas, ajustes, instancias, mundos y capturas permanecerán en este equipo. Puedes reinstalar Panita Client cuando quieras."
    Pop $0
    SetCtlColors $0 C4D0C8 1B392E
    SendMessage $mui.Button.Next ${WM_SETTEXT} 0 "STR:Desinstalar"
    nsDialogs::Show
  FunctionEnd

  Function PanitaProgress
    !insertmacro PANITA_PROGRESS_LAYOUT
  FunctionEnd

  Function un.PanitaProgress
    !insertmacro PANITA_PROGRESS_LAYOUT
  FunctionEnd
!macroend

!macro PANITA_GUI_INIT PREFIX
  Function ${PREFIX}PanitaGuiInit
    System::Call 'user32::GetDpiForWindow(p $HWNDPARENT) i .r0'
    StrCpy $PanitaDpi $0
    ${If} $PanitaDpi < 96
      StrCpy $PanitaDpi 96
    ${EndIf}
    IntOp $0 450 * $PanitaDpi
    IntOp $0 $0 / 96
    IntOp $1 650 * $PanitaDpi
    IntOp $1 $1 / 96
    System::Call 'user32::SetWindowPos(p $HWNDPARENT, p 0, i 0, i 0, i r0, i r1, i 6)'
    SetCtlColors $HWNDPARENT EFECE5 10231E
    CreateFont $PanitaTitleFont "Segoe UI" 20 700
    CreateFont $PanitaBrandFont "Segoe UI" 17 700
    System::Call 'shell32::ExtractIconExW(w "$EXEPATH", i 0, *p .r0, p 0, i 1)'
    StrCpy $PanitaIcon $0
    ; Replace the wizard header and separators with the page's own branding.
    ShowWindow $mui.Header.Text ${SW_HIDE}
    ShowWindow $mui.Header.SubText ${SW_HIDE}
    ShowWindow $mui.Header.Background ${SW_HIDE}
    ShowWindow $mui.Branding.Text ${SW_HIDE}
    ShowWindow $mui.Branding.Background ${SW_HIDE}
    ShowWindow $mui.Line.Standard ${SW_HIDE}
    ShowWindow $mui.Line.FullWindow ${SW_HIDE}
  FunctionEnd
!macroend

!macro PANITA_PROGRESS_LAYOUT
  FindWindow $PanitaDialog "#32770" "" $HWNDPARENT
  !insertmacro PANITA_PAGE_LAYOUT $PanitaDialog
  ; Keep the native progress/log controls and their failure reporting.
  GetDlgItem $0 $PanitaDialog 1006
  !insertmacro PANITA_RECT $0 0 16 386 48
  GetDlgItem $0 $PanitaDialog 1004
  !insertmacro PANITA_RECT $0 0 78 386 12
  GetDlgItem $0 $PanitaDialog 1027
  !insertmacro PANITA_RECT $0 0 108 386 28
  GetDlgItem $0 $PanitaDialog 1016
  !insertmacro PANITA_RECT $0 0 150 386 320
!macroend
