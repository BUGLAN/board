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
"%BIN%" %*
exit /b %ERRORLEVEL%
