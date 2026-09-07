@echo off
setlocal enabledelayedexpansion

echo ============================================
echo   Repomix Clean Pipeline
echo ============================================
echo.

:: ============ Parse arguments ============
::  compresser.bat [dirs...] [-dp] [-rn ^<name^>] [-inc ^<patterns^>] [-np]
::
::  dirs...       folders to pack, relative to project root (default: . = whole repo)
::  -dp           pass to clean.py: strip "presets" section from wizard_tree.json
::  -rn ^<name^>   rename final dump to ^<name^>.xml
::  -inc ^<patterns^>  repomix --include glob patterns (comma-separated),
::             handy for dumping scattered contract files from the root
::  -np           no pause (auto mode, for chaining dumps)
:: ============================================

set "REMOVE_PRESETS=0"
set "RENAME_NAME="
set "INCLUDE_ARG="
set "NO_PAUSE=0"
set "TARGETS="

:parse
if "%~1"=="" goto parse_end
set "TK=%~1"
shift
if /i "!TK!"=="-dp"           goto set_dp
if /i "!TK!"=="--dp"          goto set_dp
if /i "!TK!"=="-np"           goto set_np
if /i "!TK!"=="-rn"           goto value_rename
if /i "!TK!"=="-inc"          goto value_include
if /i "!TK!"=="--include"     goto value_include
if /i "!TK!"=="/?"            goto usage
if /i "!TK!"=="/h"            goto usage

:: not a flag - treat as pack target
set "TARGETS=!TARGETS! !TK!"
goto parse

:set_dp
set "REMOVE_PRESETS=1"
goto parse

:set_np
set "NO_PAUSE=1"
goto parse

:: %~1 here is the VALUE (already shifted)
:value_rename
if "%~1"=="" goto usage
set "RENAME_NAME=%~1"
shift
goto parse

:value_include
if "%~1"=="" goto usage
set "INCLUDE_ARG=%~1"
shift
goto parse

:parse_end

if not defined TARGETS set "TARGETS= ."

echo [CONFIG]
echo    Targets:!TARGETS!
if defined INCLUDE_ARG echo    Include: !INCLUDE_ARG!
if "%REMOVE_PRESETS%"=="1" (
    echo    Mode   : removing presets from wizard_tree.json
) else (
    echo    Mode   : keeping presets section
)
if defined RENAME_NAME echo    Output : !RENAME_NAME!.xml
if "%NO_PAUSE%"=="1" echo    Auto   : no pause mode
echo.

:: ============ Step 1: clean old dumps + temp ignores ============
echo [1/6] Preparing workspace...
if exist "repomix-output.xml" del /q "repomix-output.xml"
if exist "repomix-output.clean.xml" del /q "repomix-output.clean.xml"
if defined RENAME_NAME if exist "!RENAME_NAME!.xml" del /q "!RENAME_NAME!.xml"
echo        [OK] Old dumps removed

set "IGNORED_DIRS="
for %%p in (%TARGETS%) do (
    if /i not "%%p"=="." (
        if exist "%%p\" (
            if not exist "%%p\.repomixignore" (
                if exist ".repomixignore" (
                    copy /y ".repomixignore" "%%p\.repomixignore" >nul
                    if exist "%%p\.repomixignore" set "IGNORED_DIRS=!IGNORED_DIRS! %%p"
                )
            )
        )
    )
)
if defined IGNORED_DIRS (
    echo        [OK] Temp .repomixignore copied to:!IGNORED_DIRS!
) else (
    echo        [OK] No temp .repomixignore copies needed
)
echo.

:: ============ Step 2: run repomix ============
echo [2/6] Running Repomix...
set "EXTRA_FLAGS="
if defined INCLUDE_ARG set "EXTRA_FLAGS= --include "!INCLUDE_ARG!""

echo        npx repomix:!TARGETS!!EXTRA_FLAGS! -o repomix-output.xml --output-file-path-style cwd-relative
echo.
call npx repomix%TARGETS%%EXTRA_FLAGS% -o repomix-output.xml --output-file-path-style cwd-relative
set "REPOMIX_EXIT=!errorlevel!"

:: temp ignore copies must be removed no matter what repomix did
for %%d in (%IGNORED_DIRS%) do (
    if exist "%%d\.repomixignore" del /q "%%d\.repomixignore"
)
echo.
echo        [OK] Temp .repomixignore copies removed

if not "!REPOMIX_EXIT!"=="0" (
    set "ERR_MSG=Repomix failed with code: !REPOMIX_EXIT!"
    goto fatal
)
if not exist "repomix-output.xml" (
    set "ERR_MSG=repomix-output.xml not created!"
    goto fatal
)
echo        [OK] Repomix completed successfully
echo.

:: ============ Step 3: check clean.py ============
echo [3/6] Checking clean.py...
if not exist "clean.py" (
    set "ERR_MSG=clean.py not found!"
    goto fatal
)
echo        [OK] clean.py found
echo.

:: ============ Step 4: run clean.py ============
echo [4/6] Cleaning tests from XML...
set "RN_ARGS="
if defined RENAME_NAME set "RN_ARGS=-rn !RENAME_NAME!"
if "%REMOVE_PRESETS%"=="1" (
    echo        python clean.py repomix-output.xml -dp !RN_ARGS!
    python clean.py repomix-output.xml -dp !RN_ARGS!
) else (
    echo        python clean.py repomix-output.xml !RN_ARGS!
    python clean.py repomix-output.xml !RN_ARGS!
)
if not "!errorlevel!"=="0" (
    set "ERR_MSG=clean.py failed with code: !errorlevel!"
    goto fatal
)
echo.
echo        [OK] Cleaning completed successfully
echo.

:: ============ Step 5: finalize ============
echo [5/6] Finalizing...
set "FINAL_FILE=repomix-output.xml"
if defined RENAME_NAME (
    if exist "!RENAME_NAME!.xml" (
        if exist "repomix-output.xml" del /q "repomix-output.xml"
        if exist "repomix-output.clean.xml" del /q "repomix-output.clean.xml"
        set "FINAL_FILE=!RENAME_NAME!.xml"
        echo        [OK] Saved as !FINAL_FILE!
    ) else (
        echo        [WARN] Cleaned output "!RENAME_NAME!.xml" not found, keeping original
    )
) else (
    if exist "repomix-output.clean.xml" (
        if exist "repomix-output.xml" del /q "repomix-output.xml"
        ren "repomix-output.clean.xml" "repomix-output.xml"
        echo        [OK] Cleaned file saved as repomix-output.xml
    ) else (
        echo        [WARN] Cleaned file not found!
    )
)
echo.

:: ============ Step 6: summary ============
echo [6/6] Summary
echo.
echo ============================================
echo   COMPLETED!
echo ============================================
echo.
echo Final file: !FINAL_FILE!
if exist "!FINAL_FILE!" (
    for %%i in ("!FINAL_FILE!") do (
        set "fsize=%%~zi"
        set /a fsize_kb=!fsize!/1024
        echo    Size: !fsize_kb! KB
    )
)
echo.
echo Ready for AI agent!
echo ============================================
echo.
if "%NO_PAUSE%"=="0" pause
exit /b 0

:: ============ usage ============
:usage
echo.
echo Usage:
echo   compresser.bat [dirs...] [-dp] [-rn ^<name^>] [-inc ^<patterns^>] [-np]
echo.
echo   dirs...         folders to pack, relative to project root (default: .)
echo   -dp             remove "presets" section from wizard_tree.json in dump
echo   -rn ^<name^>     rename final dump to ^<name^>.xml
echo   -inc ^<patterns^>  repomix --include globs (comma-separated) for scattered files
echo   -np             no pause - auto mode, safe to chain with ^&^& or in a loop
echo.
echo Examples:
echo   compresser.bat src-tauri\src\modules\project_creator -dp -rn project_creator_be
echo   compresser.bat src\lib\modules\project_creator src\routes\create -rn project_creator_fe
if "%NO_PAUSE%"=="0" pause
exit /b 0

:: ============ fatal ============
:fatal
echo.
echo [ERROR] !ERR_MSG!
echo.
if "%NO_PAUSE%"=="0" pause
exit /b 1