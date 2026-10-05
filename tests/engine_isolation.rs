//! Guarda da separação motor × interface (ADR 0010, tarefa 2.5.1).
//!
//! O motor de vídeo (`domain/`, `infrastructure/`, `engine/`) tem de poder ir
//! para um crate/daemon sem `iced`. Este teste
//! lê o código-fonte e falha se alguém reintroduzir o iced ali. Comentários são
//! ignorados.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Linhas de código (sem comentários) que citam `iced` como identificador.
fn iced_uses(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .enumerate()
        .filter(|(_, l)| {
            let t = l.trim_start();
            !t.starts_with("//") && (t.contains("iced::") || t.contains("use iced"))
        })
        .map(|(n, l)| format!("{}:{}: {}", path.display(), n + 1, l.trim()))
        .collect()
}

#[test]
fn the_video_engine_does_not_depend_on_iced() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root.join("domain"), &mut files);
    rust_files(&root.join("infrastructure"), &mut files);
    rust_files(&root.join("engine"), &mut files);

    let offenders: Vec<String> = files.iter().flat_map(|f| iced_uses(f)).collect();
    assert!(
        offenders.is_empty(),
        "o motor voltou a depender do iced:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn the_video_engine_does_not_import_ui_modules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    rust_files(&root.join("engine"), &mut files);
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap();
        for (n, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            assert!(
                !t.contains("crate::ui"),
                "{}:{}: o motor importa a interface: {t}",
                file.display(),
                n + 1
            );
        }
    }
}
