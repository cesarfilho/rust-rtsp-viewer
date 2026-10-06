//! O catálogo português → inglês. A chave é o texto exatamente como está no código, dentro de `t("…")`.
//! Mantenha em ordem de arquivo/área; o teste `tests/i18n_scan.rs` exige que toda chave usada exista aqui.

pub const EN: &[(&str, &str)] = &[
    // menu (view/menu.rs)
    ("Grade", "Grid"),
    ("Exibição", "Display"),
    ("Sair do modo imersivo", "Exit immersive mode"),
    ("Modo imersivo", "Immersive mode"),
    ("Tela cheia", "Full screen"),
    ("Câmera · {}", "Camera · {}"),
    ("Spotlight", "Spotlight"),
    ("Snapshot", "Snapshot"),
    ("Parar gravação", "Stop recording"),
    ("Gravar", "Record"),
    ("Áudio", "Audio"),
    ("Zonas de movimento", "Motion zones"),
    ("Desativar", "Disable"),
    ("Ativar", "Enable"),
    ("Aparência", "Appearance"),
    ("Gravações", "Recordings"),
    ("Ajuda", "Help"),
    ("Sair", "Quit"),
    // plurais usados nos testes e em avisos
    ("câmera", "camera"),
    ("câmeras", "cameras"),
];
