//! Idiomas da interface (plano D4): português do Brasil (o texto-fonte) e inglês.
//!
//! O texto-fonte é o português, escrito no próprio código dentro de [`t`] / [`tf`]; o catálogo
//! ([`EN`]) diz como cada um fica em inglês. Quem chama nunca escolhe o idioma: `t("Gravar")` devolve
//! `"Record"` com o inglês ativo e `"Gravar"` no português. Uma frase sem tradução cai para o português
//! (nunca fica em branco), e o teste `tests/i18n_scan.rs` falha se um `t("…")` não está no catálogo ou
//! se sobrou texto fixo na interface.
//!
//! Trocar o idioma vale já no próximo quadro: a janela monta a interface inteira a cada atualização.
//! O que já foi escrito (um aviso na tela) fica no idioma em que nasceu.

mod en;

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

pub use en::EN;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    Pt,
    En,
}

impl Lang {
    /// `"pt-BR"`, `"pt"`, `"en"`, `"en-US"`… (sem diferenciar maiúsculas); `None` se não conhece.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        let base = s.split(['-', '_', '.']).next().unwrap_or("");
        match base {
            "pt" => Some(Self::Pt),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    /// O código gravado no `view.toml`.
    pub fn code(self) -> &'static str {
        match self {
            Self::Pt => "pt-BR",
            Self::En => "en",
        }
    }

    /// O nome do idioma, no próprio idioma (para o seletor).
    pub fn native_name(self) -> &'static str {
        match self {
            Self::Pt => "Português",
            Self::En => "English",
        }
    }

    pub fn all() -> [Self; 2] {
        [Self::Pt, Self::En]
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn get() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == Lang::En as u8 {
        Lang::En
    } else {
        Lang::Pt
    }
}

/// O idioma do ambiente (`LC_ALL`, `LC_MESSAGES`, `LANG`) quando nada foi escolhido: inglês se for
/// `en*`, senão português (o idioma do projeto).
pub fn from_environment() -> Lang {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.is_empty() && v != "C" && v != "POSIX")
        .and_then(|v| Lang::parse(&v))
        .unwrap_or_default()
}

fn catalog() -> &'static HashMap<&'static str, &'static str> {
    static MAP: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| EN.iter().copied().collect())
}

/// A tradução em inglês de `pt`, se houver.
pub fn english(pt: &str) -> Option<&'static str> {
    catalog().get(pt).copied()
}

/// O texto no idioma ativo.
pub fn t(pt: &'static str) -> &'static str {
    match get() {
        Lang::Pt => pt,
        Lang::En => english(pt).unwrap_or(pt),
    }
}

/// Como [`t`], para frases com valores: cada `{}` é trocado, em ordem, pelos `args`. O `{}` aparece no
/// texto-fonte e na tradução, que pode reordenar as palavras mas não os valores.
pub fn tf(pt: &'static str, args: &[&dyn Display]) -> String {
    fill(t(pt), args)
}

/// Troca os `{}` de `template` pelos `args`, em ordem. Sobra de `{}` fica como está; sobra de
/// argumentos é ignorada.
pub fn fill(template: &str, args: &[&dyn Display]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut it = args.iter();
    let mut rest = template;
    while let Some(i) = rest.find("{}") {
        out.push_str(&rest[..i]);
        match it.next() {
            Some(a) => out.push_str(&a.to_string()),
            None => out.push_str("{}"),
        }
        rest = &rest[i + 2..];
    }
    out.push_str(rest);
    out
}

/// "1 câmera" / "2 câmeras" (e "1 camera" / "2 cameras"): o plural de uma palavra em cada idioma.
pub fn plural(n: usize, pt_one: &'static str, pt_many: &'static str) -> &'static str {
    t(if n == 1 { pt_one } else { pt_many })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Nada aqui muda o idioma global: os testes que o trocam ficam em `tests/i18n.rs`, num binário
    // próprio, para não atrapalhar os outros testes da biblioteca (que esperam o português).

    #[test]
    fn language_codes_parse_in_every_usual_form() {
        for s in ["pt", "pt-BR", "pt_BR.UTF-8", "PT-br"] {
            assert_eq!(Lang::parse(s), Some(Lang::Pt), "{s}");
        }
        for s in ["en", "en-US", "en_GB.UTF-8", " EN "] {
            assert_eq!(Lang::parse(s), Some(Lang::En), "{s}");
        }
        assert_eq!(Lang::parse("fr"), None);
        assert_eq!(Lang::parse(""), None);
        for l in Lang::all() {
            assert_eq!(Lang::parse(l.code()), Some(l));
        }
    }

    #[test]
    fn placeholders_are_filled_in_order() {
        assert_eq!(fill("{} de {}", &[&1, &"a"]), "1 de a");
        assert_eq!(fill("sem valores", &[&1]), "sem valores");
        assert_eq!(fill("{} e {}", &[&1]), "1 e {}");
    }

    #[test]
    fn the_catalog_has_no_duplicate_or_empty_entries_and_keeps_its_placeholders() {
        let mut seen = std::collections::HashSet::new();
        for (pt, en) in EN {
            assert!(!pt.is_empty() && !en.is_empty(), "entrada vazia: {pt:?}");
            assert!(seen.insert(*pt), "chave repetida: {pt}");
            assert_eq!(
                pt.matches("{}").count(),
                en.matches("{}").count(),
                "a tradução tem de manter os {{}} de \"{pt}\""
            );
        }
    }
}
