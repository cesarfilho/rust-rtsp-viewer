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
| `ptz.rs` | 6 | `PtzCommand`, `PtzConfig`, presets | cliente ONVIF (não existe), UI de controle, atalhos |

Todos têm testes unitários, então o risco está na integração (threads, mutexes,
pipeline), não no cálculo.

## `allow(dead_code)`

Zero. Os 10 que existiam foram removidos em 2026-10-05: só um escondia código morto de verdade
(`AudioController`, usado apenas pelos próprios testes), e saiu junto com eles.

## Observações
- Fora de testes não há `unwrap()`/`expect()` nem `unsafe`; há 1 `TODO` no código.
- `clap` continua sendo usado só para o caminho do config (argumento posicional).
- Dos módulos que não eram usados, sobrou `ptz` (a base do PTZ via ONVIF, plano 5.2). `streaming`, `timelapse`,
  `bidirectional_audio` e `hw_encoder` foram removidos (voltam do histórico do git, se preciso).
