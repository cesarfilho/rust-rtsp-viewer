# Spec — Movimento + Zonas (roadmap M1)

Estado: rascunho para revisão. Depende de ADR 0005 (detect stream).
Referência de comportamento: Frigate (motion + zones).

## Objetivo
Detectar movimento por câmera em um ramo de baixa resolução, filtrar por zonas e emitir
eventos `EventType::Motion` na timeline, sem afetar o display nem a UI thread.

## O que já existe (não reescrever)
- `domain/motion.rs`: `MotionConfig { enabled, threshold: u8, contour_area: f64, sample_stride }`,
  `MotionConfigFile` (espelho TOML **já definido**, ainda não referenciado por `config.rs`),
  `detect_motion(prev, curr, w, h, cfg, Option<&ZoneConfig>) -> Option<MotionResult>` (RGBA, luma por diferença absoluta),
  `MotionResult { motion_level, motion_active, changed_pixels, total_sampled }`. 14 testes.
- `domain/zones.rs`: `Point` (normalizado 0..1), `MotionZone { name, vertices, enabled }`, `ZoneConfig::is_motion_allowed`
  (sem zonas = tudo conta), `MotionZoneFile`/`PointFile` (TOML). 11 testes.
- `ui/zone_editor.rs`: widget canvas de edição (174 linhas), sem view/mensagens.
- `EventType::Motion` existe (cor `#FFA500`) e nunca é emitido.
- Nota: `detect_motion` recebe RGBA; o ramo de detect do ADR 0005 pode entregar RGBA em 320x180
  (simples) ou GRAY8 (mais barato, exigiria nova função). Decidir no spike.

## Config (`config.toml`)
```toml
[motion]                 # global; cada [[cameras]] pode sobrescrever
enabled = true
threshold = 30           # 1–255, diferença de luma por pixel
contour_area = 0.01      # fração do quadro (0.0–1.0) — ATENÇÃO: no Frigate é em pixels; aqui já é fração
sample_stride = 2        # 1–32
detect_fps = 5           # NOVO: taxa do ramo de detecção (1–30)
detect_width = 320       # NOVO: largura do ramo de detecção (altura mantém proporção)
cooldown_secs = 5        # NOVO: mínimo entre eventos Motion consecutivos
end_after_secs = 3       # NOVO: sem movimento por N s encerra o "período de movimento"

[[cameras]]
url = "rtsp://..."
  [[cameras.zones]]      # NOVO: valida em `into_zone`
  name = "Portão"
  vertices = [{ x = 0.1, y = 0.2 }, { x = 0.6, y = 0.2 }, { x = 0.6, y = 0.9 }]
```
Validação (mensagem por campo, incluindo nome da câmera): polígono ≥ 3 vértices, coordenadas em 0..1,
`threshold` e `sample_stride` dentro dos limites (hoje o `into_config` apenas faz `clamp`; decidir se
clamp silencioso é aceitável ou se deve gerar erro).

## Componentes a criar/alterar
| Arquivo | Mudança |
|---|---|
| `src/config.rs` | campo `motion: Option<MotionConfigFile>` global e por câmera; `zones: Option<Vec<MotionZoneFile>>` por câmera |
| `src/ui/pipeline.rs` | ramo `detect_queue → videoscale → videorate → capsfilter → detect_sink` no `tee`; callback que guarda o último frame de detecção |
| `src/ui/bridge.rs` | estado de movimento por câmera (frame anterior, nível, `motion_active`, último evento) protegido por mutex; **nunca segurar o guard durante reconnect** (AGENTS.md) |
| `src/ui/update.rs` | no `FrameUpdate`, ler estado de movimento e chamar `push_event(Motion)` respeitando `cooldown_secs` |
| `src/ui/view/cell_overlay.rs` | indicador de movimento no pip/cell |
| `src/ui/zone_editor.rs` | ligar a uma view/modal e a `Message`s (`ZoneEditOpen`, `ZonePointAdd`, `ZoneSave`, …) |
| `src/infrastructure/view_state.rs` (ou arquivo próprio) | persistir zonas editadas em runtime |

## Regras
- O cálculo de movimento roda **fora** do callback do display e da thread da UI (thread própria por câmera ou pool pequeno).
- Frame anterior: só substituir depois de comparar (mesma lição do `sample_image_quality_rgba`, ver AGENTS.md).
- Câmera pausada por `pause_hidden` também pausa a detecção (ou mantém só o ramo leve — decidir; impacta o smart streaming).
- Mudanças bruscas do quadro inteiro (IR/cor, PTZ) devem ser ignoradas: implementar `lightning_threshold` (fração do quadro acima da qual o frame é descartado). Ainda **não existe** no código.

## Critérios de aceite
1. Com `videotestsrc pattern=ball`, `motion_active` fica true; com `pattern=black`/estático, false.
2. Movimento fora das zonas ativas não gera evento; dentro gera exatamente 1 evento por período (cooldown respeitado).
3. Com movimento habilitado e 16 câmeras sintéticas, o custo adicional de CPU medido (via `scripts/baseline.sh`) fica dentro do orçamento definido no roadmap.
4. Sem regressão: `cargo test`, `cargo clippy` sem avisos; nenhum `allow(dead_code)` novo.
5. Reconnect da câmera não trava a UI (teste manual com `test-graceful-shutdown.sh`/desconexão).

## Testes
- Unitários novos: `lightning_threshold`, cooldown/fim de período, validação de config/zona inválida.
- Integração: pipeline com `videotestsrc` + ramo detect, esperando N eventos (padrão dos testes de gravação em `ui::pipeline`, ~segundos).

## Fora de escopo
Inertia/loitering (M1.5), máscaras de movimento (só zonas inclusivas por enquanto), ML.
