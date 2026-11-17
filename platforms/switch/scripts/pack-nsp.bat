@echo off
rem Double-click wrapper for pack-nsp.sh (Git Bash).
rem Drag a game folder onto this .bat, or run it from a terminal with options:
rem   pack-nsp.bat "F:\games\MyGame" --mode b --title "My Game"
rem See platforms\switch\NSP-PACKING.md (Chinese) for the full guide.
setlocal
set "BASH=C:\Program Files\Git\bin\bash.exe"
if not exist "%BASH%" set "BASH=C:\Program Files (x86)\Git\bin\bash.exe"
if not exist "%BASH%" (
    echo Git Bash not found. Install Git for Windows, or run pack-nsp.sh manually.
    pause
    exit /b 1
)
"%BASH%" "%~dp0pack-nsp.sh" %*
echo.
pause