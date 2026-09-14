#!/usr/bin/env bash
# Собирает и устанавливает StackPilot на Arch Linux через makepkg/pacman,
# используя PKGBUILD рядом с этим скриптом — установка получается
# pacman-tracked (потом чисто удаляется через `sudo pacman -R stackpilot`),
# а не копированием файлов вручную.
#
# source= в PKGBUILD указывает на тег релиза на GitHub, в котором не будет
# твоих локальных коммитов, пока не выпущен новый релиз. makepkg сначала
# ищет одноимённый файл-источник рядом с PKGBUILD и только потом пытается
# его скачать, поэтому этот скрипт упаковывает текущий git HEAD под этим
# именем, и makepkg использует именно его.
set -euo pipefail

if ! command -v makepkg >/dev/null 2>&1; then
  echo "makepkg не найден — этот скрипт только для Arch Linux (и производных)." >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PKGVER=$(grep -m1 '^pkgver=' "$SCRIPT_DIR/PKGBUILD" | cut -d= -f2)
ARCHIVE="$SCRIPT_DIR/stackpilot-$PKGVER.tar.gz"

cleanup() { rm -f "$ARCHIVE"; }
trap cleanup EXIT

echo "==> Упаковываю текущий checkout (git HEAD) как источник для makepkg..."
git -C "$REPO_ROOT" archive --prefix="StackPilot-$PKGVER/" -o "$ARCHIVE" HEAD

echo "==> Собираю и устанавливаю через makepkg -si..."
cd "$SCRIPT_DIR"
makepkg -si

echo "==> Готово. Запуск: stackpilot"
