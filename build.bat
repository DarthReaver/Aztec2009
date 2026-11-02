@echo off
rem Build Aztec 2009 into aztec2009.exe (windowed workspace plus terminal modes).
setlocal
cd /d "%~dp0"
cargo build --release
if errorlevel 1 (
  echo Build failed. Make sure Rust and the MSVC build tools are installed: https://rustup.rs
  pause
  exit /b 1
)
copy /y "target\release\aztec2009.exe" "aztec2009.exe" >nul
echo Built aztec2009.exe
echo Open the app:    "Aztec 2009.bat"
echo Terminal UI:     aztec2009.exe --tui
echo Run a script:    aztec2009.exe -c script.mel
endlocal
