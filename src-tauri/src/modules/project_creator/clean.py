#!/usr/bin/env python3
"""
Скрипт для удаления тестов из XML дампа Repomix.
"""

import re
import sys
import argparse
from pathlib import Path
from typing import List, Tuple, Optional

class RepomixTestCleaner:
    def __init__(self, input_file: Path, output_file: Optional[Path] = None):
        self.input_file = input_file
        self.output_file = output_file or input_file.with_suffix('.clean.xml')
        self.stats = {
            'files_cleaned': 0,
            'tests_removed': 0,
            'total_files': 0
        }
    
    def clean_file_content(self, content: str) -> Tuple[str, int]:
        """Очищает содержимое одного файла от тестов"""
        removed_count = 0
        
        # 1. Удаляем блоки mod tests { ... } с вложенными блоками (включая весь блок)
        def remove_mod_tests(text: str) -> str:
            nonlocal removed_count
            result = []
            i = 0
            n = len(text)
            
            while i < n:
                # Ищем начало mod tests
                match = re.search(r'^\s*mod\s+tests\s*\{', text[i:], re.MULTILINE)
                if not match:
                    result.append(text[i:])
                    break
                
                result.append(text[i:i + match.start()])
                i += match.start()
                
                # Находим открывающую скобку
                brace_count = 0
                found_open = False
                start_pos = i
                
                for j in range(i, n):
                    if text[j] == '{':
                        brace_count = 1
                        i = j + 1
                        found_open = True
                        break
                
                if not found_open:
                    result.append(text[i:])
                    break
                
                # Пропускаем весь блок до закрытия всех скобок
                while i < n and brace_count > 0:
                    if text[i] == '{':
                        brace_count += 1
                    elif text[i] == '}':
                        brace_count -= 1
                    i += 1
                
                removed_count += 1
                
                # Пропускаем возможную запятую или точку с запятой после блока
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
                
                # Проверяем атрибуты тестов
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
                
                # Пропускаем #[cfg(test)]
                if re.search(r'^\s*#\[cfg\(test\)\]', line):
                    removed_count += 1
                    i += 1
                    continue
                
                # Пропускаем #[test_case] и другие test атрибуты
                if re.search(r'^\s*#\[test_case', line):
                    removed_count += 1
                    i += 1
                    continue
                
                # Пропускаем функции, начинающиеся с test_
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
        
        # Применяем все очистки
        content = remove_mod_tests(content)
        content = remove_test_attributes(content)
        
        # Удаляем пустые строки и комментарии с тестами
        content = re.sub(r'\n\s*\n\s*\n', '\n\n', content)
        
        return content, removed_count
    
    def extract_file_content(self, text: str) -> List[Tuple[str, str]]:
        """Извлекает содержимое файлов из XML"""
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
            # Проверяем наличие тестов
            if re.search(r'(mod\s+tests\s*\{|#\[test\]|#\[tokio::test\]|#\[cfg\(test\)\]|fn\s+test_)', file_content):
                cleaned_content, removed = self.clean_file_content(file_content)
                if removed > 0:
                    self.stats['files_cleaned'] += 1
                    self.stats['tests_removed'] += removed
                    processed_files.append((file_path, cleaned_content))
                    print(f"🧹 {file_path} - removed {removed} tests")
                else:
                    processed_files.append((file_path, file_content))
            else:
                processed_files.append((file_path, file_content))
        
        print("\n📝 Building cleaned XML...")
        
        start_match = re.search(r'<files>', content)
        end_match = re.search(r'</files>', content)
        
        if not start_match or not end_match:
            print("❌ <files> block not found in XML")
            sys.exit(1)
        
        new_content = content[:start_match.end()]
        
        for file_path, file_content in processed_files:
            new_content += f'\n<file path="{file_path}">\n{file_content}\n</file>'
        
        new_content += f'\n{content[end_match.start():]}'
        
        # Перезаписываем только очищенный файл
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
        print(f"  Removed tests: {self.stats['tests_removed']}")


def main():
    parser = argparse.ArgumentParser(
        description="Cleans Repomix XML dump from tests"
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
        '--dry-run',
        action='store_true',
        help='Show what would be removed without saving'
    )
    
    args = parser.parse_args()
    
    input_file = Path(args.input)
    if not input_file.exists():
        print(f"❌ File not found: {input_file}")
        sys.exit(1)
    
    output_file = Path(args.output) if args.output else None
    
    cleaner = RepomixTestCleaner(input_file, output_file)
    cleaner.process()


if __name__ == "__main__":
    main()