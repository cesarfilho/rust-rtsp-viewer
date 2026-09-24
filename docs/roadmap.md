# Roadmap

Baseado em: `docs/gap_analysis.md`, `docs/status.md`, `docs/libraries.md`, `docs/adr/`.
Decisões do dono (2026-09-24): Linux completo, Windows/macOS melhor esforço (mínimo) · 16 câmeras + GPU NVIDIA · ML é meta principal · app desktop pessoal · licença livre.
Estimativas **não** foram feitas: exigem spikes (M0) e o baseline. Cada marco tem critério de saída.

## M0 — Fundação e medição (nada de feature)
- [ ] CI (fmt, `clippy -D warnings`, test) em **Linux**; Windows/macOS só build, com falha permitida (ADR 0001); `deny.toml`; `rust-toolchain.toml`; LICENSE (licença livre — ver ADR 0009).
- [ ] Rodar `scripts/check_rtsp_sessions.sh` em cada modelo de câmera (limite de sessões, ADR 0008).
- [ ] Adicionar `LICENSE` (AGPL-3.0) e `license` no `Cargo.toml` (ADR 0009).
- [ ] Rodar `cargo build && cargo test` hoje e registrar o resultado (o AGENTS.md diz 310 testes; não conferido).
- [ ] Spike: `gstreamer` 0.20→0.25 e `iced` 0.13→0.14 em branch (ver `docs/libraries.md`).
- [ ] Baseline com `scripts/baseline.sh` (1, 4, 16 câmeras): CPU, RSS, fps, banda.
- [ ] Spikes: `ort` (ms/inferência 320x320 em NVIDIA), gravação direta sem transcode (ADR 0007), decoders por hardware (ADR 0002).
- **Saída:** build verde no Linux (Windows/macOS: compila, melhor esforço), números de baseline, decisões dos ADRs "Proposta" promovidas ou rejeitadas.

## M1 — Movimento + zonas (spec: `docs/specs/motion-zones.md`)
Detect stream (ADR 0005), `[motion]`, `[[cameras.zones]]`, editor de zonas, evento Motion, indicador na UI.
- **Saída:** critérios de aceite da spec; sem `allow(dead_code)` em motion/zones.

## M2 — Escala de câmeras
`multi_stream` (sub no grid, main no spotlight), `streaming` (pausa em cena estática), decoder por hardware, encoder por hardware (`hw_encoder`).
- **Saída:** 16 câmeras dentro do orçamento de CPU/GPU definido no baseline; diagnóstico mostra decoder em uso.

## M3 — Gravação inteligente e histórico
Persistência SQLite (ADR 0006), gravação por evento, pré-captura (ADR 0007), retenção por modo, limpeza por espaço, timeline persistente, playback/seek, export de clipe.
- **Saída:** gravar por evento e reproduzir pelo histórico; disco nunca enche além do limite configurado.

## M4 — Detecção de objetos (ADR 0003)
`ort` atrás de `feature = "detect"`, thread de inferência, recorte da região de movimento, eventos por label/zona/score, review items (Alerts × Detections).
- **Saída:** precisão/recall em conjunto de teste definido; latência de inferência dentro do orçamento; degrada sem GPU.

## M5 — Acabamento
Credenciais no keyring, validação de config com erro por campo, i18n, acessibilidade, empacotamento (Linux: Flatpak/AppImage; Windows/macOS: só zip/binário), PTZ (`oxvif`), áudio bidirecional, timelapse.

## Dependências
M0 → M1 → M2 → M3 → M4. M2 pode começar em paralelo a M1 no que não usa movimento (sub-stream, decoder HW). M5 é independente após M0.

## Riscos
| Risco | Mitigação |
|---|---|
| Upgrade `gstreamer`/`iced` maior que o esperado | spike em M0 com limite de tempo; branch isolada |
| `ort` é release candidate; providers diferem por SO | fixar versão; fallback `tract-onnx`/CPU |
| Pré-captura sem transcode pode não servir para HLS/arquivo | tratar por tipo de fonte; documentar limitação |
| Três SOs multiplicam testes de GStreamer | Linux completo; Windows/macOS só build (ADR 0001) |
| Escopo (ML + 3 SOs + 16 câmeras) | respeitar a ordem; não iniciar M4 antes do baseline e de M2 |
