//! O chaveiro do sistema de verdade (plano D2): guarda, lê pelo caminho que a janela usa, expande na
//! URL e apaga. Precisa de um Secret Service desbloqueado (GNOME Keyring, KWallet...) e do
//! `secret-tool`; só roda com `RRV_TEST_KEYRING=1`, porque mexe no chaveiro da pessoa (um item de
//! teste, apagado no fim).

use rrv_core::secrets::{self, keyring};

#[test]
fn a_secret_goes_through_the_system_keyring_and_into_a_url() {
    if std::env::var_os("RRV_TEST_KEYRING").is_none() {
        eprintln!("sem RRV_TEST_KEYRING=1: teste pulado");
        return;
    }
    let name = format!("rrv_test_{}_password", std::process::id());
    assert!(!keyring::exists(&name), "o item de teste já existia");
    assert_eq!(secrets::lookup(&name), None);

    // uma senha com tudo que quebraria uma URL ou uma linha de comando
    let password = "p@ss:w/rd 'com' \"aspas\" $(echo x) é ç";
    keyring::store(&name, password).unwrap();
    assert_eq!(keyring::lookup(&name).as_deref(), Some(password));
    // o caminho da janela: lookup geral (env → arquivo → chaveiro)
    assert_eq!(secrets::lookup(&name).as_deref(), Some(password));

    let url = format!("rtsp://admin:${{{name}}}@10.0.0.2/s");
    let expanded = secrets::expand(&url, &secrets::lookup).unwrap();
    assert!(
        expanded.starts_with("rtsp://admin:p%40ss%3Aw%2Frd%20"),
        "{expanded}"
    );
    assert!(expanded.ends_with("@10.0.0.2/s"));

    // trocar o valor
    keyring::store(&name, "outra").unwrap();
    assert_eq!(secrets::lookup(&name).as_deref(), Some("outra"));

    // a variável de ambiente e o arquivo têm precedência sobre o chaveiro
    // (aqui só se confere que `RRV_KEYRING=off` o desliga)
    // SAFETY: este binário de teste tem uma única função de teste; ninguém mais lê o ambiente agora.
    unsafe { std::env::set_var("RRV_KEYRING", "off") };
    assert_eq!(secrets::lookup(&name), None, "RRV_KEYRING=off desliga");
    unsafe { std::env::remove_var("RRV_KEYRING") };

    keyring::clear(&name).unwrap();
    assert_eq!(secrets::lookup(&name), None, "apagado");
    keyring::clear(&name).unwrap(); // apagar o que não existe não é erro
}

#[test]
fn bad_names_never_reach_the_keyring_tool() {
    assert_eq!(keyring::lookup("a b"), None);
    assert_eq!(keyring::lookup("$(rm -rf)"), None);
    assert_eq!(keyring::lookup(""), None);
    assert!(keyring::store("a/b", "x").is_err());
    assert!(keyring::store("ok_name", "").is_err(), "senha vazia");
    assert!(keyring::clear("a;b").is_err());
}
