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
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall-delete-desktop-data' $0
    IfErrors colossus_cleanup_failed
    ${If} $0 != 0
      colossus_cleanup_failed:
      MessageBox MB_RETRYCANCEL|MB_ICONSTOP "Colossus could not finish deleting its Desktop data. Close Colossus and its running tasks, then retry. Some data may already have been deleted. Uninstall has been stopped so you can retry cleanup." /SD IDCANCEL IDRETRY colossus_cleanup_retry
      Abort
    ${EndIf}
  ${EndIf}
!macroend
