#!/usr/bin/env bash
# Удаляет StackPilot, установленный через install.sh/PKGBUILD (pacman).
set -euo pipefail

if ! command -v pacman >/dev/null 2>&1; then
  echo "pacman не найден — этот скрипт только для Arch Linux (и производных)." >&2
  exit 1
fi

if ! pacman -Qi stackpilot >/dev/null 2>&1; then
  echo "Пакет stackpilot не установлен через pacman — удалять нечего." >&2
  exit 1
fi

echo "==> Удаляю пакет stackpilot (и неиспользуемые больше зависимости)..."
sudo pacman -Rns stackpilot

DATA_DIR="$HOME/.local/share/com.cheburek4535.stackpilot"
if [ -d "$DATA_DIR" ]; then
  echo
  echo "Пакет удалён. Настройки, логи и кэш приложения остались в:"
  echo "  $DATA_DIR"
  echo "Если они больше не нужны, удали их вручную: rm -rf \"$DATA_DIR\""
fi
