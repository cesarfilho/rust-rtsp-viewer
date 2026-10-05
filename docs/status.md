# Status dos módulos de domínio

Revisado em 2026-10-05 (v0.8.0) contra o código: referências `domain::<mod>` por `grep`,
contagem de `#[test]`, `cargo clippy --all-targets -- -D warnings` limpo e `cargo test`
com 387 testes passando.

"Ligado" = usado fora de `domain/mod.rs` e do próprio arquivo. Em "Testes", `—` = não
contado nesta revisão (só os módulos que mudaram de situação foram contados).

## Ligados

| Módulo | Testes | Onde é usado |
|---|---|---|
| `audio` | — | `infrastructure/audio.rs`, `ui/update.rs` (mute/volume, VU) |
| `codec` | — | `ui/pipeline.rs` (rótulo do codec no Inspector) |
| `diagnostics` | — | Inspector e hints (Night, Tamper, perda de pacotes) |
| `groups` | — | `[[groups]]`, chips de filtro na sidebar e na grade |
| `metrics` | — | `Metrics`, `StreamInfo` (agora com decoder), bitrate comprimido |
| `motion` | 15 | `update::detect_camera_motion` (~2 Hz), filtrado por zonas |
| `multi_stream` | 13 | **`sub_url`**: grade no sub, spotlight/flex/gravação no principal |
| `notify` | — | `notify-send` com cooldown por câmera/tipo |
| `recording` | — | gravação manual e por movimento (`on_motion`, pós-roll) |
| `redact` | — | `mask_credentials` em todo log com URL |
| `snapshot` | — | snapshot e burst |
| `timeline` | — | linha do tempo de eventos (**só em memória**) |
| `view` | — | densidade, paginação, carrossel, ordem |
| `zones` | 13 | editor visual (`ui/zone_editor.rs`) e `zones.toml` por nome de câmera |

## Ainda não ligados (lógica pura testada, sem chamador)

| Módulo | Testes | O que existe | O que falta |
|---|---|---|---|
| `streaming.rs` | 7 | máquina de estados `evaluate_streaming` (pausa em cena estática, warmup, retomada por movimento) | `motion` já está ligado; falta chamar a avaliação no tick e ligar ao `bridge.stop()`/start, com snapshot estático enquanto pausado |
| `ptz.rs` | 6 | `PtzCommand`, `PtzConfig`, presets | cliente ONVIF (não existe), UI de controle, atalhos |
| `timelapse.rs` | 7 | velocidade, formato, nome de arquivo, estimativa | pipeline que gera o timelapse |
| `hw_encoder.rs` | 9 | `HwEncoderBackend`, `detect_backend`, `build_encoder_string` | `start_recording` usa `x264enc` fixo; verificar se `detect_backend` sonda o GStreamer de fato |
| `bidirectional_audio.rs` | 6 | config, `AudioEncoding`, `MicState` | captura do microfone e backchannel RTSP |

Todos têm testes unitários, então o risco está na integração (threads, mutexes,
pipeline), não no cálculo.

## `allow(dead_code)` restantes (10)

`domain/audio.rs`, `domain/codec.rs`, `domain/recording.rs`, `domain/snapshot.rs`,
`infrastructure/audio.rs` (todos com `#![allow(dead_code)]` no arquivo inteiro, ou seja,
parte da API dentro deles também não é usada), mais `ui/bridge.rs` (2), `ui/pipeline.rs`,
`ui/view/style.rs` e `domain/diagnostics.rs` (1 cada).

## Observações
- Fora de testes não há `unwrap()`/`expect()` nem `unsafe`; há 1 `TODO` no código.
- `clap` continua sendo usado só para o caminho do config (argumento posicional).
- Ordem de ligação sugerida para o que falta: `streaming` → `hw_encoder` (junto com a
  decisão de GPU, ver `docs/roadmap.md`) → `ptz` (depende de ONVIF) → `timelapse` →
  `bidirectional_audio`.
