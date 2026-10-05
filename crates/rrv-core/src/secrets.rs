//! Segredos fora do `config.toml` (plano 2.5.8).
//!
//! Qualquer URL do config pode levar marcadores `${NOME}`; o valor vem da
//! variável de ambiente `NOME` ou, se ela não existir, do arquivo
//! `$RRV_SECRETS_DIR/NOME` (padrão `/run/secrets`, onde o Docker monta os
//! *secrets*). Assim a senha da câmera não fica no arquivo de configuração, que
//! costuma ir para um volume, um repositório ou um backup.
//!
//! - `$${` escreve um `${` literal.
//! - Na parte de usuário/senha de uma URL (`esquema://AQUI@host`), o valor é
//!   **percent-encoded** sozinho: uma senha com `@`, `/` ou `:` não quebra a URL.
//! - Um segredo ausente é erro que **nomeia o segredo, nunca o valor**.

use std::path::PathBuf;

/// Onde procurar um segredo que não é variável de ambiente.
pub fn secrets_dir() -> PathBuf {
    std::env::var_os("RRV_SECRETS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/secrets"))
}

/// O valor de `name`: variável de ambiente, senão arquivo no diretório de
/// segredos (sem o `\n` final que editores e `echo` deixam).
pub fn lookup(name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(name) {
        return Some(v);
    }
    // O nome vira nome de arquivo: nada de sair do diretório.
    if name.contains(['/', '\\']) || name == ".." || name == "." {
        return None;
    }
    std::fs::read_to_string(secrets_dir().join(name))
        .ok()
        .map(|s| s.trim_end_matches(['\n', '\r']).to_string())
}

/// Percent-encoding do que quebraria o campo de usuário/senha de uma URL.
fn encode_userinfo(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'!' | b'*' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Expande os `${NOME}` de `input` com `get`. `Err` lista os segredos que não
/// existem (sem repetir).
pub fn expand(input: &str, get: &dyn Fn(&str) -> Option<String>) -> Result<String, Vec<String>> {
    let mut out = String::with_capacity(input.len());
    let mut missing: Vec<String> = Vec::new();
    // O trecho de usuário/senha vai de `://` até o primeiro `@` antes de qualquer `/`.
    let userinfo = input.find("://").and_then(|i| {
        let start = i + 3;
        let rest = &input[start..];
        let at = rest.find('@')?;
        let slash = rest.find('/').unwrap_or(rest.len());
        (at < slash).then_some(start..start + at)
    });
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < input.len() {
        if input[i..].starts_with("$${") {
            out.push_str("${");
            i += 3;
        } else if input[i..].starts_with("${")
            && let Some(end) = input[i + 2..].find('}')
        {
            let name = &input[i + 2..i + 2 + end];
            if valid_name(name) {
                match get(name) {
                    Some(v) => {
                        let in_userinfo = userinfo.as_ref().is_some_and(|r| r.contains(&i));
                        out.push_str(&if in_userinfo { encode_userinfo(&v) } else { v });
                    }
                    None => {
                        if !missing.iter().any(|m| m == name) {
                            missing.push(name.to_string());
                        }
                        out.push_str(&input[i..i + 3 + end]);
                    }
                }
                i += 3 + end;
            } else {
                out.push('$');
                i += 1;
            }
        } else {
            // um caractere (UTF-8 inteiro)
            let ch_len = utf8_len(bytes[i]);
            out.push_str(&input[i..i + ch_len]);
            i += ch_len;
        }
    }
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(missing)
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        b if b < 0x80 => 1,
        b if b >> 5 == 0b110 => 2,
        b if b >> 4 == 0b1110 => 3,
        _ => 4,
    }
}

/// A mensagem para um segredo ausente: diz onde ele foi procurado, não o valor.
pub fn missing_message(field: &str, names: &[String]) -> String {
    let dir = secrets_dir();
    names
        .iter()
        .map(|n| {
            format!(
                "{field}: o segredo '{n}' não existe (variável de ambiente {n} ou arquivo {})",
                dir.join(n).display()
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn text_without_markers_is_untouched() {
        assert_eq!(
            expand("rtsp://admin:senha@10.0.0.2/stream", &with(&[])).unwrap(),
            "rtsp://admin:senha@10.0.0.2/stream"
        );
        assert_eq!(
            expand("pa$$word e $ solto", &with(&[])).unwrap(),
            "pa$$word e $ solto"
        );
    }

    #[test]
    fn a_marker_is_replaced_by_its_value() {
        let got = expand(
            "rtsp://admin:${CAM_PASS}@10.0.0.2/stream",
            &with(&[("CAM_PASS", "abc123")]),
        )
        .unwrap();
        assert_eq!(got, "rtsp://admin:abc123@10.0.0.2/stream");
    }

    #[test]
    fn a_value_with_special_characters_is_encoded_inside_the_userinfo() {
        let got = expand(
            "rtsp://admin:${P}@10.0.0.2/stream",
            &with(&[("P", "p@ss:w/rd#1 %")]),
        )
        .unwrap();
        assert_eq!(
            got,
            "rtsp://admin:p%40ss%3Aw%2Frd%231%20%25@10.0.0.2/stream"
        );
        // e a URL continua tendo UM só @ e a mesma estrutura
        assert_eq!(got.matches('@').count(), 1);
    }

    #[test]
    fn outside_the_userinfo_the_value_is_literal() {
        // um token no caminho (webhook) não deve ser codificado
        let got = expand(
            "https://ntfy.sh/${TOPIC}?x=1",
            &with(&[("TOPIC", "meu/topico")]),
        )
        .unwrap();
        assert_eq!(got, "https://ntfy.sh/meu/topico?x=1");
        // um `@` no caminho não faz o trecho ser tratado como userinfo
        let got = expand("https://h/a@${X}", &with(&[("X", "b@c")])).unwrap();
        assert_eq!(got, "https://h/a@b@c");
    }

    #[test]
    fn a_missing_secret_is_named_once_and_never_valued() {
        let err = expand("rtsp://u:${A}@h/${A}${B}", &with(&[])).unwrap_err();
        assert_eq!(err, vec!["A".to_string(), "B".to_string()]);
        let msg = missing_message("cameras[0].url", &err);
        assert!(
            msg.contains("'A'") && msg.contains("cameras[0].url"),
            "{msg}"
        );
        assert!(
            msg.contains("/run/secrets")
                || msg.contains("RRV_SECRETS_DIR")
                || msg.contains("secrets")
        );
    }

    #[test]
    fn double_dollar_escapes_a_literal_marker() {
        assert_eq!(expand("a$${B}c", &with(&[("B", "x")])).unwrap(), "a${B}c");
    }

    #[test]
    fn invalid_names_and_unclosed_markers_are_left_alone() {
        assert_eq!(
            expand("${} e ${a b} e ${abc", &with(&[])).unwrap(),
            "${} e ${a b} e ${abc"
        );
    }

    #[test]
    fn utf8_text_survives() {
        assert_eq!(
            expand("rtsp://u:${P}@h/Portão", &with(&[("P", "çã")])).unwrap(),
            "rtsp://u:%C3%A7%C3%A3@h/Portão"
        );
    }

    #[test]
    fn lookup_prefers_the_environment_and_reads_files_without_the_trailing_newline() {
        let dir = std::env::temp_dir().join(format!("rrv-secrets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("RRV_TEST_FILE_SECRET"), "do-arquivo\n").unwrap();
        // SAFETY: nenhum outro teste usa estas variáveis.
        unsafe {
            std::env::set_var("RRV_SECRETS_DIR", &dir);
            std::env::set_var("RRV_TEST_ENV_SECRET", "do-ambiente");
        }
        assert_eq!(
            lookup("RRV_TEST_ENV_SECRET").as_deref(),
            Some("do-ambiente")
        );
        assert_eq!(
            lookup("RRV_TEST_FILE_SECRET").as_deref(),
            Some("do-arquivo")
        );
        assert_eq!(lookup("RRV_TEST_NAO_EXISTE"), None);
        // o nome não pode sair do diretório de segredos
        assert_eq!(lookup("../etc/passwd"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
