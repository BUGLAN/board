; ==========================================================================
; 桌面端安装包自带 board 命令行（bundle.externalBin），安装时把安装目录写进
; 当前用户的 PATH，新开的终端就能直接运行 board；卸载时把这一条移除。
;
; NSIS 默认按当前用户安装（currentUser），所以写 HKCU\\Environment\\Path。
; PATH 的字符串处理交给 PowerShell：读出来、去重、写回去，比在 NSIS 里做安全。
;
; 升级前还要处理旧版遗留的 board host：桌面端调用 Windows 更新器后直接退出
; （`download_and_install` 启动安装器后 `std::process::exit`），来不及走退出流程
; 停掉它；它锁着安装目录的 board.exe，安装器复制文件时就报
; 「Error opening file for writing: …\\board.exe」（#389）。新版桌面端在安装前
; 会先停掉 board host（src-tauri/src/host.rs 的 pause_for_update）；这里按完整路径
; 再兜底处理一次，旧版桌面端发起的升级也不会卡住。
; ==========================================================================

!include WinMessages.nsh

; 升级安装前：结束路径正好是 $INSTDIR\\board.exe 的进程，等最多 5 秒；
; 还退不掉（或结束不了）就把它改名成 board.old.exe，安装器能写新的 board.exe。
; 只认这个完整路径，不碰别的目录里同名的 board.exe（参考 scripts/build-sidecar.mjs 的匹配）。
Function BoardCliStopHost
  StrCmp $INSTDIR "" done
  InitPluginsDir
  FileOpen $9 "$PLUGINSDIR\\board-cli-stop-host.ps1" w
  FileWrite $9 "if (-not $$args -or -not $$args[0]) { exit 0 }$\r$\n"
  FileWrite $9 "$$exe = [IO.Path]::GetFullPath($$args[0])$\r$\n"
  FileWrite $9 "$$pname = [IO.Path]::GetFileNameWithoutExtension($$exe)$\r$\n"
  FileWrite $9 "function Get-BoardProcs {$\r$\n"
  FileWrite $9 "  @(Get-Process -Name $$pname -ErrorAction SilentlyContinue | Where-Object {$\r$\n"
  FileWrite $9 "    try { $$p = $$_.Path } catch { $$p = $$null }$\r$\n"
  FileWrite $9 "    $$p -and ([IO.Path]::GetFullPath($$p) -ieq $$exe)$\r$\n"
  FileWrite $9 "  })$\r$\n"
  FileWrite $9 "}$\r$\n"
  FileWrite $9 "$$procs = @(Get-BoardProcs)$\r$\n"
  FileWrite $9 "if ($$procs.Count -gt 0) {$\r$\n"
  FileWrite $9 "  $$procs | Stop-Process -Force -ErrorAction SilentlyContinue$\r$\n"
  FileWrite $9 "  $$deadline = (Get-Date).AddSeconds(5)$\r$\n"
  FileWrite $9 "  while ((Get-Date) -lt $$deadline -and @(Get-BoardProcs).Count -gt 0) {$\r$\n"
  FileWrite $9 "    Start-Sleep -Milliseconds 100$\r$\n"
  FileWrite $9 "  }$\r$\n"
  FileWrite $9 "  if (@(Get-BoardProcs).Count -gt 0 -and (Test-Path -LiteralPath $$exe)) {$\r$\n"
  FileWrite $9 "    $$dir = [IO.Path]::GetDirectoryName($$exe)$\r$\n"
  FileWrite $9 "    $$old = [IO.Path]::Combine($$dir, 'board.old.exe')$\r$\n"
  FileWrite $9 "    Remove-Item -LiteralPath $$old -Force -ErrorAction SilentlyContinue$\r$\n"
  FileWrite $9 "    if (Test-Path -LiteralPath $$old) {$\r$\n"
  FileWrite $9 "      $$old = [IO.Path]::Combine($$dir, ('board.old.' + (Get-Date -Format 'yyyyMMddHHmmss') + '.exe'))$\r$\n"
  FileWrite $9 "    }$\r$\n"
  FileWrite $9 "    try { Rename-Item -LiteralPath $$exe -NewName ([IO.Path]::GetFileName($$old)) -ErrorAction Stop } catch {}$\r$\n"
  FileWrite $9 "  }$\r$\n"
  FileWrite $9 "}$\r$\n"
  FileClose $9
  nsExec::ExecToStack 'powershell.exe -NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "$PLUGINSDIR\\board-cli-stop-host.ps1" "$INSTDIR\\board.exe"'
  Pop $1
  Pop $2
  done:
FunctionEnd

; 安装：把 $INSTDIR 追加到用户 PATH（已经有了就不重复加）
Function BoardCliAddToPath
  StrCmp $INSTDIR "" done
  InitPluginsDir
  FileOpen $9 "$PLUGINSDIR\\board-cli-path.ps1" w
  FileWrite $9 "if (-not $$args -or -not $$args[0]) { exit 1 }$\r$\n"
  FileWrite $9 "$$dir = $$args[0]$\r$\n"
  FileWrite $9 "$$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $$true)$\r$\n"
  FileWrite $9 "$$p = $$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)$\r$\n"
  FileWrite $9 "if (-not $$p) { $$p = '' }$\r$\n"
  FileWrite $9 "$$parts = @($$p -split ';' | Where-Object { $$_ -ne '' })$\r$\n"
  FileWrite $9 "if ($$parts -notcontains $$dir) {$\r$\n"
  FileWrite $9 "  $$key.SetValue('Path', (($$parts + $$dir) -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)$\r$\n"
  FileWrite $9 "}$\r$\n"
  FileClose $9
  nsExec::ExecToStack 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\\board-cli-path.ps1" "$INSTDIR"'
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
  FileWrite $9 "if (-not $$args -or -not $$args[0]) { exit 1 }$\r$\n"
  FileWrite $9 "$$dir = $$args[0]$\r$\n"
  FileWrite $9 "$$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $$true)$\r$\n"
  FileWrite $9 "$$p = $$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)$\r$\n"
  FileWrite $9 "if ($$p) {$\r$\n"
  FileWrite $9 "  $$parts = @($$p -split ';' | Where-Object { $$_ -ne '' -and $$_ -ne $$dir })$\r$\n"
  FileWrite $9 "  $$key.SetValue('Path', ($$parts -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)$\r$\n"
  FileWrite $9 "}$\r$\n"
  FileClose $9
  nsExec::ExecToStack 'powershell.exe -NoProfile -ExecutionPolicy Bypass -File "$PLUGINSDIR\\board-cli-path.ps1" "$INSTDIR"'
  Pop $1
  Pop $2
  SendMessage ${HWND_BROADCAST} ${WM_SETTINGCHANGE} 0 "STR:Environment" /TIMEOUT=5000
  done:
FunctionEnd

!macro NSIS_HOOK_PREINSTALL
  Call BoardCliStopHost
!macroend

!macro NSIS_HOOK_POSTINSTALL
  Call BoardCliAddToPath
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  Call un.BoardCliRemoveFromPath
!macroend
