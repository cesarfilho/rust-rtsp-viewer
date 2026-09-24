# Status dos módulos de domínio

Levantado em 2026-09-24 por grep de referências (`domain::<mod>`) e contagem de `#[test]`.
Não foi rodado `cargo build`/`cargo test` para gerar esta tabela.

"Ligado" = usado fora de `domain/mod.rs` e do próprio arquivo.

| Módulo | Linhas | Testes | Ligado? | O que existe | O que falta |
|---|---|---|---|---|---|
| `motion.rs` | 382 | 14 | **Não** | `MotionConfig`, `detect_motion(prev, curr, w, h, cfg, zones)` por diferença de luma com stride; aceita `ZoneConfig` | seção `[motion]` no `config.rs`; guardar frame anterior por câmera; chamar no tick/pipeline; emitir `EventType::Motion`; máscaras; `lightning_threshold` |
| `zones.rs` | 250 | 11 | **Não** (só via `motion.rs` e `zone_editor.rs`) | `Point`, `MotionZone::contains`, `ZoneConfig::filter_motion_points`, `MotionZoneFile` (TOML) | `[[cameras.zones]]` no config; inertia/loitering; zonas obrigatórias para alerta |
| `ui/zone_editor.rs` | 174 | — | **Não** | widget canvas | view, `Message`s, persistência, ligação ao menu de contexto |
| `multi_stream.rs` | 156 | 10 | **Não** | `StreamQuality`, `MultiStreamConfig`, `stream_url_for_quality`, `has_sub_stream` | `sub_url` em `[[cameras]]`; troca de stream em `sync_active_streams`/spotlight; reinício de pipeline sem perder gravação |
| `streaming.rs` | 237 | 7 | **Não** | máquina de estados `evaluate_streaming` (pausa em cena estática, warmup, retomada por movimento) | depende de `motion`; ligar ao `bridge.stop()`/start; snapshot estático enquanto pausado |
| `ptz.rs` | 196 | 6 | **Não** | `PtzCommand`, `PtzConfig`, presets, `PtzResult` | cliente ONVIF (nenhum existe), UI de controle, atalhos |
| `timelapse.rs` | 254 | 7 | **Não** | `SpeedMultiplier`, formato, nome de arquivo, estimativa de duração | pipeline que gera o timelapse; não há caller |
| `hw_encoder.rs` | 198 | 9 | **Não** | `HwEncoderBackend`, `detect_backend`, `build_encoder_string` | `start_recording` em `ui/pipeline.rs` usa `x264enc` fixo; ligar a seleção de encoder (VA-API/NVENC/etc.). Verificar se `detect_backend` realmente sonda o GStreamer ou só devolve o preferido |
| `bidirectional_audio.rs` | 147 | 6 | **Não** | config, `AudioEncoding`, `MicState` | captura do microfone e envio (backchannel RTSP); nenhum pipeline |

## Ligados (referência)
`audio`, `codec`, `diagnostics`, `groups`, `metrics`, `recording`, `redact`, `snapshot`, `timeline`, `view`.
Ressalva: `audio`, `codec`, `recording`, `snapshot` têm `#![allow(dead_code)]`, ou seja, parte
da API dentro deles também não é usada.

## Observações
- O `AGENTS.md` diz que "não há flags de CLI", mas `src/bin/iced_viewer.rs` usa `clap::Parser`
  para o caminho do config (argumento posicional). Consistente com "sem flags", porém `clap`
  com features `derive` e `env` é pesado para um argumento; `env` não é usado.
- Os módulos não ligados têm testes unitários, então a lógica pura está coberta; o risco
  está na integração (threads, mutexes, pipeline), não no cálculo.
- Ordem de ligação recomendada (dependências): `motion` → `zones` (+ editor) → `streaming` →
  `multi_stream`. `ptz`, `timelapse`, `hw_encoder`, `bidirectional_audio` são independentes.
