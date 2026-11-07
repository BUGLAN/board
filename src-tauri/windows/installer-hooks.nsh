; ==========================================================================
; 桌面端安装包自带 board 命令行（bundle.externalBin），安装时把安装目录写进
; 当前用户的 PATH，新开的终端就能直接运行 board；卸载时把这一条移除。
;
; NSIS 默认按当前用户安装（currentUser），所以写 HKCU\\Environment\\Path。
; PATH 的字符串处理交给 PowerShell：读出来、去重、写回去，比在 NSIS 里做安全。
; ==========================================================================

!include WinMessages.nsh

; 安装：把 $INSTDIR 追加到用户 PATH（已经有了就不重复加）
Function BoardCliAddToPath
  StrCmp $INSTDIR "" done
  InitPluginsDir
  FileOpen $9 "$PLUGINSDIR\\board-cli-path.ps1" w
  FileWrite $9 "$$dir = '$INSTDIR'$\r$\n"
  FileWrite $9 "$$p = [Environment]::GetEnvironmentVariable('Path','User')$\r$\n"
  FileWrite $9 "if (-not $$p) { $$p = '' }$\r$\n"
  FileWrite $9 "$$parts = @($$p -split ';' | Where-Object { $$_ -ne '' })$\r$\n"
  FileWrite $9 "if ($$parts -notcontains $$dir) {$\r$\n"
  FileWrite $9 "  [Environment]::SetEnvironmentVariable('Path', ($$parts + $$dir -join ';'), 'User')$\r$\n"
  FileWrite $9 "}$\r$\n"
  FileClose $9
  nsExec::ExecToStack 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\\board-cli-path.ps1"'
  Pop $1
  Pop $2
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
  done:
FunctionEnd

; 卸载：把 $INSTDIR 从用户 PATH 里移除
Function un.BoardCliRemoveFromPath
  StrCmp $INSTDIR "" done
  InitPluginsDir
  FileOpen $9 "$PLUGINSDIR\\board-cli-path.ps1" w
  FileWrite $9 "$$dir = '$INSTDIR'$\r$\n"
  FileWrite $9 "$$p = [Environment]::GetEnvironmentVariable('Path','User')$\r$\n"
  FileWrite $9 "if ($$p) {$\r$\n"
  FileWrite $9 "  $$parts = @($$p -split ';' | Where-Object { $$_ -ne '' -and $$_ -ne $$dir })$\r$\n"
  FileWrite $9 "  [Environment]::SetEnvironmentVariable('Path', ($$parts -join ';'), 'User')$\r$\n"
  FileWrite $9 "}$\r$\n"
  FileClose $9
  nsExec::ExecToStack 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\\board-cli-path.ps1"'
  Pop $1
  Pop $2
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
  done:
FunctionEnd

!macro NSIS_HOOK_POSTINSTALL
  Call BoardCliAddToPath
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Call un.BoardCliRemoveFromPath
!macroend
