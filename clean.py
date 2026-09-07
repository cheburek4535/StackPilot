#!/usr/bin/env python3
"""
Скрипт для удаления тестов из XML дампа Repomix.
Также удаляет раздел "presets" из wizard_tree.json, если указан флаг -dp/--dp.

Флаг -rn/--rename <name> записывает результат как <name>.xml
(переопределяет -o/--output).
"""

import re
import sys
import argparse
import json
import xml.sax.saxutils
from pathlib import Path
from typing import List, Tuple, Optional

# Консольный вывод с эмодзи ломается под cp1251 (cmd -> python из bat)
for _stream in (sys.stdout, sys.stderr):
    try:
        _stream.reconfigure(encoding='utf-8', errors='replace')
    except (AttributeError, ValueError):
        pass


class RepomixTestCleaner:
    def __init__(self, input_file: Path, output_file: Optional[Path] = None, remove_presets: bool = False):
        self.input_file = input_file
        self.output_file = output_file or input_file.with_suffix('.clean.xml')
        self.remove_presets = remove_presets
        self.stats = {
            'files_cleaned': 0,
            'tests_removed': 0,
            'total_files': 0,
            'presets_removed': False
        }

    def remove_presets_from_json(self, content: str) -> Tuple[str, int]:
        """Удаляет раздел 'presets' из JSON содержимого"""
        try:
            data = json.loads(content)
        except json.JSONDecodeError as e:
            print(f"   ⚠️  JSON decode error: {e}")
            return content, 0

        if 'presets' not in data:
            print(f"   ℹ️  No 'presets' key found in JSON")
            return content, 0

        presets_count = len(data.get('presets', []))
        print(f"   🗑️  Found 'presets' with {presets_count} items, removing...")

        del data['presets']
        new_content = json.dumps(data, indent=2, ensure_ascii=False)
        return new_content, 1

    def clean_file_content(self, content: str, file_path: str = "") -> Tuple[str, int, bool]:
        """
        Очищает содержимое одного файла от тестов и (опционально) presets.
        Возвращает (новое_содержимое, количество_удалений, был_ли_изменён_JSON)
        """
        removed_count = 0
        modified_json = False

        # Специальная обработка для wizard_tree.json - удаляем presets только если флаг установлен
        if self.remove_presets and 'wizard_tree.json' in file_path:
            # Декодируем XML-сущности, чтобы получить валидный JSON
            decoded_content = xml.sax.saxutils.unescape(content)
            new_json, removed = self.remove_presets_from_json(decoded_content)
            if removed > 0:
                # Кодируем обратно XML-сущности для вставки в XML
                content = xml.sax.saxutils.escape(new_json)
                removed_count += removed
                self.stats['presets_removed'] = True
                modified_json = True

        # Удаление тестов (работаем с содержимым как есть, без декодирования)
        # 1. Удаляем блоки mod tests { ... }
        def remove_mod_tests(text: str) -> str:
            nonlocal removed_count
            result = []
            i = 0
            n = len(text)

            while i < n:
                match = re.search(r'^\s*mod\s+tests\s*\{', text[i:], re.MULTILINE)
                if not match:
                    result.append(text[i:])
                    break

                result.append(text[i:i + match.start()])
                i += match.start()

                brace_count = 0
                found_open = False

                for j in range(i, n):
                    if text[j] == '{':
                        brace_count = 1
                        i = j + 1
                        found_open = True
                        break

                if not found_open:
                    result.append(text[i:])
                    break

                while i < n and brace_count > 0:
                    if text[i] == '{':
                        brace_count += 1
                    elif text[i] == '}':
                        brace_count -= 1
                    i += 1

                removed_count += 1

                if i < n and text[i] in ';,':
                    i += 1

            return ''.join(result)

        # 2. Удаляем атрибуты тестов и их функции
        def remove_test_attributes(text: str) -> str:
            nonlocal removed_count
            lines = text.split('\n')
            new_lines = []
            i = 0

            while i < len(lines):
                line = lines[i]

                if re.search(r'^\s*#\[(test|tokio::test)\]', line):
                    removed_count += 1
                    i += 1

                    if i < len(lines):
                        next_line = lines[i]
                        if re.search(r'^\s*(pub\s+)?(async\s+)?fn\s+', next_line):
                            removed_count += 1
                            i += 1

                            brace_count = 0
                            found_body = False

                            while i < len(lines):
                                if re.search(r'\{', lines[i]):
                                    brace_count = 1
                                    i += 1
                                    found_body = True
                                    break
                                i += 1

                            if found_body:
                                while i < len(lines) and brace_count > 0:
                                    brace_count += lines[i].count('{') - lines[i].count('}')
                                    i += 1
                        else:
                            new_lines.append(next_line)
                    continue

                if re.search(r'^\s*#\[cfg\(test\)\]', line):
                    removed_count += 1
                    i += 1
                    continue

                if re.search(r'^\s*#\[test_case', line):
                    removed_count += 1
                    i += 1
                    continue

                if re.search(r'^\s*(pub\s+)?(async\s+)?fn\s+test_', line):
                    removed_count += 1
                    i += 1
                    brace_count = 0
                    found_body = False

                    while i < len(lines):
                        if re.search(r'\{', lines[i]):
                            brace_count = 1
                            i += 1
                            found_body = True
                            break
                        i += 1

                    if found_body:
                        while i < len(lines) and brace_count > 0:
                            brace_count += lines[i].count('{') - lines[i].count('}')
                            i += 1
                    continue

                new_lines.append(line)
                i += 1

            return '\n'.join(new_lines)

        # Применяем удаление тестов только если есть что удалять
        if re.search(r'(mod\s+tests\s*\{|#\[test\]|#\[tokio::test\]|#\[cfg\(test\)\]|fn\s+test_)', content):
            content = remove_mod_tests(content)
            content = remove_test_attributes(content)
            content = re.sub(r'\n\s*\n\s*\n', '\n\n', content)

        return content, removed_count, modified_json

    def extract_file_content(self, text: str) -> List[Tuple[str, str]]:
        """Извлекает содержимое файлов из XML без декодирования"""
        files = []
        pattern = re.compile(
            r'<file\s+path="([^"]+)"[^>]*>(.*?)</file>',
            re.DOTALL
        )

        for match in pattern.finditer(text):
            file_path = match.group(1)
            content = match.group(2)
            files.append((file_path, content))

        return files

    def process(self) -> None:
        """Обрабатывает XML файл"""
        print(f"📂 Reading file: {self.input_file}")

        try:
            with open(self.input_file, 'r', encoding='utf-8') as f:
                content = f.read()
        except Exception as e:
            print(f"❌ Error reading file: {e}")
            sys.exit(1)

        files = self.extract_file_content(content)
        self.stats['total_files'] = len(files)

        print(f"📊 Found {self.stats['total_files']} files")

        if self.stats['total_files'] == 0:
            print("⚠️  No files found in XML dump")
            return

        processed_files = []
        for file_path, file_content in files:
            has_tests = re.search(r'(mod\s+tests\s*\{|#\[test\]|#\[tokio::test\]|#\[cfg\(test\)\]|fn\s+test_)', file_content)
            has_presets = self.remove_presets and 'wizard_tree.json' in file_path and '"presets"' in file_content

            if has_tests or has_presets:
                cleaned_content, removed, modified_json = self.clean_file_content(file_content, file_path)
                if removed > 0:
                    self.stats['files_cleaned'] += 1
                    self.stats['tests_removed'] += removed
                    processed_files.append((file_path, cleaned_content))
                    if has_tests and not has_presets:
                        print(f"🧹 {file_path} - removed {removed} tests")
                    elif has_presets and not has_tests:
                        print(f"🧹 {file_path} - removed presets section")
                    else:
                        print(f"🧹 {file_path} - removed {removed} items")
                else:
                    processed_files.append((file_path, file_content))
            else:
                processed_files.append((file_path, file_content))

        print("\n📝 Building cleaned XML...")

        # Находим начало и конец блока <files>
        start_match = re.search(r'<files>', content)
        end_match = re.search(r'</files>', content)

        if not start_match or not end_match:
            print("❌ <files> block not found in XML")
            sys.exit(1)

        # Строим новый XML
        new_content = content[:start_match.end()]

        for file_path, file_content in processed_files:
            # Вставляем содержимое как есть (оно уже корректно экранировано или мы сами экранировали при изменении)
            new_content += f'\n<file path="{file_path}">\n{file_content}\n</file>'

        new_content += f'\n{content[end_match.start():]}'

        try:
            with open(self.output_file, 'w', encoding='utf-8') as f:
                f.write(new_content)
            print(f"\n✅ Cleaned dump saved: {self.output_file}")
        except Exception as e:
            print(f"❌ Error saving: {e}")
            sys.exit(1)

        print("\n📊 Statistics:")
        print(f"  Total files: {self.stats['total_files']}")
        print(f"  Cleaned files: {self.stats['files_cleaned']}")
        print(f"  Removed items: {self.stats['tests_removed']}")
        if self.stats['presets_removed']:
            print(f"  🗑️  Presets section removed from wizard_tree.json")


def main():
    parser = argparse.ArgumentParser(
        description="Cleans Repomix XML dump from tests and optionally from presets"
    )
    parser.add_argument(
        'input',
        help='Input XML file from Repomix'
    )
    parser.add_argument(
        '-o', '--output',
        help='Output file (default: input_file.clean.xml)'
    )
    parser.add_argument(
        '-dp', '--dp',
        action='store_true',
        help='Remove presets section from wizard_tree.json'
    )
    parser.add_argument(
        '-rn', '--rename',
        help='Write cleaned dump as <name>.xml (overrides -o/--output)'
    )

    args = parser.parse_args()

    input_file = Path(args.input)
    if not input_file.exists():
        print(f"❌ File not found: {input_file}")
        sys.exit(1)

    output_file = None
    if args.rename:
        out_name = args.rename
        if not out_name.lower().endswith('.xml'):
            out_name += '.xml'
        output_file = input_file.parent / out_name
    elif args.output:
        output_file = Path(args.output)

    cleaner = RepomixTestCleaner(input_file, output_file, remove_presets=args.dp)
    cleaner.process()


if __name__ == "__main__":
    main()