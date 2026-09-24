# 0001 — Plataformas-alvo: Linux (completo), Windows e macOS (melhor esforço)
**Status:** Aceita (refinada em 2026-09-24)

## Contexto
O projeto hoje só foi usado/testado em Linux (Ubuntu/Arch). Resposta do dono: as três plataformas.

## Decisão
Suportar as três. Nada em `domain/` pode depender de SO. Código específico de SO fica em
`infrastructure/` atrás de `cfg(target_os)` ou de uma trait.

## Consequências
- Caminhos: usar `dirs`/XDG no Linux, `%APPDATA%` no Windows, `~/Library` no macOS (hoje `view_state.rs` usa `~/.local/state`).
- Espaço em disco: `sysinfo` (multiplataforma) em vez de `rustix::statvfs`.
- Decoders/encoders por plataforma (ver ADR 0002): a seleção deve ser dirigida por sondagem do registry do GStreamer, não por `cfg`.
- CI precisa de matriz com as três plataformas (GStreamer instalado em cada uma).
- Teclas/atalhos, fullscreen (`F11`) e Wayland/X11 precisam de teste manual por plataforma.
- Custo alto de testes: começar com CI de build+testes unitários; testes de pipeline real só onde o GStreamer estiver instalado.

## Revisão (2026-09-24)
- Fato verificado no código: `view_state.rs` e `recording_paths.rs` dependem da variável `HOME`, `XDG_STATE_HOME` e
  `XDG_VIDEOS_DIR` (`domain/recording.rs`). No Windows `HOME` normalmente não existe → **quebra**. Trocar por uma
  abstração de diretórios (crate `dirs`/`directories`, a avaliar) em `infrastructure/`.
- `domain/recording.rs` lê variável de ambiente: viola "domain puro, sem I/O" (AGENTS.md). Mover a resolução de caminho para `infrastructure/`.
- Definir o **nível de suporte** por SO (ex.: Linux = completo; Windows/macOS = "melhor esforço" no início). "Três SOs" sem isso
  multiplica testes e atrasa M1.
- GStreamer: distribuição e plugins mudam por SO (Windows: instalador MSVC; macOS: framework/Homebrew). Definir como será empacotado.

## Decisão do dono (2026-09-24): nível de suporte
- **Linux:** suporte completo e plataforma de referência (todos os testes, empacotamento, GPU NVIDIA).
- **Windows e macOS:** *melhor esforço* — disponibilizar o **mínimo possível**: build compila e o app abre/exibe câmeras.
  Sem promessa de paridade (ML por GPU, gravação por evento otimizada, empacotamento polido).
- Regras práticas:
  - CI: Linux roda build + clippy + testes completos; Windows/macOS rodam **só build** (e testes unitários de `domain/`), com falha permitida no início.
  - Nenhuma feature nova pode *quebrar o build* nos outros SOs, mas pode chegar lá depois ou nunca (marcar como "somente Linux" na doc).
  - Código específico de SO atrás de `cfg(target_os)`; o corrige-caminhos (`HOME`/XDG) deve ao menos não dar panic no Windows.
  - Empacotamento mínimo: binário/zip com instruções; sem MSI/dmg por enquanto.
- Isso reduz o esforço do M0/M5 e tira Windows/macOS do caminho crítico de M1–M4.
