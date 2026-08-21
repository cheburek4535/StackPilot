// ============================================================
// Криптопримитивы без внешних крейтов (crypto.rs)
// ============================================================
// Модулю нужны ровно две вещи:
//   1. SHA-256 — проверка целостности скачанных установщиков
//      (контрольные суммы из tools.json);
//   2. CSPRNG — генерация паролей БД (PostgreSQL) с честной
//      энтропией. Прежняя генерация из наносекунд времени была
//      угадываемой и заменена здесь.
//
// Внешние крейты не используются: SHA-256 реализован по FIPS 180-4
// и покрыт стандартными тестовыми векторами; случайные байты берутся
// из ОС (BCryptGenRandom на Windows через динамическую загрузку,
// /dev/urandom на Unix). Если системный источник недоступен,
// generate_db_password возвращает Err вместо тихой подмены слабым
// источником.

// ------------------------------------------------------------
// SHA-256 (FIPS 180-4)
// ------------------------------------------------------------

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// Пошаговый хешировальщик: можно скармливать файл кусками.
#[derive(Clone)]
pub struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffered: usize,
    len_bytes: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0u8; 64],
            buffered: 0,
            len_bytes: 0,
        }
    }

    pub fn update(&mut self, mut data: &[u8]) {
        self.len_bytes = self.len_bytes.wrapping_add(data.len() as u64);
        if self.buffered > 0 {
            let take = std::cmp::min(64 - self.buffered, data.len());
            self.buffer[self.buffered..self.buffered + take].copy_from_slice(&data[..take]);
            self.buffered += take;
            data = &data[take..];
            if self.buffered == 64 {
                let block = self.buffer;
                self.compress(&block);
                self.buffered = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffered = data.len();
        }
    }

    pub fn finish(mut self) -> [u8; 32] {
        let bit_len = self.len_bytes.wrapping_mul(8);
        // padding: 0x80, нули, 8 байт длины
        self.update(&[0x80]);
        while self.buffered != 56 {
            self.update(&[0]);
        }
        // update() увеличил len_bytes — не важно, битовая длина уже снята
        self.len_bytes = 0;
        self.update(&bit_len.to_be_bytes());
        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}

/// SHA-256 данных одной строкой → hex (нижний регистр).
pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    to_hex(&h.finish())
}

/// SHA-256 файла, читая его потоково (установщики бывают >1 ГБ).
pub fn sha256_file_hex(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| {
        format!(
            "Не удалось открыть {} для проверки суммы: {e}",
            path.display()
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 512 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Ошибка чтения {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finish()))
}

/// Нормализует ожидаемую сумму: hex без разделителей, нижний регистр.
/// Возвращает None для пустых/нечисловых значений (такая сумма не
/// может совпать — источник считается непроверяемым).
pub fn normalize_digest(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect();
    if cleaned.len() == 64 && cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(cleaned)
    } else {
        None
    }
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

// ------------------------------------------------------------
// CSPRNG
// ------------------------------------------------------------

/// Случайные байты из системного источника энтропии.
/// Windows: BCryptGenRandom (динамическая загрузка bcrypt.dll —
/// линковка с системными import-библиотеками не требуется).
/// Unix: /dev/urandom. Ошибка возвращается наружу — вызывающий код
/// обязан НЕ продолжать со слабым «паролем».
pub fn random_bytes(len: usize) -> Result<Vec<u8>, String> {
    if len == 0 {
        return Ok(Vec::new());
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(bytes) = bcrypt_gen_random(len) {
            return Ok(bytes);
        }
        Err("BCryptGenRandom недоступен: системный источник энтропии не найден".to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        use std::io::Read;
        let mut f = std::fs::File::open("/dev/urandom")
            .map_err(|e| format!("/dev/urandom недоступен: {e}"))?;
        let mut out = vec![0u8; len];
        f.read_exact(&mut out)
            .map_err(|e| format!("Не удалось прочитать /dev/urandom: {e}"))?;
        Ok(out)
    }
}

#[cfg(target_os = "windows")]
fn bcrypt_gen_random(len: usize) -> Option<Vec<u8>> {
    use std::os::raw::{c_char, c_int, c_uchar, c_ulong};

    // BCRYPT_USE_SYSTEM_PREFERRED_RNG = 0x00000002
    const USE_SYSTEM_PREFERRED_RNG: c_ulong = 0x0000_0002;

    type BCryptGenRandomFn = unsafe extern "system" fn(
        hAlgorithm: isize,
        pBuffer: *mut c_uchar,
        cbBuffer: c_ulong,
        dwFlags: c_ulong,
    ) -> c_int;

    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryA(lpFileName: *const c_char) -> isize;
        fn GetProcAddress(hModule: isize, lpProcName: *const c_char) -> isize;
    }

    unsafe {
        let dll = b"bcrypt.dll\0";
        let lib = LoadLibraryA(dll.as_ptr() as *const c_char);
        if lib == 0 {
            return None;
        }
        let name = b"BCryptGenRandom\0";
        let addr = GetProcAddress(lib, name.as_ptr() as *const c_char);
        if addr == 0 {
            return None;
        }
        let gen: BCryptGenRandomFn = std::mem::transmute(addr);
        let mut out = vec![0u8; len];
        let status = gen(
            0,
            out.as_mut_ptr(),
            len as c_ulong,
            USE_SYSTEM_PREFERRED_RNG,
        );
        // STATUS_SUCCESS = 0
        if status == 0 {
            Some(out)
        } else {
            None
        }
    }
}

// ------------------------------------------------------------
// Пароли БД
// ------------------------------------------------------------

/// Генерирует пароль БД (PostgreSQL): 16 символов из алфавита
/// без двусмысленных знаков, ~95 бит энтропии от CSPRNG.
/// Алфавит исключает кавычки/обратный слеш/двоеточие — пароль
/// безопасно вставляется в winget --override и SQL без экранирования.
pub fn generate_db_password() -> Result<String, String> {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let bytes = random_bytes(16)?;
    let pw: String = bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect();
    Ok(pw)
}

// ============================================================
// Тесты
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Стандартные векторы FIPS 180-4 / NIST.
    #[test]
    fn sha256_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn sha256_chunked_matches_one_shot() {
        // Куски по 63 байта ломают выравнивание блоков — проверяет буферизацию.
        let data: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let one_shot = sha256_hex(&data);
        let mut chunked = Sha256::new();
        for chunk in data.chunks(63) {
            chunked.update(chunk);
        }
        assert_eq!(to_hex(&chunked.finish()), one_shot);
    }

    #[test]
    fn sha256_large_input_vector() {
        // 'a' × 1 000 000 → известный вектор NIST.
        let mut h = Sha256::new();
        let chunk = [b'a'; 1000];
        for _ in 0..1000 {
            h.update(&chunk);
        }
        assert_eq!(
            to_hex(&h.finish()),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn normalize_digest_accepts_and_rejects() {
        assert_eq!(
            normalize_digest("BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_string())
        );
        assert_eq!(
            normalize_digest("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".to_string())
        );
        // подделка/опечатка не должна притворяться суммой
        assert_eq!(normalize_digest("deadbeef"), None);
        assert_eq!(normalize_digest(""), None);
        assert_eq!(normalize_digest("zzzz"), None);
    }

    #[test]
    fn password_is_strong_alphabet() {
        const ALPHABET: &str = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
        let pw = generate_db_password().unwrap();
        assert_eq!(pw.len(), 16);
        assert!(
            pw.chars().all(|c| ALPHABET.contains(c)),
            "пароль вне безопасного алфавита: {pw}"
        );
        // символы, ломающие shell/SQL/PS-экранирование, запрещены
        assert!(
            !pw.chars().any(|c| "\"'`:;$\\ \0".contains(c)),
            "пароль содержит опасные символы: {pw}"
        );
    }

    #[test]
    fn passwords_are_unique_across_runs() {
        let set: std::collections::HashSet<String> =
            (0..64).map(|_| generate_db_password().unwrap()).collect();
        assert_eq!(set.len(), 64, "CSPRNG не должен повторяться на 64 пробы");
    }

    #[test]
    fn random_bytes_are_random() {
        let a = random_bytes(32).unwrap();
        let b = random_bytes(32).unwrap();
        assert_ne!(a, b);
        assert_eq!(random_bytes(0).unwrap(), Vec::<u8>::new());
    }
}
