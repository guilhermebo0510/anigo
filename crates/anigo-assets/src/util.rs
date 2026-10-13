//! Utilitarios pequenos: hex, tempo, escrita atomica, hash em streaming.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{AssetError, IoContext, Result};
use crate::job::CancelToken;
use crate::model::ContentHash;

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 15) as usize] as char);
    }
    s
}

pub(crate) fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    let nib = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    for pair in b.chunks(2) {
        out.push(nib(pair[0])? << 4 | nib(pair[1])?);
    }
    Some(out)
}

/// Segundos desde a epoca Unix (0 se o relogio estiver antes de 1970).
pub fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `mtime` do arquivo em nanossegundos desde a epoca (0 se indisponivel).
pub(crate) fn mtime_ns(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Data civil (UTC) `yyyymmdd` a partir de segundos Unix (algoritmo de Howard Hinnant).
pub(crate) fn yyyymmdd(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}{m:02}{d:02}")
}

/// Escrita atomica: grava em `<path>.tmp-<pid>` e renomeia sobre o destino.
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).at(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}", std::process::id()));
    let tmp = PathBuf::from(tmp);
    {
        let mut f = fs::File::create(&tmp).at(&tmp)?;
        f.write_all(bytes).at(&tmp)?;
        f.flush().at(&tmp)?;
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        AssetError::io(path, e)
    })
}

const CHUNK: usize = 256 * 1024;

/// BLAKE3 de um arquivo em streaming, respeitando cancelamento. Retorna `(hash, bytes_lidos)`.
pub(crate) fn hash_file(path: &Path, cancel: Option<&CancelToken>) -> Result<(ContentHash, u64)> {
    let mut f = fs::File::open(path).at(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        if cancel.map(|c| c.is_cancelled()).unwrap_or(false) {
            return Err(AssetError::Cancelled);
        }
        let n = f.read(&mut buf).at(path)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        total += n as u64;
    }
    Ok((ContentHash(*hasher.finalize().as_bytes()), total))
}

/// Copia `src` para `dst` calculando o BLAKE3 na mesma passada. Cria diretorios pais.
pub(crate) fn copy_and_hash(src: &Path, dst: &Path, cancel: Option<&CancelToken>) -> Result<(ContentHash, u64)> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).at(parent)?;
    }
    let mut r = fs::File::open(src).at(src)?;
    let mut w = fs::File::create(dst).at(dst)?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        if cancel.map(|c| c.is_cancelled()).unwrap_or(false) {
            return Err(AssetError::Cancelled);
        }
        let n = r.read(&mut buf).at(src)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        w.write_all(&buf[..n]).at(dst)?;
        total += n as u64;
    }
    w.flush().at(dst)?;
    Ok((ContentHash(*hasher.finalize().as_bytes()), total))
}

/// Remove `name` de modo a ser seguro para virar nome de arquivo (sem separadores, controle ou pontos iniciais).
pub(crate) fn sanitize_file_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    out = out.trim().trim_start_matches('.').to_string();
    if out.is_empty() {
        out.push_str("asset");
    }
    if out.len() > 120 {
        let mut cut = 120;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    out
}

/// Remove diretorios VAZIOS subindo de `start` ate (sem incluir) `stop`. Para no primeiro nao-vazio.
pub(crate) fn prune_empty_parents(start: &Path, stop: &Path) {
    let mut cur = start.parent().map(|p| p.to_path_buf());
    while let Some(dir) = cur {
        if dir == stop || !dir.starts_with(stop) {
            break;
        }
        if std::fs::remove_dir(&dir).is_err() {
            break;
        }
        cur = dir.parent().map(|p| p.to_path_buf());
    }
}

/// Tamanho recursivo de um diretorio (ignora erros por entrada).
pub(crate) fn dir_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let data = [0u8, 1, 15, 16, 254, 255];
        let h = hex_encode(&data);
        assert_eq!(h, "00010f10feff");
        assert_eq!(hex_decode(&h).unwrap(), data);
        assert!(hex_decode("abc").is_none());
        assert!(hex_decode("zz").is_none());
        assert!(hex_decode("\u{e9}\u{e9}").is_none());
    }

    #[test]
    fn civil_date() {
        assert_eq!(yyyymmdd(0), "19700101");
        assert_eq!(yyyymmdd(951_782_400), "20000229"); // ano bissexto
        assert_eq!(yyyymmdd(1_791_763_200), "20261012");
        assert_eq!(yyyymmdd(-1), "19691231");
    }

    #[test]
    fn sanitize_names() {
        assert_eq!(sanitize_file_name("a/b\\c"), "a_b_c");
        assert_eq!(sanitize_file_name("..hidden"), "hidden");
        assert_eq!(sanitize_file_name(""), "asset");
        assert_eq!(sanitize_file_name(&"x".repeat(500)).len(), 120);
    }

    #[test]
    fn copy_hash_matches_blake3() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.bin");
        fs::write(&src, vec![7u8; 1_000_000]).unwrap();
        let dst = dir.path().join("sub/b.bin");
        let (h, n) = copy_and_hash(&src, &dst, None).unwrap();
        assert_eq!(n, 1_000_000);
        assert_eq!(h.0, *blake3::hash(&fs::read(&src).unwrap()).as_bytes());
        assert_eq!(hash_file(&dst, None).unwrap().0, h);
    }

    #[test]
    fn hash_respects_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.bin");
        fs::write(&src, vec![1u8; 10]).unwrap();
        let c = CancelToken::new();
        c.cancel();
        assert!(matches!(hash_file(&src, Some(&c)), Err(AssetError::Cancelled)));
    }
}
