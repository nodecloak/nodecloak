; Tauri preserves shortcuts during /UPDATE but does not create missing ones.
; Repair only the Start menu entry, keeping desktop shortcuts and /NS choices.
!macro NSIS_HOOK_POSTINSTALL
  ${If} $UpdateMode = 1
  ${AndIf} $NoShortcutMode != 1
    !if "${STARTMENUFOLDER}" != ""
      ${IfNot} ${FileExists} "$SMPROGRAMS\$AppStartMenuFolder\${PRODUCTNAME}.lnk"
    !else
      ${IfNot} ${FileExists} "$SMPROGRAMS\${PRODUCTNAME}.lnk"
    !endif
        Push $UpdateMode
        StrCpy $UpdateMode 0
        Call CreateOrUpdateStartMenuShortcut
        Pop $UpdateMode
      ${EndIf}
  ${EndIf}
!macroend
