@echo off
rem board bundled with the skill (Windows): runs bin\windows-x64\board.exe from the skill directory,
rem falling back to board.exe on PATH.  Usage: <skill dir>\scripts\board.cmd <command> ...
rem bin\ is produced by node scripts/build-skill-bin.mjs; the board-skills.zip release asset includes it.
rem Keep this file ASCII-only: cmd.exe reads batch files in the OEM code page.
rem Multi-line text cannot pass through this file: cmd.exe ends the whole command at the first line break
rem in an argument (before this script even runs), so --body "line1<LF>line2" arrives as "line1".
rem Write multi-line text to a UTF-8 file and pass --body-file <file> (new, edit) or --comment-file <file> (comment, attach).
setlocal
rem Sessions started by board host use the host's own board (BOARD_EXE), same version as the host.
if defined BOARD_EXE if exist "%BOARD_EXE%" (
  set "BIN=%BOARD_EXE%"
  goto run
)
set "BIN=%~dp0..\bin\windows-x64\board.exe"
if exist "%BIN%" goto run
set "BIN="
for /f "delims=" %%i in ('where board.exe 2^>nul') do if not defined BIN set "BIN=%%i"
if defined BIN (
  rem Not silent: an older board on PATH may not know newer options; say which one is used
  echo board: no bin\windows-x64\board.exe in the skill directory, using "%BIN%" from PATH. If a command or option is rejected, reinstall the skill from the board-skills.zip release asset. 1>&2
  goto run
)
echo board: no bin\windows-x64\board.exe in the skill directory and no board.exe on PATH. Reinstall the skill from the board-skills.zip release asset. 1>&2
exit /b 127
:run
rem Sessions started by board host write the BOARD_* variables they need into %BOARD_CONFIG%'s
rem [agent_env]: agents that filter the environment they give to commands (Codex, dsh) drop them,
rem so board.exe reads them from there itself (see crates/board-cli/src/config.rs from_agent_env).
rem Only when BOARD_CONFIG is set, same as scripts/board (bash): a plain ~/.board/config.toml is not one.
if defined BOARD_CONFIG set "BOARD_AGENT_ENV=1"
rem start / submit / giveup were added later: an older board does not know them. Say so instead of
rem letting the agent rebuild the same steps from older commands.
if /i "%~1"=="start" goto newcmd
if /i "%~1"=="submit" goto newcmd
if /i "%~1"=="giveup" goto newcmd
goto exec
:newcmd
"%BIN%" help 2>nul | findstr /c:"board %~1 " >nul
if not errorlevel 1 goto exec
echo board: "%BIN%" is an old version without the start / submit / giveup commands. Upgrade it and try again: run board update, or reinstall the skill from the board-skills.zip release asset; on a machine running board host, upgrade it from the Machines page. 1>&2
exit /b 1
:exec
"%BIN%" %*
exit /b %ERRORLEVEL%
