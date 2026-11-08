# Backward-compatible wrapper: the heartbeat now lives in the board CLI.
# This script only forwards to the board bundled with the skill (scripts\board.cmd):
#   $env:BOARD_AS='<name>'; powershell -NoProfile -File heartbeat.ps1 start | stop | final <ID>
# New form (see SKILL.md): <skill dir>\scripts\board.cmd heartbeat start --as <name>
& (Join-Path $PSScriptRoot 'board.cmd') heartbeat @args
exit $LASTEXITCODE
