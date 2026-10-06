//! O que garante a tradução (plano D4): varre o código da interface.
//!
//! 1. Toda frase dentro de `t("…")`, `tf("…", …)` ou `plural(n, "…", "…")` existe no catálogo inglês.
//! 2. Em todo arquivo de `src/ui`, não sobra texto em português fora dessas chamadas:
//!    uma frase com acento ou com palavra de interface conhecida, entre aspas, em código que não seja de
//!    log, de teste ou marcado com `// i18n-ok`, falha o teste.
//!

use std::path::{Path, PathBuf};

use rust_rtsp_viewer::i18n;

/// Linhas que são de log, depuração ou erro de programação: ficam em português para quem opera.
const SKIP_LINE: &[&str] = &[
    "log::",
    "println!",
    "eprintln!",
    "assert",
    "unreachable!",
    "panic!",
    ".expect(",
    "i18n-ok",
];

/// Palavras de interface em português, para pegar texto sem acento ("Gravar", "Selecione uma câmera").
const WORDS: &[&str] = &[
    "Gravar",
    "Parar",
    "Sem ",
    "Todas",
    "Nenhum",
    "Selecione",
    "Ligar",
    "Desligar",
    "Clique",
    "Carregando",
    "Fechar",
    "Abrir",
    "Filtrar",
    "Ampliar",
    "Ouvir",
    "Câmera",
    "Camera",
    "Tente",
    "Falha",
    "Não ",
];

fn ui_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui")
}

/// Texto do núcleo que a janela mostra (estados, diagnóstico, avisos): também passa pelo catálogo.
const CORE_FILES: &[&str] = &[
    "crates/rrv-core/src/domain/camera_status.rs",
    "crates/rrv-core/src/domain/diagnostics.rs",
    "crates/rrv-core/src/domain/notify.rs",
    "crates/rrv-core/src/engine/backoff.rs",
];

fn scanned_files() -> Vec<String> {
    let mut v = all_ui_files();
    v.extend(CORE_FILES.iter().map(|f| (*f).to_string()));
    v
}

fn source(rel: &str) -> String {
    let path = if rel.starts_with("crates/") {
        Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
    } else {
        ui_dir().join(rel)
    };
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{rel}: {e}"));
    // o módulo de testes não é interface
    match text.find("#[cfg(test)]") {
        Some(i) => text[..i].to_string(),
        None => text,
    }
}

/// Os literais de string de uma linha: `(posição do início, conteúdo sem escapes processados)`.
fn literals(line: &str) -> Vec<(usize, String)> {
    let b = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => break, // comentário até o fim da linha
            b'\'' => {
                // caractere: '"' não abre string
                i += if b.get(i + 1) == Some(&b'\\') { 4 } else { 3 };
            }
            b'"' => {
                let start = i;
                i += 1;
                let mut s = String::new();
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        s.push('\\');
                        i += 1;
                    }
                    let ch_len = line[i..].chars().next().map_or(1, char::len_utf8);
                    s.push_str(&line[i..i + ch_len]);
                    i += ch_len;
                }
                out.push((start, s));
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

/// O texto como o programa o vê: `\u{2026}`, `\"` e `\\` já viram o caractere.
fn unescape(lit: &str) -> String {
    let mut out = String::new();
    let mut chars = lit.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('u') if chars.peek() == Some(&'{') => {
                chars.next();
                let hex: String = chars.by_ref().take_while(|c| *c != '}').collect();
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// `before` termina na chamada `name(` (e não numa função de nome maior, como `text(`)?
fn ends_with_call(before: &str, name: &str) -> bool {
    before.strip_suffix(name).is_some_and(|rest| {
        !rest
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || c == '_')
    })
}

/// Esse literal vem logo depois de `t(`, `tf(` ou de um argumento de `plural(`?
fn is_translated(prev: &str, line: &str, pos: usize) -> bool {
    // rustfmt pode pôr o literal sozinho na linha de baixo: a chamada termina a linha de cima
    let before = line[..pos].trim_end();
    let before = if before.is_empty() {
        prev.trim_end()
    } else {
        before
    };
    ends_with_call(before, "t(")
        || ends_with_call(before, "tf(")
        || ends_with_call(before, "plural(")
        || (before.ends_with(',') && line[..pos].contains("plural("))
}

/// Parece texto para a pessoa ler: começa com maiúscula e tem 4+ letras ("Concluir", "Atalhos de
/// teclado", "DESATIVADA"); nomes de código (`a_b`, `a::b`) e modelos (`{}…`) não contam.
fn looks_like_ui_text(lit: &str) -> bool {
    // "Ctrl+Q", "Space / k", "Enter / Backspace", "F11": nomes de tecla, iguais nos dois idiomas
    const KEYS: &[&str] = &[
        "Ctrl",
        "Shift",
        "Alt",
        "Space",
        "Enter",
        "Backspace",
        "Tab",
        "Esc",
        "PgUp",
        "PgDn",
    ];
    let key_name = |w: &str| {
        w.chars().count() == 1
            || KEYS.contains(&w)
            || (w.starts_with('F') && w[1..].chars().all(|c| c.is_ascii_digit()))
    };
    if lit
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .all(key_name)
    {
        return false;
    }
    let letters = lit.chars().filter(|c| c.is_alphabetic()).count();
    if letters < 4 || lit.contains("::") || lit.contains('_') || lit.starts_with('{') {
        return false;
    }
    lit.chars().next().is_some_and(char::is_uppercase)
}

fn looks_portuguese(lit: &str) -> bool {
    lit.chars().any(|c| "áéíóúâêôãõçÁÉÍÓÚÂÊÔÃÕÇ".contains(c))
        || WORDS.iter().any(|w| lit.contains(w))
        || looks_like_ui_text(lit)
}

/// Todos os `(arquivo, linha, literal)` de português solto num arquivo.
fn untranslated(rel: &str) -> Vec<String> {
    let mut found = Vec::new();
    let text = source(rel);
    let lines: Vec<&str> = text.lines().collect();
    for (n, line) in lines.iter().enumerate() {
        let prev = if n > 0 { lines[n - 1] } else { "" };
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || SKIP_LINE.iter().any(|m| line.contains(m)) {
            continue;
        }
        for (pos, lit) in literals(line) {
            if looks_portuguese(&lit) && !is_translated(prev, line, pos) {
                found.push(format!("{rel}:{}: \"{lit}\"", n + 1));
            }
        }
    }
    found
}

fn translated_keys(rel: &str) -> Vec<(usize, String)> {
    let mut keys = Vec::new();
    let text = source(rel);
    let lines: Vec<&str> = text.lines().collect();
    for (n, line) in lines.iter().enumerate() {
        let prev = if n > 0 { lines[n - 1] } else { "" };
        if line.trim_start().starts_with("//") {
            continue;
        }
        for (pos, lit) in literals(line) {
            if is_translated(prev, line, pos) {
                keys.push((n + 1, unescape(&lit)));
            }
        }
    }
    keys
}

#[test]
fn every_translated_phrase_is_in_the_english_catalog() {
    let mut missing = Vec::new();
    for rel in &scanned_files() {
        for (n, key) in translated_keys(rel) {
            // o código escreve `\u{…}` e `\"`; o catálogo, o caractere
            if i18n::english(&key).is_none() {
                missing.push(format!("{rel}:{n}: \"{key}\""));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "frases sem tradução em i18n/en.rs:\n{}",
        missing.join("\n")
    );
}

#[test]
fn converted_files_have_no_loose_portuguese() {
    let loose: Vec<String> = all_ui_files()
        .iter()
        .flat_map(|r| untranslated(r))
        .collect();
    assert!(
        loose.is_empty(),
        "texto fixo fora de t()/tf() (ou marque a linha com // i18n-ok):\n{}",
        loose.join("\n")
    );
}

#[test]
fn the_catalog_has_no_dead_entries_for_converted_files() {
    // uma entrada que nenhum arquivo usa é lixo (ou um erro de digitação na chave)
    let mut used = std::collections::HashSet::new();
    for rel in scanned_files() {
        for (_, key) in translated_keys(&rel) {
            used.insert(key);
        }
    }
    // frases usadas fora de `src/ui` (núcleo e seus testes) também contam
    for dir in [
        "crates/rrv-core/src",
        "crates/rrv-core/tests",
        "crates/rrv-daemon/src",
    ] {
        for f in walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join(dir)) {
            if let Ok(text) = std::fs::read_to_string(&f) {
                let lines: Vec<&str> = text.lines().collect();
                for (n, line) in lines.iter().enumerate() {
                    let prev = if n > 0 { lines[n - 1] } else { "" };
                    for (pos, lit) in literals(line) {
                        if is_translated(prev, line, pos) {
                            used.insert(unescape(&lit));
                        }
                    }
                }
            }
        }
    }
    let dead: Vec<&str> = i18n::EN
        .iter()
        .map(|(pt, _)| *pt)
        .filter(|pt| !used.contains(*pt))
        .collect();
    assert!(
        dead.is_empty(),
        "entradas do catálogo que ninguém usa: {dead:?}"
    );
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    out
}

fn all_ui_files() -> Vec<String> {
    let base = ui_dir();
    let mut v: Vec<String> = walk(&base)
        .into_iter()
        .map(|p| {
            p.strip_prefix(&base)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    v.sort();
    v
}
