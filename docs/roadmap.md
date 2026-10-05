# Roadmap

Baseado em: `docs/gap_analysis.md`, `docs/status.md`, `docs/libraries.md`, `docs/adr/`.
Revisado em 2026-10-05 (v0.8.0). Legenda: `[x]` feito · `[ ]` aberto.
Decisões do dono (2026-09-24): Linux completo, Windows/macOS melhor esforço (mínimo) · 16 câmeras + GPU NVIDIA · ML é meta principal · app desktop pessoal · licença livre (AGPL-3.0).
Estimativas **não** foram feitas: exigem spikes e baseline. Cada marco tem critério de saída.

## Decisão em aberto: posicionamento
A pesquisa de mercado (ver `docs/gap_analysis.md`) mostra que NVRs com IA (Frigate, UniFi,
Milestone) são maduros e caros de alcançar. Duas direções:
- **Video wall nativo e leve**, com IA opcional ou consumida de fora (ex.: eventos do Frigate via MQTT).
- **NVR completo** com detecção própria (M3 e M4 abaixo).

A meta declarada é ML (ADR 0003), então M4 segue no plano, mas vale confirmar antes de investir nele.

## M0 — Fundação e medição
- [x] CI em Linux (clippy `-D warnings` + testes). Windows/macOS: ainda sem build no CI (ADR 0001).
- [x] LICENSE (AGPL-3.0), `license` e `rust-version` no `Cargo.toml` (ADR 0009; 1.92 desde o GStreamer 0.25).
- [x] `cargo build && cargo test` registrados: 387 testes passando, clippy limpo.
- [ ] `cargo fmt --check` no CI; `deny.toml`; `rust-toolchain.toml`.
- [ ] Rodar `scripts/check_rtsp_sessions.sh` em cada modelo de câmera (limite de sessões, ADR 0008).
- [ ] Spike: `gstreamer` 0.20→0.25 e `iced` 0.13→0.14 em branch (ver `docs/libraries.md`).
- [ ] Baseline com `scripts/baseline.sh` (1, 4, 16 câmeras **reais**): CPU, RSS, fps, banda, VRAM.
  Já existe uma medição de decode em GPU × CPU (`docs/gap_analysis.md`), só com vídeo sintético.
- [ ] Spikes: `ort` (ms/inferência 320x320 na GTX 1650), gravação direta sem transcode (ADR 0007).
- **Saída:** baseline com câmeras reais, decisões dos ADRs "Proposta" promovidas ou rejeitadas.

## M1 — Movimento + zonas (spec: `docs/specs/motion-zones.md`) — entregue
- [x] `[motion]` configurável, detecção por diferença de luma (~2 Hz) filtrada por zonas.
- [x] Editor de zonas visual; zonas por câmera em `zones.toml` (chave = nome da câmera, nunca a URL).
- [x] Evento Motion, notificação de desktop com cooldown, gravação por movimento com pós-roll.
- [ ] Detect stream separado (ADR 0005) e `[[cameras.zones]]` no `config.toml` (hoje as zonas vivem no `zones.toml`).

## M2 — Escala de câmeras
- [x] Sub/main stream por câmera (`sub_url`): grade no sub; spotlight, flex e gravação no principal.
- [x] Diagnóstico mostra o decoder em uso e se roda em CPU ou GPU; bitrate passa a medir o stream comprimido.
- [ ] `streaming` (pausa em cena estática) — o módulo existe, falta ligar (`docs/status.md`).
- [ ] **Decoder por hardware.** Medido: hoje é 100% CPU, e trocar para `nvh264dec` mantendo o caminho RGBA atual
  **dobra** a CPU. Opções, da mais barata à mais cara:
  1. `gst-plugin-va` e decodificar na iGPU Intel (deixa a NVIDIA para IA);
  2. baixar o frame em NV12 e converter em shader (widget wgpu próprio no iced);
  3. zero-copy (neste notebook híbrido exigiria renderizar na NVIDIA).
- [ ] Encoder por hardware (`hw_encoder`) — só relevante enquanto a gravação reencoda (ADR 0007).
- [ ] Áudio usa a URL principal mesmo com a câmera no sub (sessão extra na câmera).
- **Saída:** 16 câmeras dentro do orçamento de CPU/GPU definido no baseline.

## M3 — Gravação inteligente e histórico
- [x] Gravação por evento (`on_motion`) com pós-roll; segmentação por tempo/tamanho; arquivo tocável (EOS).
- [ ] Persistência SQLite (ADR 0006), timeline persistente, playback/seek, export de clipe.
- [ ] Pré-captura (ADR 0007) e gravação sem reencode.
- [ ] Retenção por modo (contínuo × movimento, como no Frigate 0.17) e limpeza por espaço em disco.
- **Saída:** gravar por evento e reproduzir pelo histórico; disco nunca enche além do limite configurado.

## M4 — Detecção de objetos (ADR 0003)
`ort` atrás de `feature = "detect"`, thread de inferência, recorte da região de movimento, eventos por label/zona/score, review items (Alerts × Detections).
- Ambiente: GTX 1650 de 4 GB; ONNX Runtime, CUDA e cuDNN **ainda não instalados**.
- **Saída:** precisão/recall em conjunto de teste definido; latência de inferência dentro do orçamento; degrada sem GPU.

## M5 — Acabamento
- [ ] ONVIF: descoberta de câmeras e PTZ (`oxvif`); Profile T é a base de 2026.
- [ ] Credenciais no keyring; validação de config com erro por campo e `--check`.
- [ ] i18n, acessibilidade, empacotamento (Linux: Flatpak/AppImage; Windows/macOS: só zip/binário).
- [ ] Áudio bidirecional, timelapse.

## Dependências
M0 → M1 → M2 → M3 → M4. M2 pode andar em paralelo a M3 no que não usa movimento. M5 é independente após M0.

## Riscos
| Risco | Mitigação |
|---|---|
| Upgrade `gstreamer`/`iced` maior que o esperado | spike em M0 com limite de tempo; branch isolada |
| Decoder de GPU piora a CPU se o frame voltar em RGBA | medido; só adotar com caminho NV12/shader ou zero-copy |
| 4 GB de VRAM com muitos streams NVDEC | sub-stream na grade; medir VRAM no baseline |
| `ort` é release candidate; providers diferem por SO | fixar versão; fallback `tract-onnx`/CPU |
| Pré-captura sem transcode pode não servir para HLS/arquivo | tratar por tipo de fonte; documentar limitação |
| Três SOs multiplicam testes de GStreamer | Linux completo; Windows/macOS só build (ADR 0001) |
| Escopo (ML + 3 SOs + 16 câmeras) | respeitar a ordem; não iniciar M4 antes do baseline real e de M2 |
