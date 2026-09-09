@echo off
REM Скрипт для обновления версии во всех файлах проекта (Windows)

if "%1"=="" (
    echo Использование: scripts\bump-version.bat 1.2.1
    exit /b 1
)

set NEW_VERSION=%1

echo Обновление версии до %NEW_VERSION%...

REM Обновляем package.json
call npm version %NEW_VERSION% --no-git-tag-version

REM Обновляем Cargo.toml
powershell -Command "(Get-Content src-tauri\Cargo.toml) -replace 'version = \".*\"', 'version = \"%NEW_VERSION%\"' | Set-Content src-tauri\Cargo.toml"

REM Обновляем tauri.conf.json
powershell -Command "(Get-Content src-tauri\tauri.conf.json) -replace '\"version\": \".*\"', '\"version\": \"%NEW_VERSION%\"' | Set-Content src-tauri\tauri.conf.json"

echo.
echo ✓ package.json
echo ✓ src-tauri\Cargo.toml
echo ✓ src-tauri\tauri.conf.json
echo.
echo Версия обновлена до %NEW_VERSION%
echo.
echo Следующие шаги:
echo 1. Проверьте изменения: git diff
echo 2. Закоммитьте: git commit -am "Bump version to %NEW_VERSION%"
echo 3. Создайте тег: git tag v%NEW_VERSION%
echo 4. Запушьте: git push ^&^& git push origin v%NEW_VERSION%
