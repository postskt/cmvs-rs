@echo off
rem Windows wrapper: rustc cannot execute linker.py directly (CreateProcess).
rem Forwards every argument unchanged to the upstream Python linker driver.
python "%~dp0linker.py" %*