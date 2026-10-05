# Spec — Movimento + Zonas (roadmap M1)

Estado: **parcialmente implementada** (revisada em 2026-10-05, tarefa 1.5 do plano). O rascunho
original previa `[[cameras.zones]]` no TOML e um ramo de detecção dedicado; o código seguiu
outro caminho (abaixo). Depende do ADR 0005 (detect stream) para o que ainda falta.
Referência de comportamento: Frigate (motion + zones).

## Objetivo
Detectar movimento por câmera, filtrar por zonas e emitir `EventType::Motion` na timeline,
sem afetar o display nem a UI thread.

## Como está hoje
- **Detecção**: `update::detect_camera_motion` amostra o último quadro **decodificado e em
  resolução cheia** (`bridge.capture_frame()`) a ~2 Hz (a cada 5º tick de 100 ms) e compara com a
  amostra anterior via `domain::motion::detect_motion` (RGBA, luma por diferença absoluta,
  `sample_stride`). Registra `EventType::Motion` na **borda de subida**. Não há ramo de detecção
  separado no pipeline (isso é a tarefa 1.2).
- **Config**: `[motion]` é **global** (`enabled`, `threshold`, `contour_area`, `sample_stride`),
  lido em `Config.motion` → `App.motion_config`. Não há sobrescrita por câmera.
- **Zonas**: não ficam no `config.toml`. São desenhadas no editor (`ui::zone_editor`, aberto pelo
  menu da câmera → `Message::EditZones`) e persistidas em `~/.local/state/rust-rtsp-viewer/zones.toml`
  (`infrastructure::zone_state`), **chaveadas pelo nome da câmera, nunca pela URL** (a URL carrega
  senha). Com `ZoneConfig::has_active()` (habilitada e ≥ 3 vértices) só os pixels dentro da zona
  são amostrados, então `motion_level` é a fração alterada *da zona*.
- **Reações ao evento**: `[recording] on_motion` grava (`drive_motion_recording`, pós-roll em
  `motion_post_roll_secs`) e `[notifications]` dispara `notify-send` com cooldown por câmera/tipo.
- **Câmeras ocultas**: o movimento vem de quadros decodificados, então uma câmera pausada por
  `pause_hidden` fica cega. `domain::motion::needs_background_watch` (`[motion] enabled` **e**
  `on_motion` ou `[notifications]`) mantém todas decodificando.
- Testes: `domain/motion.rs` (detector), `domain/zones.rs` (geometria), `config_check` (validação).

## Decisões que divergem do rascunho
| Rascunho | Realidade | Decisão |
|---|---|---|
| `[[cameras.zones]]` no TOML | `zones.toml` em `~/.local/state` por nome de câmera | **Manter** o arquivo de estado: zonas se desenham na UI, e misturar estado editado em runtime com o `config.toml` anotado à mão cria conflito. |
| Ramo `detect_queue → videoscale → videorate → GRAY8` | Amostragem do quadro RGBA cheio | Pendente, tarefa **1.2**. |
| `detect_fps`, `detect_width`, `cooldown_secs`, `end_after_secs` | Não existem; cadência fixa de ~2 Hz, 1 evento por borda de subida | Só implementar se a medição (0.4) mostrar necessidade. |
| `[motion]` por câmera | Só global | Aberto, sem demanda registrada. |

## Falta (M1)
- **1.2** ramo de detecção reduzido (~320×180), para tirar o custo de comparar quadros cheios.
- **1.6** `lightning_threshold` (descartar mudança brusca do quadro inteiro: IR/cor, PTZ) e
  inércia/loitering. Não existem no código.
- Validação de valores de `[motion]`: `into_config` faz `clamp` silencioso; `config_check` já
  avisa fora de faixa, então o clamp fica como rede de segurança.

## Regras
- A comparação não pode rodar no callback do display; hoje roda na thread da UI a 2 Hz (barato o
  bastante a 1 câmera, a medir com 16 — por isso 1.2).
- Frame anterior: só substituir depois de comparar (mesma lição do `sample_image_quality_rgba`).
- **Nunca** segurar o guard do bridge durante reconexão (AGENTS.md).

## Critérios de aceite
1. `videotestsrc pattern=ball` → `motion_active` verdadeiro; padrão estático → falso. *(coberto
   por testes unitários do detector)*
2. Movimento fora das zonas ativas não gera evento; dentro gera 1 evento por período. *(zonas:
   coberto; "1 por período" = borda de subida, sem cooldown configurável)*
3. 16 câmeras sintéticas com movimento ligado dentro do orçamento de CPU medido com
   `scripts/baseline.sh`. **Aberto** (depende de 0.4 e 1.2).
4. `cargo test` e `cargo clippy` limpos, sem `allow(dead_code)` novo. *(vale a cada commit)*
5. Reconexão não trava a UI. *(coberto por `tests/bridge_flows.rs` + validação manual)*

## Fora de escopo
Máscaras de movimento (só zonas inclusivas), ML (ver M4).
