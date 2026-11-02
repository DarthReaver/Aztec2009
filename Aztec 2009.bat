@echo off
rem Aztec 2009 launcher for Windows. Double click to start.
rem Order: aztec2009.exe next to this file, then target\release, then build from source.
setlocal
cd /d "%~dp0"
set "EXE=%~dp0aztec2009.exe"
if not exist "%EXE%" set "EXE=%~dp0target\release\aztec2009.exe"
if not exist "%EXE%" (
  where cargo >nul 2>nul
  if errorlevel 1 (
    echo Aztec 2009 is not built yet and Rust was not found.
    echo Install Rust from https://rustup.rs, or download the prebuilt zip, then run this again.
    pause
    exit /b 1
  )
  echo First run: building Aztec 2009, this takes a few minutes...
  call "%~dp0build.bat"
  if errorlevel 1 exit /b 1
  set "EXE=%~dp0aztec2009.exe"
)
if "%~1"=="" (
  start "" "%EXE%"
) else (
  "%EXE%" %*
)
endlocal
