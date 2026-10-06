//! A troca de idioma em execução. Fica num binário só dele: o idioma é global ao processo.

use rrv_core::i18n::{self, Lang, plural, t, tf};

#[test]
fn the_active_language_decides_what_t_returns() {
    i18n::set(Lang::Pt);
    assert_eq!(t("Gravar"), "Gravar");
    assert_eq!(tf("Câmera · {}", &[&"Portão"]), "Câmera · Portão");
    assert_eq!(plural(1, "câmera", "câmeras"), "câmera");
    assert_eq!(plural(3, "câmera", "câmeras"), "câmeras");

    i18n::set(Lang::En);
    assert_eq!(i18n::get(), Lang::En);
    assert_eq!(t("Gravar"), "Record");
    assert_eq!(tf("Câmera · {}", &[&"Portão"]), "Camera · Portão");
    assert_eq!(plural(1, "câmera", "câmeras"), "camera");
    assert_eq!(plural(2, "câmera", "câmeras"), "cameras");
    // sem tradução cai para o português, nunca em branco
    assert_eq!(
        t("uma frase que ninguém traduziu"),
        "uma frase que ninguém traduziu"
    );

    i18n::set(Lang::Pt);
    assert_eq!(i18n::get(), Lang::Pt);
}
