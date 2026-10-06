# Estado do projeto

Revisado em 2026-10-06 (v0.8.0 + o que veio depois; a v0.9.0 é a tarefa A7 do `plano-restante.md`). Medido no código:
~33 mil linhas de Rust, **~580 testes** (451 do núcleo, 102 da janela, mais os de integração), `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo fmt --check` e `cargo deny check` limpos, **zero `allow(dead_code)`**.

## Como é montado (ADR 0010)
| Crate | O que é |
|---|---|
| `crates/rrv-core` | O motor, **sem `iced`** (o compilador e um teste garantem): `domain/` (lógica pura), `infrastructure/` (GStreamer, SQLite, disco, notificação), `engine/` (câmeras, pipelines, gravação, reprodução, clipes), `ipc/` (canal de controle), `config`, `secrets`, `webhook` |
| `crates/rrv-daemon` | `rrv-daemon` (roda sem janela, no Docker) e `rrvctl` (linha de comando) |
| raiz `rust-rtsp-viewer` | A janela (`src/ui/`), que com um daemon só **mostra** e comanda |

## O que está feito
- **Visualização:** grade/flex/spotlight, sub/main stream, 16 câmeras (65% de um núcleo; 39% com a iGPU Intel), temas com teste de contraste.
- **Movimento e zonas:** detecção por diferença de luma em ramo reduzido, zonas com editor, filtro de mudança global.
- **Gravação:** RTSP H.264/H.265 **sem reencode**, por movimento com **pré-roll** e pós-roll, áudio opcional, segmentos com EOS correto;
  HLS/arquivo/MJPEG reencodam (decisão, ADR 0007).
- **Histórico (SQLite):** segmentos e eventos, retenção por idade e por disco, aviso de disco cheio, proteção de trechos.
- **Reprodução:** vista Gravações com linha do tempo, player (seek, velocidade, quadro a quadro, arrastar), vários canais
  sincronizados, lista de eventos, exportar clipe sem reencode, atualização automática.
- **Daemon e Docker:** grava e detecta com a janela fechada, canal de controle versionado, segredos (`${NOME}`), webhook,
  decodificação por iGPU Intel (VA-API); NVIDIA preparada (CDI), não testada.

## O que falta
Tudo o que está aberto está em **`docs/plano-restante.md`**: fechar o M3 (fase A), iced 0.14 e NV12 na janela (B), IA (C) e
acabamento para a 1.0 (D). O resumo do que não depende de nós: a janela ainda converte cada quadro para RGBA (2.3).

## Código "unsafe" e `unwrap`
Cinco `unsafe`, todos pequenos e justificados: `libc::statvfs` (`infrastructure/disk.rs`) e `std::env::set_var` em testes
(Rust 2024). Fora de testes não há `unwrap()`/`expect()` nos caminhos de câmera. `clap` só lê o caminho do config.

## Módulos removidos
`streaming`, `timelapse`, `bidirectional_audio`, `hw_encoder` e `ptz` não tinham chamador e foram apagados; voltam do histórico
do git se algum dia fizerem sentido.
