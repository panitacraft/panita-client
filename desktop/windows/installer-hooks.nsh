; Refuse unrelated protocol handlers before Tauri registers this installation.
; Tauri's generated uninstaller checks the exact executable command before removal.
!ifdef PANITA_CUSTOM_UI
  !include "${__FILEDIR__}\installer-ui.nsh"
!endif
!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $R7 HKCU "Software\Classes\panitacraft\shell\open\command" ""
  ReadRegStr $R8 HKCU "Software\Classes\panitacraft" "PanitaClientOwner"
  ${If} $R7 != ""
  ${AndIf} $R7 != "$\"$INSTDIR\${MAINBINARYNAME}.exe$\" $\"%1$\""
  ${AndIf} $R8 != "com.panitacraft.client"
    !ifdef PANITA_CUSTOM_UI
      !insertmacro PANITA_REPORT "Otra aplicación utiliza el enlace panitacraft. Libera ese enlace antes de instalar Panita Client."
    !else
      MessageBox MB_OK|MB_ICONSTOP "Otra aplicación utiliza el enlace panitacraft. Libera ese enlace antes de instalar Panita Client." /SD IDOK
    !endif
    Abort
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTINSTALL
  WriteRegStr HKCU "Software\Classes\panitacraft" "PanitaClientOwner" "com.panitacraft.client"
!macroend
