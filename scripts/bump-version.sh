#!/bin/bash

# Скрипт для обновления версии во всех файлах проекта

if [ -z "$1" ]; then
    echo "Использование: ./scripts/bump-version.sh 1.2.1"
    exit 1
fi

NEW_VERSION=$1

echo "Обновление версии до $NEW_VERSION..."

# Обновляем package.json
npm version $NEW_VERSION --no-git-tag-version

# Обновляем Cargo.toml
sed -i.bak "s/^version = \".*\"/version = \"$NEW_VERSION\"/" src-tauri/Cargo.toml
rm -f src-tauri/Cargo.toml.bak

# Обновляем tauri.conf.json
if [[ "$OSTYPE" == "darwin"* ]]; then
    # macOS
    sed -i '' "s/\"version\": \".*\"/\"version\": \"$NEW_VERSION\"/" src-tauri/tauri.conf.json
else
    # Linux
    sed -i "s/\"version\": \".*\"/\"version\": \"$NEW_VERSION\"/" src-tauri/tauri.conf.json
fi

echo "✓ package.json"
echo "✓ src-tauri/Cargo.toml"
echo "✓ src-tauri/tauri.conf.json"
echo ""
echo "Версия обновлена до $NEW_VERSION"
echo ""
echo "Следующие шаги:"
echo "1. Проверьте изменения: git diff"
echo "2. Закоммитьте: git commit -am 'Bump version to $NEW_VERSION'"
echo "3. Создайте тег: git tag v$NEW_VERSION"
echo "4. Запушьте: git push && git push origin v$NEW_VERSION"
