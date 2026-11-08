@echo off
rem board bundled with the skill (Windows): runs bin\windows-x64\board.exe from the skill directory,
rem falling back to board.exe on PATH.  Usage: <skill dir>\scripts\board.cmd <command> ...
rem bin\ is produced by node scripts/build-skill-bin.mjs; the board-skills.zip release asset includes it.
rem Keep this file ASCII-only: cmd.exe reads batch files in the OEM code page.
setlocal
set "BIN=%~dp0..\bin\windows-x64\board.exe"
if exist "%BIN%" goto run
set "BIN="
for /f "delims=" %%i in ('where board.exe 2^>nul') do if not defined BIN set "BIN=%%i"
if defined BIN goto run
echo board: no bin\windows-x64\board.exe in the skill directory and no board.exe on PATH. Reinstall the skill from the board-skills.zip release asset. 1>&2
exit /b 127
:run
"%BIN%" %*
exit /b %ERRORLEVEL%
