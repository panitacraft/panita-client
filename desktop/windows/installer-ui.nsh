; Native presentation only. Installation and upgrade logic remains in installer.nsi.
!include nsDialogs.nsh
!define PANITA_BRAND_ICON "${__FILEDIR__}\..\icons\icon.ico"
!addplugindir /x86-unicode "${__FILEDIR__}\..\..\target\installer-ui"
!define MUI_BGCOLOR "141414"
!define MUI_TEXTCOLOR "F9FFF8"
!define MUI_INSTFILESPAGE_COLORS "F9FFF8 1B1B1B"
!define MUI_CUSTOMFUNCTION_GUIINIT PanitaGuiInit
!define MUI_CUSTOMFUNCTION_UNGUIINIT un.PanitaGuiInit
!define MUI_CUSTOMFUNCTION_ABORT PanitaUserAbort
!define MUI_CUSTOMFUNCTION_UNABORT un.PanitaUserAbort
Var PanitaDpi
Var PanitaDialog
Var PanitaIcon
Var PanitaTitleFont
Var PanitaBrandFont
Var PanitaBodyFont
Var PanitaPath
Var PanitaDesktopCheckbox
Var PanitaDesktopWanted
Var PanitaIdleResult
Var PanitaPriorDirectory
Var PanitaRemoveDataCheckbox
Var PanitaRemoveDataWanted
Var PanitaHeading
Var PanitaDescription
Var PanitaCardBackground
Var PanitaCardText
Var PanitaDataSummary
Var PanitaEraseStage
Var PanitaEraseAcknowledgement
Var PanitaEraseBack
Var PanitaProgressActive
Var PanitaProgressNotice

; Presentation roles let the plugin measure actual fonts/text instead of keeping
; large fixed-height gaps. Values are transient window properties, not disk data.
!macro PANITA_ROLE HANDLE ROLE
  System::Call 'user32::SetPropW(p ${HANDLE}, w "PanitaRole", p ${ROLE})'
!macroend

; Stay within the branded page. Silent runs only log and retain their error code.
!macro PANITA_REPORT MESSAGE
  DetailPrint "${MESSAGE}"
  ${IfNot} ${Silent}
  ${AndIf} $PassiveMode != 1
    System::Call 'user32::MessageBeep(i 0x10)'
  ${EndIf}
  Push $0
  ${If} $PanitaProgressActive == 1
    System::Call 'user32::SetPropW(p $HWNDPARENT, w "PanitaPhase", p 2)'
    System::Call 'user32::IsWindow(p $PanitaProgressNotice) i .r0'
    ${If} $0 != 0
      SendMessage $PanitaProgressNotice ${WM_SETTEXT} 0 "STR:${MESSAGE}"
      ShowWindow $PanitaProgressNotice ${SW_SHOW}
      GetDlgItem $0 $PanitaDialog 1006
      ShowWindow $0 ${SW_HIDE}
      GetDlgItem $0 $PanitaDialog 1004
      ShowWindow $0 ${SW_HIDE}
      GetDlgItem $0 $PanitaDialog 1016
      !insertmacro PANITA_RECT $0 0 214 402 304
    ${EndIf}
  ${Else}
    System::Call 'user32::IsWindow(p $PanitaDescription) i .r0'
    ${If} $0 != 0
      SendMessage $PanitaDescription ${WM_SETTEXT} 0 "STR:${MESSAGE}"
      SetCtlColors $PanitaDescription FCA5A5 141414
    ${EndIf}
  ${EndIf}
  Pop $0
  PanitaUi::Apply /NOUNLOAD
!macroend

; Never force-close a launcher that may own live games. Probe the exact file.
!macro PANITA_REQUIRE_IDLE EXECUTABLE
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $R0
  StrCpy $PanitaIdleResult "unavailable"
  !insertmacro RestartManager_StartSession $R0
  ${If} $R0 != ""
    !insertmacro RestartManager_RegisterFile $R0 "${EXECUTABLE}"
    ${If} $0 == 0
      StrCpy $1 0
      StrCpy $2 0
      System::Call 'RSTRTMGR::RmGetList(p R0, *i .r1, *i .r2, p 0, *i .r3) i .r4'
      ${If} $4 == 0
        StrCpy $PanitaIdleResult ""
      ${ElseIf} $4 == ${ERROR_MORE_DATA}
        StrCpy $PanitaIdleResult "busy"
      ${EndIf}
    ${EndIf}
    !insertmacro RestartManager_EndSession $R0
  ${EndIf}
  Pop $R0
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
  ; Restore the stack before rejecting a page transition, so retries are safe.
  ${If} $PanitaIdleResult != ""
    ${If} $PanitaIdleResult == "busy"
      !insertmacro PANITA_REPORT "Cierra todas las instancias y Panita Client antes de continuar. Después, vuelve a intentarlo."
    ${Else}
      !insertmacro PANITA_REPORT "No se pudo comprobar si Panita Client está en uso. Cierra el launcher e inténtalo de nuevo."
    ${EndIf}
    SetErrorLevel 2
    Abort
  ${EndIf}
  SetErrorLevel 0
!macroend

!macro PANITA_REQUIRE_IDLE_INSTALLATION
  ReadRegStr $OldMainBinaryName SHCTX "${UNINSTKEY}" "MainBinaryName"
  ${If} $OldMainBinaryName != ""
  ${AndIf} $OldMainBinaryName != "panita-desktop.exe"
  ${AndIf} $OldMainBinaryName != "panita-client.exe"
    !insertmacro PANITA_REPORT "La instalación anterior no se reconoce. Conserva sus archivos y revisa la instalación antes de continuar."
    SetErrorLevel 2
    Abort
  ${EndIf}
  ${If} $OldMainBinaryName != ""
    ReadRegStr $PanitaPriorDirectory SHCTX "${MANUPRODUCTKEY}" ""
    ${If} $PanitaPriorDirectory != ""
      !insertmacro PANITA_REQUIRE_IDLE "$PanitaPriorDirectory\$OldMainBinaryName"
    ${EndIf}
  ${EndIf}
  !insertmacro PANITA_REQUIRE_IDLE "$INSTDIR\${MAINBINARYNAME}.exe"
!macroend

; Coordinates are logical pixels; preserve registers used by upstream pages.
!macro PANITA_RECT HANDLE X Y WIDTH HEIGHT
  ; Capture HANDLE before using $0-$3: callers also pass handles in $0.
  Push $4
  StrCpy $4 ${HANDLE}
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
  System::Call 'user32::SetWindowPos(p r4, p 0, i r0, i r1, i r2, i r3, i 4)'
  Pop $3
  Pop $2
  Pop $1
  Pop $0
  Pop $4
!macroend

!macro PANITA_PAGE_LAYOUT HANDLE
  StrCpy $PanitaProgressActive 0
  System::Call 'user32::SetPropW(p $HWNDPARENT, w "PanitaPhase", p 0)'
  SendMessage ${HANDLE} ${WM_SETFONT} $PanitaBodyFont 1
  SendMessage $mui.Button.Next ${WM_SETFONT} $PanitaBodyFont 1
  SetCtlColors ${HANDLE} F9FFF8 141414
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
  !insertmacro PANITA_RECT ${HANDLE} 24 24 402 484
  ShowWindow $mui.Button.Back ${SW_HIDE}
  ShowWindow $mui.Button.Cancel ${SW_HIDE}
  !insertmacro PANITA_RECT $mui.Button.Next 24 532 402 44
!macroend

!macro PANITA_BRAND PREFIX
  ${NSD_CreateIcon} 0 0 32u 32u ""
  Pop $0
  !insertmacro PANITA_ROLE $0 1
  SetCtlColors $0 F9FFF8 141414
  SendMessage $0 ${STM_SETICON} $PanitaIcon 0
  ${NSD_CreateLabel} 0 0 0 0 "PANITA CLIENT"
  Pop $0
  !insertmacro PANITA_ROLE $0 2
  ${NSD_AddStyle} $0 ${SS_CENTERIMAGE}
  !insertmacro PANITA_RECT $0 64 4 214 40
  SetCtlColors $0 F9FFF8 141414
  SendMessage $0 ${WM_SETFONT} $PanitaBrandFont 1
  ${NSD_CreateButton} 0 0 0 0 "Minimizar"
  Pop $0
  !insertmacro PANITA_ROLE $0 3
  !insertmacro PANITA_RECT $0 280 0 64 28
  ${NSD_CreateButton} 0 0 0 0 "Cerrar"
  Pop $0
  !insertmacro PANITA_ROLE $0 4
  !insertmacro PANITA_RECT $0 350 0 52 28
!macroend

!macro PANITA_HEADING TITLE DESCRIPTION
  ${NSD_CreateLabel} 0 0 0 0 "${TITLE}"
  Pop $0
  StrCpy $PanitaHeading $0
  !insertmacro PANITA_ROLE $0 6
  !insertmacro PANITA_RECT $0 0 110 402 72
  SendMessage $0 ${WM_SETFONT} $PanitaTitleFont 1
  SetCtlColors $0 F9FFF8 141414
  ${NSD_CreateLabel} 0 0 0 0 "${DESCRIPTION}"
  Pop $0
  StrCpy $PanitaDescription $0
  !insertmacro PANITA_ROLE $0 7
  !insertmacro PANITA_RECT $0 0 190 402 68
  SetCtlColors $0 BABABA 141414
!macroend

!macro PANITA_CARD TEXT Y HEIGHT
  ${NSD_CreateLabel} 0 0 0 0 " "
  Pop $0
  StrCpy $PanitaCardBackground $0
  !insertmacro PANITA_ROLE $0 8
  !insertmacro PANITA_RECT $0 0 ${Y} 402 ${HEIGHT}
  SetCtlColors $0 BABABA 1B1B1B
  ${NSD_CreateLabel} 0 0 0 0 "${TEXT}"
  Pop $0
  StrCpy $PanitaCardText $0
  !insertmacro PANITA_ROLE $0 9
  ${NSD_AddStyle} $PanitaCardText ${SS_NOPREFIX}
  !insertmacro PANITA_RECT $0 16 ${Y} 370 ${HEIGHT}
  ; SS_CENTERIMAGE vertically centers a single line; multiline copy uses a
  ; separately inset rectangle so it remains readable at every DPI.
  !define /math PANITA_CARD_TOP ${Y} + 14
  !define /math PANITA_CARD_HEIGHT ${HEIGHT} - 28
  !insertmacro PANITA_RECT $0 16 ${PANITA_CARD_TOP} 370 ${PANITA_CARD_HEIGHT}
  !undef PANITA_CARD_TOP
  !undef PANITA_CARD_HEIGHT
  SetCtlColors $0 BABABA 1B1B1B
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
    !insertmacro PANITA_BRAND ""
    ReadRegStr $0 SHCTX "${UNINSTKEY}" "DisplayVersion"
    ${If} $0 == ""
      !insertmacro PANITA_HEADING "Instala Panita Client" "Tus instancias y amigos, en un solo lugar."
      SendMessage $mui.Button.Next ${WM_SETTEXT} 0 "STR:Instalar"
    ${Else}
      !insertmacro PANITA_HEADING "Actualiza Panita Client" "Instala la nueva versión y sigue jugando."
      SendMessage $mui.Button.Next ${WM_SETTEXT} 0 "STR:Actualizar"
    ${EndIf}
    !insertmacro PANITA_CARD "Solo para tu usuario. Minecraft y Java se descargarán cuando instales una instancia." 270 82
    ${NSD_CreateLabel} 0 0 0 0 "Carpeta: $INSTDIR"
    Pop $PanitaPath
    !insertmacro PANITA_ROLE $PanitaPath 10
    ${NSD_AddStyle} $PanitaPath ${SS_NOPREFIX}
    !insertmacro PANITA_RECT $PanitaPath 0 370 402 44
    SetCtlColors $PanitaPath BABABA 141414
    ${NSD_CreateButton} 0 0 0 0 "Cambiar carpeta"
    Pop $0
    !insertmacro PANITA_ROLE $0 11
    !insertmacro PANITA_RECT $0 0 430 402 40
    ${NSD_OnClick} $0 PanitaChooseDirectory
    ${NSD_CreateCheckbox} 0 0 0 0 "Crear acceso directo en el escritorio"
    Pop $PanitaDesktopCheckbox
    !insertmacro PANITA_ROLE $PanitaDesktopCheckbox 12
    !insertmacro PANITA_RECT $PanitaDesktopCheckbox 0 490 402 28
    ; The themed checkbox ignores custom foreground colors on a dark dialog.
    System::Call 'uxtheme::SetWindowTheme(p $PanitaDesktopCheckbox, w "", w "")'
    SetCtlColors $PanitaDesktopCheckbox BABABA 141414
    ${If} $NoShortcutMode != 1
      ${NSD_Check} $PanitaDesktopCheckbox
    ${EndIf}
    PanitaUi::Apply /NOUNLOAD
    nsDialogs::Show
  FunctionEnd

  Function PanitaWelcomeLeave
    !insertmacro PANITA_REQUIRE_IDLE_INSTALLATION
    ${NSD_GetState} $PanitaDesktopCheckbox $PanitaDesktopWanted
  FunctionEnd

  Function PanitaChooseDirectory
    Pop $0 ; nsDialogs click-control handle.
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
    System::Call 'user32::SetPropW(p $HWNDPARENT, w "PanitaPhase", p 4)'
    !insertmacro PANITA_BRAND ""
    !insertmacro PANITA_HEADING "Panita Client está listo" "Ya puedes abrir el launcher y elegir tu próxima aventura."
    !insertmacro PANITA_CARD "Tus cuentas, mundos y ajustes se conservan al actualizar el launcher." 270 104
    SendMessage $mui.Button.Next ${WM_SETTEXT} 0 "STR:Abrir Panita Client"
    SendMessage $mui.Button.Cancel ${WM_SETTEXT} 0 "STR:Cerrar"
    EnableWindow $mui.Button.Cancel 1
    PanitaUi::Apply /NOUNLOAD
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
    !insertmacro PANITA_BRAND "un."
    !insertmacro PANITA_HEADING "¿Desinstalar Panita Client?" "Se eliminará el launcher de este equipo."
    StrCpy $PanitaRemoveDataWanted 0
    StrCpy $PanitaEraseStage 0
    !insertmacro PANITA_CARD "Tus cuentas, instancias y mundos se conservarán." 270 82
    ${NSD_CreateCheckbox} 0 0 0 0 "Eliminar también mis datos"
    Pop $PanitaRemoveDataCheckbox
    !insertmacro PANITA_ROLE $PanitaRemoveDataCheckbox 13
    !insertmacro PANITA_RECT $PanitaRemoveDataCheckbox 0 376 402 42
    System::Call 'uxtheme::SetWindowTheme(p $PanitaRemoveDataCheckbox, w "", w "")'
    SetCtlColors $PanitaRemoveDataCheckbox F9FFF8 141414
    ${NSD_Uncheck} $PanitaRemoveDataCheckbox
    ${NSD_CreateLabel} 0 0 0 0 "%APPDATA%\PanitaClient: cuentas y sesiones guardadas, ajustes, todas las instancias con mundos, capturas y copias de seguridad, descargas de Minecraft y Java, cachés y registros locales."
    Pop $PanitaDataSummary
    !insertmacro PANITA_ROLE $PanitaDataSummary 14
    !insertmacro PANITA_RECT $PanitaDataSummary 0 432 402 104
    SetCtlColors $PanitaDataSummary BABABA 141414
    ${NSD_CreateCheckbox} 0 0 0 0 "Entiendo que se eliminarán todos estos datos de forma permanente."
    Pop $PanitaEraseAcknowledgement
    !insertmacro PANITA_ROLE $PanitaEraseAcknowledgement 15
    !insertmacro PANITA_RECT $PanitaEraseAcknowledgement 0 450 402 52
    SetCtlColors $PanitaEraseAcknowledgement F9FFF8 141414
    ${NSD_OnClick} $PanitaEraseAcknowledgement un.PanitaAcknowledgeErasure
    ShowWindow $PanitaEraseAcknowledgement ${SW_HIDE}
    ${NSD_CreateButton} 0 0 0 0 "Volver"
    Pop $PanitaEraseBack
    !insertmacro PANITA_ROLE $PanitaEraseBack 16
    !insertmacro PANITA_RECT $PanitaEraseBack 0 512 402 28
    ${NSD_OnClick} $PanitaEraseBack un.PanitaCancelErasure
    ShowWindow $PanitaEraseBack ${SW_HIDE}
    SendMessage $mui.Button.Next ${WM_SETTEXT} 0 "STR:Desinstalar"
    PanitaUi::Apply /NOUNLOAD
    nsDialogs::Show
  FunctionEnd

  Function un.PanitaConfirmLeave
    !insertmacro PANITA_REQUIRE_IDLE "$INSTDIR\${MAINBINARYNAME}.exe"
    ${NSD_GetState} $PanitaRemoveDataCheckbox $PanitaRemoveDataWanted
    ${If} $PanitaRemoveDataWanted == ${BST_CHECKED}
      ${If} $PanitaEraseStage == 0
        StrCpy $PanitaEraseStage 1
        ${NSD_SetText} $PanitaHeading "¿Eliminar todos tus datos?"
        ${NSD_SetText} $PanitaDescription "Esta acción es permanente y no se puede deshacer. Revisa la carpeta y su contenido antes de confirmar."
        SetCtlColors $PanitaDescription FCA5A5 141414
        ${NSD_SetText} $PanitaCardText "$APPDATA\PanitaClient$\r$\n$\r$\nSe eliminará toda esta carpeta: cuentas y sesiones, ajustes, todas las instancias con mundos, capturas, configuraciones y copias de seguridad, descargas de Minecraft y Java, cachés, registros y cualquier otro archivo que contenga."
        !insertmacro PANITA_RECT $PanitaCardBackground 0 270 402 168
        !insertmacro PANITA_RECT $PanitaCardText 16 282 370 144
        ShowWindow $PanitaRemoveDataCheckbox ${SW_HIDE}
        ShowWindow $PanitaDataSummary ${SW_HIDE}
        ${NSD_Uncheck} $PanitaEraseAcknowledgement
        ShowWindow $PanitaEraseAcknowledgement ${SW_SHOW}
        ShowWindow $PanitaEraseBack ${SW_SHOW}
        SendMessage $mui.Button.Next ${WM_SETTEXT} 0 "STR:Eliminar datos y desinstalar"
        EnableWindow $mui.Button.Next 0
        PanitaUi::Apply /NOUNLOAD
        ${NSD_SetFocus} $PanitaEraseBack
        Abort
      ${EndIf}
      ; Check again at the boundary; Enter or a stale command cannot bypass it.
      ${NSD_GetState} $PanitaEraseAcknowledgement $0
      ${If} $0 != ${BST_CHECKED}
        Abort
      ${EndIf}
    ${EndIf}
  FunctionEnd

  Function un.PanitaAcknowledgeErasure
    Pop $0 ; nsDialogs click-control handle.
    ${NSD_GetState} $PanitaEraseAcknowledgement $0
    ${If} $0 == ${BST_CHECKED}
      EnableWindow $mui.Button.Next 1
    ${Else}
      EnableWindow $mui.Button.Next 0
    ${EndIf}
  FunctionEnd

  Function un.PanitaCancelErasure
    Pop $0 ; nsDialogs click-control handle.
    StrCpy $PanitaEraseStage 0
    StrCpy $PanitaRemoveDataWanted 0
    ${NSD_Uncheck} $PanitaRemoveDataCheckbox
    ${NSD_Uncheck} $PanitaEraseAcknowledgement
    ${NSD_SetText} $PanitaHeading "¿Desinstalar Panita Client?"
    ${NSD_SetText} $PanitaDescription "Se eliminará el launcher de este equipo."
    SetCtlColors $PanitaDescription BABABA 141414
    ${NSD_SetText} $PanitaCardText "Tus cuentas, instancias y mundos se conservarán."
    !insertmacro PANITA_RECT $PanitaCardBackground 0 270 402 82
    !insertmacro PANITA_RECT $PanitaCardText 16 284 370 54
    ShowWindow $PanitaRemoveDataCheckbox ${SW_SHOW}
    ShowWindow $PanitaDataSummary ${SW_SHOW}
    ShowWindow $PanitaEraseAcknowledgement ${SW_HIDE}
    ShowWindow $PanitaEraseBack ${SW_HIDE}
    EnableWindow $mui.Button.Next 1
    SendMessage $mui.Button.Next ${WM_SETTEXT} 0 "STR:Desinstalar"
    PanitaUi::Apply /NOUNLOAD
    ${NSD_SetFocus} $PanitaRemoveDataCheckbox
  FunctionEnd

  Function PanitaProgress
    !insertmacro PANITA_PROGRESS_LAYOUT "Instalando Panita Client"
  FunctionEnd

  Function un.PanitaProgress
    !insertmacro PANITA_PROGRESS_LAYOUT "Desinstalando Panita Client"
  FunctionEnd

  !insertmacro PANITA_PROGRESS_DONE ""
  !insertmacro PANITA_PROGRESS_DONE "un."
!macroend

!macro PANITA_PROGRESS_DONE PREFIX
  Function ${PREFIX}PanitaProgressDone
    IfAbort failed completed
    failed:
      System::Call 'user32::GetPropW(p $HWNDPARENT, w "PanitaPhase") p .r0'
      ${If} $0 != 2
        !insertmacro PANITA_REPORT "No se pudo completar la operación. Revisa los detalles y cierra esta ventana para volver a intentarlo."
      ${EndIf}
      System::Call 'user32::SetPropW(p $HWNDPARENT, w "PanitaPhase", p 2)'
      Goto done
    completed:
      System::Call 'user32::SetPropW(p $HWNDPARENT, w "PanitaPhase", p 3)'
    done:
      PanitaUi::Apply /NOUNLOAD
  FunctionEnd
!macroend

!macro PANITA_GUI_INIT PREFIX
  Function ${PREFIX}PanitaGuiInit
    System::Call 'user32::GetDpiForWindow(p $HWNDPARENT) i .r0'
    StrCpy $PanitaDpi $0
    ${If} $PanitaDpi < 96
      StrCpy $PanitaDpi 96
    ${EndIf}
    ; Keep the system menu/keyboard controls but replace the native caption.
    System::Call 'user32::GetWindowLongW(p $HWNDPARENT, i -16) i .r0'
    IntOp $0 $0 & 0xFF3BFFFF
    System::Call 'user32::SetWindowLongW(p $HWNDPARENT, i -16, i r0)'
    IntOp $0 560 * $PanitaDpi
    IntOp $0 $0 / 96
    IntOp $1 504 * $PanitaDpi
    IntOp $1 $1 / 96
    System::Call 'user32::SetWindowPos(p $HWNDPARENT, p 0, i 0, i 0, i r0, i r1, i 38)'
    SetCtlColors $HWNDPARENT F9FFF8 141414
    CreateFont $PanitaTitleFont "Segoe UI" 20 700
    CreateFont $PanitaBrandFont "Segoe UI" 11 700
    CreateFont $PanitaBodyFont "Segoe UI" 11 400
    InitPluginsDir
    File /oname=$PLUGINSDIR\panita-brand.ico "${PANITA_BRAND_ICON}"
    System::Call 'user32::LoadImageW(p 0, w "$PLUGINSDIR\panita-brand.ico", i 1, i 48, i 48, i 0x10) p .r0'
    StrCpy $PanitaIcon $0
    System::Call 'user32::SetPropW(p $HWNDPARENT, w "PanitaIcon", p $PanitaIcon)'
    System::Call 'user32::CreateWindowExW(i 0, w "STATIC", w "v${VERSION}", i 0x40000000, i 0, i 0, i 0, i 0, p $HWNDPARENT, p 0, p 0, p 0) p .r0'
    System::Call 'user32::SetPropW(p $HWNDPARENT, w "PanitaVersionLabel", p r0)'
    !insertmacro PANITA_ROLE $0 5
    !if "${PREFIX}" == "un."
      System::Call 'user32::SetPropW(p $HWNDPARENT, w "PanitaUninstall", p 1)'
    !endif
    ; Replace the wizard header and separators with the page's own branding.
    ShowWindow $mui.Header.Text ${SW_HIDE}
    ShowWindow $mui.Header.SubText ${SW_HIDE}
    ShowWindow $mui.Header.Background ${SW_HIDE}
    ShowWindow $mui.Branding.Text ${SW_HIDE}
    ShowWindow $mui.Branding.Background ${SW_HIDE}
    ShowWindow $mui.Line.Standard ${SW_HIDE}
    ShowWindow $mui.Line.FullWindow ${SW_HIDE}
  FunctionEnd

  Function ${PREFIX}PanitaUserAbort
    System::Call 'user32::GetPropW(p $HWNDPARENT, w "PanitaPhase") p .r0'
    ${If} $0 == 1
      Abort ; Never interrupt file mutation through the custom close control.
    ${ElseIf} $0 == 3
    ${OrIf} $0 == 4
      SetErrorLevel 0
      Quit ; Completed setup may close without starting the launcher.
    ${EndIf}
  FunctionEnd

!macroend

!macro PANITA_PROGRESS_LAYOUT TITLE
  FindWindow $PanitaDialog "#32770" "" $HWNDPARENT
  !insertmacro PANITA_PAGE_LAYOUT $PanitaDialog
  StrCpy $PanitaProgressActive 1
  System::Call 'user32::SetPropW(p $HWNDPARENT, w "PanitaPhase", p 1)'
  System::Call 'user32::CreateWindowExW(i 0, w "STATIC", w " ", i 0x40000000, i 0, i 0, i 0, i 0, p $PanitaDialog, p 0, p 0, p 0) p .r0'
  StrCpy $PanitaProgressNotice $0
  !insertmacro PANITA_ROLE $PanitaProgressNotice 17
  SendMessage $PanitaProgressNotice ${WM_SETFONT} $PanitaBodyFont 1
  SetCtlColors $PanitaProgressNotice FCA5A5 141414
  !insertmacro PANITA_RECT $PanitaProgressNotice 0 70 402 128
  ; Brand the native progress page while retaining its actual status/log controls.
  System::Call 'user32::CreateWindowExW(i 0, w "STATIC", w "${TITLE}", i 0x50000000, i 0, i 0, i 0, i 0, p $PanitaDialog, p 0, p 0, p 0) p .r0'
  System::Call 'user32::SetPropW(p r0, w "PanitaStageTitle", p 1)'
  SendMessage $0 ${WM_SETFONT} $PanitaBrandFont 1
  SetCtlColors $0 F9FFF8 141414
  !insertmacro PANITA_RECT $0 0 4 402 32
  GetDlgItem $0 $PanitaDialog 1006
  SetCtlColors $0 BABABA 141414
  !insertmacro PANITA_RECT $0 0 48 402 36
  GetDlgItem $0 $PanitaDialog 1004
  System::Call 'uxtheme::SetWindowTheme(p r0, w "", w "")'
  SendMessage $0 0x409 0 0x36A654 ; PBM_SETBARCOLOR, green COLORREF.
  SendMessage $0 0x2001 0 0x1B1B1B ; PBM_SETBKCOLOR.
  !insertmacro PANITA_RECT $0 0 96 402 10
  GetDlgItem $0 $PanitaDialog 1027
  ShowWindow $0 ${SW_HIDE}
  GetDlgItem $0 $PanitaDialog 1016
  Push $1
  System::Call 'user32::GetWindowLongW(p r0, i -16) i .r1'
  IntOp $1 $1 & 0xFF7FFFFF
  System::Call 'user32::SetWindowLongW(p r0, i -16, i r1)'
  System::Call 'user32::GetWindowLongW(p r0, i -20) i .r1'
  IntOp $1 $1 & 0xFFFFFDFF
  System::Call 'user32::SetWindowLongW(p r0, i -20, i r1)'
  System::Call 'user32::SetWindowPos(p r0, p 0, i 0, i 0, i 0, i 0, i 39)'
  Pop $1
  SendMessage $0 0x1001 0 0x1B1B1B ; LVM_SETBKCOLOR.
  SendMessage $0 0x1026 0 0x1B1B1B ; LVM_SETTEXTBKCOLOR.
  SendMessage $0 0x1024 0 0xF8FFF9 ; LVM_SETTEXTCOLOR.
  !insertmacro PANITA_RECT $0 0 122 402 240
  PanitaUi::Apply /NOUNLOAD
!macroend
