; Preserve the supported data-retention/uninstall behavior verbatim.
!include "${__FILEDIR__}\installer-hooks.nsh"

; NSIS copies bytes rather than source ACLs. Chromium's sandboxed helpers need
; read/execute access to this public engine subtree only. No profiles, keys or
; installation ancestors receive package grants, and no elevation is requested.
!macro NSIS_HOOK_POSTINSTALL
  IfFileExists "$INSTDIR\browser\component\libcef.dll" +3
    MessageBox MB_OK|MB_ICONSTOP "The browser preview component is missing. Installation cannot finish." /SD IDOK
    Abort
  nsExec::ExecToStack '"$SYSDIR\icacls.exe" "$INSTDIR\browser\component" /grant "*S-1-15-2-1:(OI)(CI)(RX)" "*S-1-15-2-2:(OI)(CI)(RX)"'
  Pop $0
  Pop $1
  ${If} $0 != 0
    MessageBox MB_OK|MB_ICONSTOP "Windows could not prepare the browser preview sandbox resources. Installation cannot finish." /SD IDOK
    Abort
  ${EndIf}
!macroend
