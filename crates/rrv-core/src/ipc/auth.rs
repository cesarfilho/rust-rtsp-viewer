//! Autenticação do canal TCP (plano: janela e daemon em máquinas diferentes).
//!
//! O daemon e a janela dividem um **token**. Na conexão o daemon manda um desafio aleatório (`nonce`) e
//! a janela responde com `HMAC-SHA256(token, nonce)` em hexadecimal: **o token nunca trafega**, e uma
//! resposta gravada por quem escuta a rede não serve em outra conexão (o desafio muda). O que esta
//! escolha **não** dá: criptografia nem integridade do que vem depois da autenticação. Os dados (nomes
//! das câmeras, eventos, caixas de detecção, os vídeos gravados) viajam em texto puro: use só numa LAN
//! de confiança, ou por uma VPN / túnel SSH.

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// O token precisa ter pelo menos isto: com menos, adivinhar offline a partir de um desafio e uma
/// resposta capturados é viável.
pub const MIN_TOKEN_LEN: usize = 16;

/// Um desafio novo: 16 bytes aleatórios em hexadecimal.
pub fn new_challenge() -> String {
    crate::random::hex(16)
}

fn mac(token: &str, nonce: &str) -> HmacSha256 {
    // HMAC aceita chave de qualquer tamanho: `new_from_slice` não falha.
    let mut m = HmacSha256::new_from_slice(token.as_bytes()).expect("HMAC aceita qualquer chave");
    m.update(nonce.as_bytes());
    m
}

/// A resposta da janela: `HMAC-SHA256(token, nonce)` em hexadecimal.
pub fn respond(token: &str, nonce: &str) -> String {
    mac(token, nonce)
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// A resposta confere? A comparação é em tempo constante (a do próprio HMAC).
pub fn verify(token: &str, nonce: &str, response: &str) -> bool {
    from_hex(response).is_some_and(|bytes| mac(token, nonce).verify_slice(&bytes).is_ok())
}

/// `Err` com o motivo se `token` não serve como segredo do canal.
pub fn validate_token(token: &str) -> Result<(), String> {
    if token.chars().count() < MIN_TOKEN_LEN {
        return Err(format!(
            "o token precisa ter pelo menos {MIN_TOKEN_LEN} caracteres (gere um com: openssl rand -hex 24)"
        ));
    }
    if token.chars().any(char::is_whitespace) {
        return Err("o token não pode ter espaços".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_sha256_matches_the_rfc_4231_test_vector() {
        // RFC 4231, caso 2: chave "Jefe", dado "what do ya want for nothing?"
        let m = {
            let mut m = HmacSha256::new_from_slice(b"Jefe").unwrap();
            m.update(b"what do ya want for nothing?");
            m.finalize().into_bytes()
        };
        let hex: String = m.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn the_right_token_answers_the_challenge_and_the_wrong_one_does_not() {
        let nonce = new_challenge();
        let ok = respond("token-secreto-123456", &nonce);
        assert!(verify("token-secreto-123456", &nonce, &ok));
        assert!(!verify("outro-token-654321xx", &nonce, &ok));
        assert!(
            !verify("token-secreto-123456", &new_challenge(), &ok),
            "outro desafio"
        );
    }

    #[test]
    fn garbage_responses_are_refused_without_panicking() {
        let nonce = "abc";
        for bad in ["", "zz", "123", "é", &"0".repeat(64), &"f".repeat(2000)] {
            assert!(!verify("token-secreto-123456", nonce, bad), "{bad:?}");
        }
    }

    #[test]
    fn challenges_are_unique_and_32_hex_characters() {
        let (a, b) = (new_challenge(), new_challenge());
        assert_ne!(a, b);
        assert_eq!(a.len(), 32);
    }

    #[test]
    fn short_or_spaced_tokens_are_not_secrets() {
        assert!(validate_token("curto").is_err());
        assert!(validate_token("tem espaço no meio do token").is_err());
        assert!(validate_token("0123456789abcdef").is_ok());
        assert!(validate_token("0123456789abcde").is_err(), "15 caracteres");
    }
}
