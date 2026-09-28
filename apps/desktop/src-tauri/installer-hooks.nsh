; Supported Tauri NSIS hook. Normal uninstall, updates, and silent uninstall
; preserve user data. Tauri owns the unchecked-by-default checkbox.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    MessageBox MB_YESNO|MB_ICONEXCLAMATION|MB_DEFBUTTON2 "Permanently delete local conversations, provider and model configurations, saved Desktop credentials, plugins, and Desktop settings? Your project files and custom data folders will remain." /SD IDNO IDYES colossus_cleanup_confirmed
    Abort
    colossus_cleanup_confirmed:
    !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
    colossus_cleanup_retry:
    ClearErrors
    StrCpy $0 1
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall-delete-desktop-data' $0
    IfErrors colossus_cleanup_failed
    ${If} $0 != 0
      colossus_cleanup_failed:
      StrCpy $1 "Colossus could not run its Desktop data cleanup."
      ${If} $0 = 2
        StrCpy $1 "Desktop data is still in use. Close Colossus and its running tasks, then retry."
      ${ElseIf} $0 = 3
        StrCpy $1 "Colossus could not verify that all Desktop data is safe to delete. The folder may contain shared files, an unrecognized folder, or unsupported settings."
      ${ElseIf} $0 = 4
        StrCpy $1 "Colossus could not remove its saved credentials from Windows."
      ${ElseIf} $0 = 5
        StrCpy $1 "Windows could not remove the Desktop data files. Check file permissions and whether another program is using the folder."
      ${EndIf}
      MessageBox MB_RETRYCANCEL|MB_ICONSTOP "$1$\r$\n$\r$\nUninstall has stopped. Some data may already have been deleted. Retry cleanup, or cancel and uninstall again without selecting Delete application data to keep the remaining data." /SD IDCANCEL IDRETRY colossus_cleanup_retry
      Abort
    ${EndIf}
  ${EndIf}
!macroend
