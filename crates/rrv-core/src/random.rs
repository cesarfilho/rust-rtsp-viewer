//! Bytes aleatórios do sistema (`/dev/urandom`), sem dependência nova: nonces e desafios.

use std::io::Read;

/// Enche `buf` com bytes aleatórios. Sem `/dev/urandom` (não acontece num Linux de verdade) cai para a
/// hora e o pid, que bastam para um identificador mas **não** para um segredo.
pub fn fill(buf: &mut [u8]) {
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(buf))
        .is_ok()
    {
        return;
    }
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    for (i, b) in buf.iter_mut().enumerate() {
        *b = ((t >> ((i % 16) * 8)) as u8) ^ (std::process::id() as u8).wrapping_add(i as u8);
    }
}

/// `n` bytes aleatórios em hexadecimal (`2n` caracteres).
pub fn hex(n: usize) -> String {
    let mut b = vec![0u8; n];
    fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_bytes_differ_and_hex_has_the_right_length() {
        assert_ne!(hex(16), hex(16));
        assert_eq!(hex(16).len(), 32);
        assert!(hex(8).chars().all(|c| c.is_ascii_hexdigit()));
    }
}
