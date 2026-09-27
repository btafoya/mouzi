!macro NSIS_HOOK_PREINSTALL
  ; Attempt to close a running Mouzi instance gracefully before installing.
  ; First try a soft terminate, wait a moment, then force-kill if still running.
  ExecWait '"$SYSDIR\taskkill.exe" /IM mouzi.exe /T' $0
  Sleep 500
  ExecWait '"$SYSDIR\taskkill.exe" /F /IM mouzi.exe /T' $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DeleteRegKey HKCU "Software\Classes\Directory\shell\Mouzi"
!macroend
