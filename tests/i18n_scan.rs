//! O que garante a tradução (plano D4): varre o código da interface.
//!
//! 1. Toda frase dentro de `t("…")`, `tf("…", …)` ou `plural(n, "…", "…")` existe no catálogo inglês.
//! 2. Nos arquivos já convertidos (`CONVERTED`), não sobra texto em português fora dessas chamadas:
//!    uma frase com acento ou com palavra de interface conhecida, entre aspas, em código que não seja de
//!    log, de teste ou marcado com `// i18n-ok`, falha o teste.
//!
//! A lista cresce até cobrir toda a pasta `src/ui`; o teste final (`every_ui_file_is_converted`) a
//! compara com o que existe em disco.

use std::path::{Path, PathBuf};

use rust_rtsp_viewer::i18n;

/// Arquivos de `src/ui` cujos textos já passam pelo catálogo.
const CONVERTED: &[&str] = &["view/menu.rs"];

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

fn source(rel: &str) -> String {
    let text = std::fs::read_to_string(ui_dir().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
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
fn is_translated(line: &str, pos: usize) -> bool {
    let before = line[..pos].trim_end();
    ends_with_call(before, "t(")
        || ends_with_call(before, "tf(")
        || ends_with_call(before, "plural(")
        || (before.ends_with(',') && line[..pos].contains("plural("))
}

fn looks_portuguese(lit: &str) -> bool {
    lit.chars().any(|c| "áéíóúâêôãõçÁÉÍÓÚÂÊÔÃÕÇ".contains(c))
        || WORDS.iter().any(|w| lit.contains(w))
}

/// Todos os `(arquivo, linha, literal)` de português solto num arquivo.
fn untranslated(rel: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (n, line) in source(rel).lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || SKIP_LINE.iter().any(|m| line.contains(m)) {
            continue;
        }
        for (pos, lit) in literals(line) {
            if looks_portuguese(&lit) && !is_translated(line, pos) {
                found.push(format!("{rel}:{}: \"{lit}\"", n + 1));
            }
        }
    }
    found
}

fn translated_keys(rel: &str) -> Vec<(usize, String)> {
    let mut keys = Vec::new();
    for (n, line) in source(rel).lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for (pos, lit) in literals(line) {
            if is_translated(line, pos) {
                keys.push((n + 1, lit));
            }
        }
    }
    keys
}

#[test]
fn every_translated_phrase_is_in_the_english_catalog() {
    let mut missing = Vec::new();
    for rel in CONVERTED {
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
    let loose: Vec<String> = CONVERTED.iter().flat_map(|r| untranslated(r)).collect();
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
    for rel in all_ui_files() {
        for (_, key) in translated_keys(&rel) {
            used.insert(key);
        }
    }
    // frases usadas fora de `src/ui` (núcleo) também contam: procura nelas
    let core = Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/rrv-core/src");
    for f in walk(&core) {
        if let Ok(text) = std::fs::read_to_string(&f) {
            for line in text.lines() {
                for (pos, lit) in literals(line) {
                    if is_translated(line, pos) {
                        used.insert(lit);
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

/// Quando a conversão termina, `CONVERTED` cobre tudo (menos o que é de fato sem texto).
#[test]
fn progress_report() {
    let all = all_ui_files();
    let todo: Vec<&String> = all
        .iter()
        .filter(|f| !CONVERTED.contains(&f.as_str()))
        .filter(|f| !untranslated(f).is_empty())
        .collect();
    eprintln!("arquivos de src/ui com texto ainda por converter: {todo:?}");
}
