#!/usr/bin/env bash
# Собирает или устанавливает StackPilot на Arch Linux (и производных: Omarchy, Manjaro, EndeavourOS)
# через makepkg / pacman.
#
# Режимы установки:
#   ./install.sh         — сборка текущего локального checkout и установка через pacman
#   ./install.sh --bin   — быстрая установка официального готового бинарника (за 5 секунд, без Node/Rust)
#   ./install.sh --help  — справка
set -euo pipefail

if ! command -v makepkg >/dev/null 2>&1; then
  echo "Ошибка: makepkg не найден — этот скрипт предназначен для Arch Linux и производных (Omarchy, Manjaro, EndeavourOS)." >&2
  echo "Убедитесь, что установлен метапакет base-devel: sudo pacman -S --needed base-devel" >&2
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
MODE="source"

for arg in "$@"; do
  case "$arg" in
    --bin|-b)
      MODE="bin"
      ;;
    --source|-s)
      MODE="source"
      ;;
    --help|-h)
      echo "Использование: $0 [ПАРАМЕТР]"
      echo "Параметры:"
      echo "  --bin, -b     Быстрая установка предсобранного официального релиза (без компиляции)"
      echo "  --source, -s  Сборка из локальных исходных кодов через makepkg (по умолчанию)"
      echo "  --help, -h    Показать эту справку"
      exit 0
      ;;
  esac
done

cd "$SCRIPT_DIR"

if [ "$MODE" = "bin" ]; then
  echo "==> Установка официального готового релиза StackPilot (быстрый режим)..."
  makepkg -si -p PKGBUILD-bin --needed
  echo "==> Готово! StackPilot успешно установлен. Запуск: stackpilot"
  exit 0
fi

# Проверка инструментов для сборки из исходников
if ! command -v pkg-config >/dev/null 2>&1 && ! command -v pkgconf >/dev/null 2>&1; then
  echo "Внимание: pkg-config / pkgconf не найден в системе." >&2
  echo "Для сборки из исходников необходим метапакет base-devel:" >&2
  echo "  sudo pacman -S --needed base-devel" >&2
  echo
  read -rp "Хотите установить готовую бинарную версию вместо сборки из исходников? [Y/n] " choice
  choice="${choice:-Y}"
  if [[ "$choice" =~ ^[YyДд] ]]; then
    makepkg -si -p PKGBUILD-bin --needed
    echo "==> Готово! StackPilot успешно установлен. Запуск: stackpilot"
    exit 0
  fi
fi

PKGVER=$(grep -m1 '^pkgver=' "$SCRIPT_DIR/PKGBUILD" | cut -d= -f2)
ARCHIVE="$SCRIPT_DIR/stackpilot-$PKGVER.tar.gz"

cleanup() { rm -f "$ARCHIVE"; }
trap cleanup EXIT

if git -C "$REPO_ROOT" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  echo "==> Упаковываю текущий git checkout (HEAD) как источник для makepkg..."
  git -C "$REPO_ROOT" archive --prefix="StackPilot-$PKGVER/" -o "$ARCHIVE" HEAD
else
  echo "==> Git-репозиторий не найден, упаковываю рабочие файлы каталога..."
  tar --exclude='.git' \
      --exclude='node_modules' \
      --exclude='target' \
      --exclude='src-tauri/target' \
      --exclude='build' \
      --exclude='dist' \
      --transform "s|^|StackPilot-$PKGVER/|" \
      -czf "$ARCHIVE" -C "$REPO_ROOT" .
fi

echo "==> Собираю и устанавливаю через makepkg -si..."
makepkg -si

echo "==> Готово! StackPilot успешно установлен. Запуск: stackpilot"
