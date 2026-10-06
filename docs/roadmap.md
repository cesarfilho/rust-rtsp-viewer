# Roadmap

Revisado em 2026-10-06. Legenda: `[x]` feito · `[ ]` aberto. O **plano executável do que resta** está em
`docs/plano-restante.md`; o histórico das tarefas está em `docs/plano-de-execucao.md`.

Decisões do dono: **NVR completo**, que **grava e detecta com a janela fechada** (daemon no Docker) · **só Linux**
(ADR 0001) · **sem PTZ** · 16 câmeras · IA com o **YOLO da Ultralytics** (AGPL, compatível com a AGPL-3.0 do projeto) ·
licença livre. Estimativas só existem no plano.

## M0 — Fundação e medição — feito
- [x] CI (fmt, clippy `-D warnings`, testes, `cargo deny`, MSRV 1.92), LICENSE AGPL-3.0, `rust-toolchain.toml`.
- [x] Baselines com câmeras reais e sintéticas: 1/4/16 câmeras, CPU × iGPU (`docs/baseline.md`); limite de sessões da Intelbras.
- [x] GStreamer 0.25 na master. **iced 0.14**: spike pronto, falta o rebase e a validação na tela (plano fase B).

## M1 — Movimento e zonas — feito
- [x] Detecção em ramo reduzido, zonas com editor visual (por nome da câmera), filtro de mudança global, notificação com cooldown,
  gravação por movimento.

## M2 — Escala de câmeras — feito, salvo a janela
- [x] Sub/main stream; decoder visível no Inspector; daemon sem RGBA (CPU −57%); decodificação por iGPU Intel (−47% a mais);
  limite de threads do decodificador; estresse de 1 h sem vazamento.
- [ ] **2.3: caminho NV12 + shader na janela** (a janela ainda converte para RGBA) — fase B.

## M2.5 — O motor sem janela — feito
- [x] Workspace `rrv-core` / `rrv-daemon`; `Engine`; canal de controle por socket Unix; a janela com o daemon (chip, banner,
  confirmações); segredos; webhook; Docker + healthcheck; GPU Intel.
- [ ] 2.5.4 (vídeo do daemon para a janela): adiada, a sessão própria basta.

## M3 — Gravação inteligente e histórico — feito
- [x] Gravação sem reencode, pré-roll, áudio opcional; SQLite; retenção; aviso de disco; vista Gravações (linha do tempo, player,
  vários canais, eventos, proteger, exportar clipe).
- Falta só fechar arestas (fase A do plano): pré-roll curto em câmera de GOP longo, revisão com mouse, release v0.9.

## M4 — Detecção de objetos (ADR 0003) — aberto
- [ ] Spike do `ort` (CPU/OpenVINO/CUDA), módulo `detect`, fila limitada, gatilho por movimento e zona, eventos por rótulo,
  interface, avaliação, ONNX Runtime na imagem. Detalhes: `plano-restante.md`, fase C.

## M5 — Acabamento — aberto
- [ ] ONVIF (só descoberta), chaveiro para as senhas da janela, MQTT/Home Assistant, i18n pt-BR/en, empacotamento Linux
  (AUR, AppImage/Flatpak, `cargo-dist`). Fase D.

## Riscos
| Risco | Mitigação |
|---|---|
| NV12 + shader (2.3) é um widget wgpu dentro do iced | plano B: reduzir a resolução decodificada no grid (já há sub-stream) |
| iced 0.14 clareia as cores na GTX 1650 | `WGPU_POWER_PREF=low` (iGPU); validar na tela antes do merge |
| 4 GB de VRAM com YOLO e NVDEC juntos | iGPU Intel decodifica; a NVIDIA fica para a IA; medir no spike 0.6 |
| `ort` ainda é release candidate | fixar a versão; fallback em CPU |
| GOP esticado (Smart Codec) alonga o pré-roll | teto de 30 s no ring; reduzir o quadro-I na câmera |
