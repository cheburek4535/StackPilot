@echo off
setlocal enabledelayedexpansion

echo ============================================
echo   Repomix Clean Pipeline
echo ============================================
echo.

:: Step 1: Remove old files
echo [1/5] Removing old files...
if exist "repomix-output.xml" del /q "repomix-output.xml"
if exist "repomix-output.clean.xml" del /q "repomix-output.clean.xml"
echo        [OK] Old files removed
echo.

:: Step 2: Run Repomix
echo [2/5] Running Repomix...
echo        npx repomix --compress --remove-comments
echo.

call npx repomix --compress --remove-comments

if %errorlevel% neq 0 (
    echo.
    echo [ERROR] Repomix failed with code: %errorlevel%
    echo.
    pause
    exit /b 1
)

if not exist "repomix-output.xml" (
    echo.
    echo [ERROR] repomix-output.xml not created!
    echo.
    pause
    exit /b 1
)

echo.
echo        [OK] Repomix completed successfully
echo.

:: Step 3: Check clean.py
echo [3/5] Checking clean.py...
if not exist "clean.py" (
    echo [ERROR] clean.py not found!
    pause
    exit /b 1
)
echo        [OK] clean.py found
echo.

:: Step 4: Run Python script
echo [4/5] Cleaning tests from XML...
python clean.py repomix-output.xml

if %errorlevel% neq 0 (
    echo.
    echo [ERROR] clean.py failed with code: %errorlevel%
    pause
    exit /b 1
)

echo.
echo        [OK] Cleaning completed successfully
echo.

:: Step 5: Remove original, keep only cleaned
echo [5/5] Finalizing...
if exist "repomix-output.clean.xml" (
    if exist "repomix-output.xml" del "repomix-output.xml"
    ren "repomix-output.clean.xml" "repomix-output.xml"
    echo        [OK] Cleaned file saved as repomix-output.xml
    echo        [OK] Original file removed
) else (
    echo        [WARN] Cleaned file not found!
)

:: Final results
echo.
echo ============================================
echo   COMPLETED!
echo ============================================
echo.
echo [OK] All operations completed successfully!
echo.
echo Final file: repomix-output.xml
if exist "repomix-output.xml" (
    for %%i in ("repomix-output.xml") do (
        set size=%%~zi
        set /a size_kb=!size!/1024
        echo    Size: !size_kb! KB
    )
)
echo.
echo Ready for AI agent!
echo ============================================
echo.

pause