//! Guarda da separação motor × interface (ADR 0010).
//!
//! O compilador já impede `use iced` aqui (o crate não tem a dependência). O que
//! ele não impede é alguém *acrescentar* uma dependência de interface ao
//! `Cargo.toml` deste crate, que é o que o daemon sem janela executa.

const FORBIDDEN: &[&str] = &[
    "iced",
    "winit",
    "wgpu",
    "rust-rtsp-viewer",
    "clap", // argumentos de linha de comando são do binário, não do motor
];

#[test]
fn the_core_crate_has_no_ui_dependencies() {
    let manifest = include_str!("../Cargo.toml");
    let deps: Vec<&str> = manifest
        .lines()
        .skip_while(|l| !l.trim().starts_with("[dependencies]"))
        .skip(1)
        .take_while(|l| !l.trim().starts_with('['))
        .filter(|l| !l.trim().is_empty() && !l.trim().starts_with('#'))
        .collect();
    assert!(!deps.is_empty(), "não li as dependências do Cargo.toml");
    for line in deps {
        let name = line.split('=').next().unwrap().trim();
        assert!(
            !FORBIDDEN.contains(&name),
            "rrv-core não pode depender de `{name}` (interface): {line}"
        );
    }
}
