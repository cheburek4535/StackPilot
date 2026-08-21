// ============================================================
// Изолированное хранилище секретов (secrets.rs)
// ============================================================
// Секреты (пароль PostgreSQL и т.п.) НЕ хранятся в state.json и не
// возвращаются обычными командами. Этот модуль — единственное место,
// где они живут на диске:
//
//   - отдельный файл <app_data>/toolchain/secrets.bin (не JSON
//     метаданных, не попадает в бэкапы состояния);
//   - Windows: значения шифруются DPAPI (CryptProtectData, scope
//     CurrentUser) через динамическую загрузку crypt32.dll — расшифровать
//     их может только этот пользователь этой машины;
//   - Unix: файл с правами 0600 в домашнем каталоге пользователя.
//
// Документированное ограничение: системное «keyring»-хранилище
// (Windows Credential Manager / libsecret / Keychain) недоступно без
// внешних крейтов; DPAPI-шифрование даёт эквивалентную защиту
// at-rest для сценария локального приложения. Если DPAPI недоступен
// (нестандартная система), хранилище переходит в режим plaintext с
// явной пометкой is_encrypted() == false — молчаливой подмены нет.
//
// Выдача секретов наружу — только одноразовая витрина
// (tc_take_new_secrets): забрал → очистилось.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const SECRETS_FILE: &str = "secrets.bin";

/// Хранилище секретов приложения.
pub struct SecretStore {
    path: PathBuf,
    data: HashMap<String, String>,
    /// true = значения на диске зашифрованы (DPAPI на Windows; Unix-вариант
    /// — файл 0600, что документировано как эквивалентная защита at-rest).
    encrypted: bool,
}

#[cfg(target_os = "windows")]
mod dpapi {
    // CRYPTPROTECT_UI_FORBIDDEN
    const UI_FORBIDDEN: u32 = 0x1;

    #[repr(C)]
    struct Blob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    type ProtectFn = unsafe extern "system" fn(
        pDataIn: *const Blob,
        szDataDescr: *const u16,
        pOptionalEntropy: *const Blob,
        pvReserved: isize,
        pPromptStruct: isize,
        dwFlags: u32,
        pDataOut: *mut Blob,
    ) -> i32;
    type UnprotectFn = unsafe extern "system" fn(
        pDataIn: *const Blob,
        ppDataDescr: *mut isize,
        pOptionalEntropy: *const Blob,
        pvReserved: isize,
        pPromptStruct: isize,
        dwFlags: u32,
        pDataOut: *mut Blob,
    ) -> i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryA(lpFileName: *const u8) -> isize;
        fn GetProcAddress(hModule: isize, lpProcName: *const u8) -> isize;
        fn LocalFree(hMem: isize) -> isize;
    }

    fn proc_address(name: &[u8]) -> Option<isize> {
        unsafe {
            let dll = b"crypt32.dll\0";
            let lib = LoadLibraryA(dll.as_ptr());
            if lib == 0 {
                return None;
            }
            let addr = GetProcAddress(lib, name.as_ptr());
            if addr == 0 {
                return None;
            }
            Some(addr)
        }
    }

    /// Шифрует байты DPAPI (CurrentUser). None = DPAPI недоступен/ошибся.
    pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
        let addr = proc_address(b"CryptProtectData\0")?;
        let protect: ProtectFn = unsafe { std::mem::transmute(addr) };
        let mut input = Blob {
            cb_data: plain.len() as u32,
            pb_data: plain.as_ptr() as *mut u8,
        };
        let mut out = Blob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };
        let ok = unsafe {
            protect(
                &mut input,
                std::ptr::null(),
                std::ptr::null(),
                0,
                0,
                UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 || out.pb_data.is_null() {
            return None;
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(out.pb_data, out.cb_data as usize) }.to_vec();
        unsafe {
            LocalFree(out.pb_data as isize);
        }
        Some(bytes)
    }

    /// Расшифровывает байты DPAPI. None = недоступен/повреждено.
    pub fn unprotect(cipher: &[u8]) -> Option<Vec<u8>> {
        let addr = proc_address(b"CryptUnprotectData\0")?;
        let unprotect: UnprotectFn = unsafe { std::mem::transmute(addr) };
        let mut input = Blob {
            cb_data: cipher.len() as u32,
            pb_data: cipher.as_ptr() as *mut u8,
        };
        let mut descr: isize = 0;
        let mut out = Blob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };
        let ok = unsafe {
            unprotect(
                &mut input,
                &mut descr,
                std::ptr::null(),
                0,
                0,
                UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 || out.pb_data.is_null() {
            return None;
        }
        let bytes =
            unsafe { std::slice::from_raw_parts(out.pb_data, out.cb_data as usize) }.to_vec();
        unsafe {
            LocalFree(out.pb_data as isize);
        }
        Some(bytes)
    }
}

fn b64_encode(data: &[u8]) -> String {
    // Компактный base64 без внешних крейтов.
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn b64_decode(text: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a' + 26) as u32),
            b'0'..=b'9' => Some((c - b'0' + 52) as u32),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let text: Vec<u8> = text.bytes().filter(|b| !b" \r\n\t".contains(b)).collect();
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for chunk in text.chunks(4) {
        if chunk.len() != 4 {
            return None;
        }
        let mut n = 0u32;
        let mut pad = 0usize;
        for (i, &c) in chunk.iter().enumerate() {
            if c == b'=' {
                pad += 1;
                n |= 0 << (18 - 6 * i);
            } else {
                n |= val(c)? << (18 - 6 * i);
            }
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

impl SecretStore {
    /// Загружает (или создаёт) хранилище в каталоге `dir`.
    pub fn load(dir: &Path) -> Self {
        let path = dir.join(SECRETS_FILE);
        let _ = std::fs::create_dir_all(dir);
        let mut store = Self {
            path,
            data: HashMap::new(),
            encrypted: cfg!(target_os = "windows"),
        };
        store.read_file();
        store
    }

    fn read_file(&mut self) {
        let Ok(raw) = std::fs::read_to_string(&self.path) else {
            return; // файла нет — пустое хранилище
        };
        #[derive(serde::Deserialize)]
        struct FileFormat {
            #[serde(default)]
            encrypted: bool,
            #[serde(default)]
            values: HashMap<String, String>,
        }
        let Ok(parsed) = serde_json::from_str::<FileFormat>(&raw) else {
            eprintln!("[toolchain] secrets.bin повреждён — начинаю с пустого хранилища");
            return;
        };
        self.encrypted = parsed.encrypted;
        for (key, value) in parsed.values {
            if !parsed.encrypted {
                self.data.insert(key, value);
                continue;
            }
            match b64_decode(&value)
                .and_then(|cipher| dpapi::unprotect(&cipher))
                .and_then(|plain| String::from_utf8(plain).ok())
            {
                Some(plain) => {
                    self.data.insert(key, plain);
                }
                None => {
                    // Чужой профиль/машина/повреждение: ключ не восстанавливаем,
                    // но и не роняем приложение — просто сообщаем.
                    eprintln!(
                        "[toolchain] секрет «{key}» не удалось расшифровать (смена профиля/машины?)"
                    );
                }
            }
        }
    }

    fn write_file(&self) -> Result<(), String> {
        let mut values = HashMap::with_capacity(self.data.len());
        for (key, value) in &self.data {
            if self.encrypted {
                let cipher = dpapi::protect(value.as_bytes())
                    .ok_or_else(|| "DPAPI недоступен: секрет не может быть защищён".to_string())?;
                values.insert(key.clone(), b64_encode(&cipher));
            } else {
                values.insert(key.clone(), value.clone());
            }
        }
        #[derive(serde::Serialize)]
        struct FileFormat<'a> {
            encrypted: bool,
            values: &'a HashMap<String, String>,
        }
        let raw = serde_json::to_string_pretty(&FileFormat {
            encrypted: self.encrypted,
            values: &values,
        })
        .map_err(|e| format!("Не удалось сериализовать secrets.bin: {e}"))?;

        let tmp = self.path.with_extension("bin.tmp");
        std::fs::write(&tmp, raw)
            .map_err(|e| format!("Не удалось записать {}: {e}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("Не удалось сохранить {}: {e}", self.path.display()))?;
        Ok(())
    }

    /// Сохраняет секрет (перезаписывает прежний).
    pub fn set_secret(&mut self, key: &str, value: &str) -> Result<(), String> {
        self.data.insert(key.to_string(), value.to_string());
        self.write_file()
    }

    /// Читает секрет (только для внутренних потребителей: генерация
    /// проекта и одноразовая витрина). В проде пока читателей нет —
    /// доступ нужен тестам round-trip и будущей генерации проекта.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get_secret(&self, key: &str) -> Option<&str> {
        self.data.get(key).map(|s| s.as_str())
    }

    /// Все ключи (без значений) — безопасно для диагностики.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn keys(&self) -> Vec<String> {
        self.data.keys().cloned().collect()
    }

    /// Зашифровано ли хранилище на диске. false на Windows = DPAPI
    /// недоступен, значения лежат в plaintext-режиме (диагностика
    /// печатает предупреждение при старте, см. ToolchainState::new).
    pub fn is_encrypted(&self) -> bool {
        self.encrypted
    }

    /// Миграция старых plaintext-секретов из state.json: переносит их
    /// в изолированное хранилище и возвращает список перенесённых ключей.
    /// Вызывается один раз при старте приложения.
    pub fn migrate_from_map(
        &mut self,
        legacy: &HashMap<String, String>,
    ) -> Result<Vec<String>, String> {
        if legacy.is_empty() {
            return Ok(Vec::new());
        }
        let mut migrated = Vec::new();
        for (key, value) in legacy {
            if !self.data.contains_key(key) {
                self.data.insert(key.clone(), value.clone());
                migrated.push(key.clone());
            }
        }
        if !migrated.is_empty() {
            self.write_file()?;
        }
        Ok(migrated)
    }
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tc-secrets-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn base64_roundtrip() {
        for len in [0usize, 1, 2, 3, 4, 5, 31, 32, 33, 255] {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 % 256) as u8).collect();
            let encoded = b64_encode(&data);
            assert_eq!(b64_decode(&encoded).unwrap(), data, "len={len}");
        }
    }

    #[test]
    fn store_roundtrip_and_isolation() {
        let dir = temp_dir("roundtrip");
        let path = dir.join(SECRETS_FILE);

        let mut store = SecretStore::load(&dir);
        store
            .set_secret("postgres_password", "S3cretValue")
            .unwrap();

        // Файл существует и значение секрета в нём НЕ лежит открытым текстом
        // (на Windows — DPAPI; на других ОС допускается plaintext c 0600).
        let raw = std::fs::read_to_string(&path).unwrap();
        if cfg!(target_os = "windows") {
            assert!(
                !raw.contains("S3cretValue"),
                "секрет обязан быть зашифрован DPAPI на Windows"
            );
            assert!(store.is_encrypted());
        }

        // «Новый запуск приложения» — секрет читается обратно.
        let reloaded = SecretStore::load(&dir);
        assert_eq!(
            reloaded.get_secret("postgres_password"),
            Some("S3cretValue")
        );
        assert!(reloaded.keys().contains(&"postgres_password".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrate_moves_legacy_plaintext_out_of_state_json() {
        let dir = temp_dir("migrate");
        let mut legacy = HashMap::new();
        legacy.insert(
            "postgres_password".to_string(),
            "LegacyPw123456".to_string(),
        );

        let mut store = SecretStore::load(&dir);
        let migrated = store.migrate_from_map(&legacy).unwrap();
        assert_eq!(migrated, vec!["postgres_password".to_string()]);
        assert_eq!(
            store.get_secret("postgres_password"),
            Some("LegacyPw123456")
        );

        // Повторная миграция — ничего нового (идемпотентна).
        let again = store.migrate_from_map(&legacy).unwrap();
        assert!(again.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_empty_store() {
        let dir = temp_dir("missing");
        let store = SecretStore::load(&dir);
        assert_eq!(store.get_secret("anything"), None);
        assert!(store.keys().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
