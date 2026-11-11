@echo off
title Uninstall litewave
set "APPDIR=%~dp0"
cd /d "%TEMP%"

echo ============================================
echo   This will COMPLETELY remove litewave:
echo    - the program and its shortcuts
echo    - your settings, cache and saved Spotify sign-in
echo ============================================
set /p ANSWER=Type Y and press Enter to continue: 
if /i not "%ANSWER%"=="Y" exit /b

taskkill /f /im litewave.exe >nul 2>&1

rmdir /s /q "%APPDATA%\litewave" >nul 2>&1
rmdir /s /q "%LOCALAPPDATA%\litewave" >nul 2>&1
cmdkey /delete:spotify_refresh.litewave >nul 2>&1
cmdkey /delete:spotify_browse_refresh.litewave >nul 2>&1
echo Settings and cache removed.

if not exist "%APPDIR%unins000.exe" goto missing
echo Removing the program...
start "" "%APPDIR%unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES
echo.
echo Done. Last step: go to spotify.com/account/apps and remove access for litewave.
timeout /t 5 >nul
exit

:missing
echo Could not find unins000.exe next to this file. Remove litewave from Settings - Apps instead.
pause
exit /b
