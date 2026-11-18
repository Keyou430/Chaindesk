@echo off
setlocal
start "" /wait "%~dp0ChainDesk.exe" --recover-proxy
set "chaindeskRecoveryStatus=%errorlevel%"
if "%chaindeskRecoveryStatus%"=="0" (
  echo ChainDesk proxy recovery finished. Current external proxy settings are preserved.
) else (
  echo Recovery needs attention. Close the running ChainDesk instance and try again.
  echo If another application changed the proxy, keep its current settings in ChainDesk recovery.
)
pause
exit /b %chaindeskRecoveryStatus%
