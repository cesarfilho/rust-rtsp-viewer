# Plano de execução do roadmap

Criado em 2026-10-05 (v0.8.0). Complementa `docs/roadmap.md` (o quê) com ordem, critérios
de saída, tamanhos e decisões pendentes (o como e o quando). Estado real do código em
`docs/status.md` e `docs/gap_analysis.md`.

## Convenções
- **Tamanho** (palpite relativo, sem medição; o roadmap já dizia que estimativas exigem
  spikes): **P** ≤ 1 dia · **M** 2–4 dias · **G** 1–2 semanas · **XG** > 2 semanas.
- **D** = decisão do dono que bloqueia tarefas. Tarefas sem D podem ser executadas direto.
- Versões propostas: M0 → 0.8.x · M1 → 0.9 · M2 → 0.10 · M3 → 0.11 · M4 → 0.12 · M5 → 1.0.
- Estado: `[x]` feito · `[ ]` aberto.

## Defeitos achados ao planejar
1. `scripts/baseline.sh` nunca achava o processo: `pgrep -x` com nome de 16 caracteres (o nome
   do processo é truncado em 15). Sem baseline não há como otimizar (M2, M4).
2. **Câmeras fora da página ficam cegas**: `detect_camera_motion` só avalia câmeras
   `Live`/`Recording` e `pause_hidden` vem ligado por padrão. Movimento, gravação por evento e
   notificação não funcionam em páginas ocultas (ADR 0008).

## Decisões pendentes
| ID | Decisão | Bloqueia |
|---|---|---|
| D1 | Posicionamento: **video wall nativo** (IA opcional/externa) ou **NVR completo** | M3, M4 inteiros |
| D2 | Câmeras reais: quantas, modelos, se aceitam 2 sessões RTSP, se têm sub-stream | 0.3, 0.4, 2.5 |
| D3 | Licença do modelo de detecção (YOLO da Ultralytics é AGPL; o projeto também é, confirmar) | 4.1 |
| D4 | Windows/macOS: manter só "compila" (ADR 0001) ou subir o nível | 5.6 |

## M0 — Fundação e medição (0.8.x)
| # | Tarefa | Critério de saída | Tam. | Estado |
|---|---|---|---|---|
| 0.1 | Corrigir `baseline.sh` (`pgrep -f`) e testar | roda contra o app aberto e gera CSV | P | [x] |
| 0.2 | CI: `cargo fmt --check`, `deny.toml` (licenças, CVEs), `rust-toolchain.toml`; `cargo test --doc` | CI falha se formatação/licença/CVE falhar | P | [x] |
| 0.3 | Medir sessões RTSP por modelo (`check_rtsp_sessions.sh`), preencher o ADR 0008 | nº de sessões por modelo (**D2**) | P | [ ] |
| 0.4 | Baseline real em 1/4/16 câmeras, main × sub → `docs/baseline.md` | CPU, RSS, fps, banda, VRAM por cenário (**D2**) | M | [ ] |
| 0.5 | Spike `gstreamer` 0.25 + `iced` 0.14 em branch, limite de 3 dias | decisão go/no-go documentada | G | [ ] |
| 0.6 | Spike `ort`: ONNX Runtime + CUDA + cuDNN, YOLO-n a 320 na GTX 1650 | ms/inferência e VRAM medidos | M | [ ] |
| 0.7 | Testes de integração (`tests/`) com `videotestsrc`: troca sub/main, reconexão, recuperação de falha na partida | fluxos hoje validados à mão viram teste | M | [x] |
| 0.8 | Validação de config com mensagem por campo e `--check` | erro aponta campo e linha; `--check` | M | [x] |

Gate M0: baseline real publicado, 0.5 decidido, ADRs "Proposta" promovidos ou rejeitados.
0.5 vem **antes** de qualquer widget wgpu próprio (2.3), senão o widget seria reescrito.

## M1 — Movimento confiável (0.9)
| # | Tarefa | Critério de saída | Tam. | Estado |
|---|---|---|---|---|
| 1.1 | **Corrigir a cegueira**: câmeras com `[motion]`/`on_motion` ficam fora do `pause_hidden`, decodificando o sub | movimento e gravação funcionam em câmera de página oculta; teste | P–M | [x] |
| 1.2 | Ramo de detecção reduzido (ADR 0005): `tee → leaky queue → videorate → videoscale → ~320×180 GRAY8 → appsink` no lugar do `capture_frame` em resolução cheia | CPU dentro do orçamento medido em 0.4 | M | [ ] |
| 1.3 | Recuperação rápida na partida: retentar em segundos, não só após os 12 s de graça | câmera que falha ao iniciar tenta de novo em ~1 s (limitado pela fonte, não pelo app) | P | [x] |
| 1.4 | Áudio usa o sub (ou só abre a sessão quando ouvido) | sem sessão extra com a câmera na grade | P | [ ] |
| 1.5 | Zonas: manter `zones.toml` e atualizar a spec (em vez de `[[cameras.zones]]`) | spec e código coerentes | P | [ ] |
| 1.6 | Opcional: inércia/loitering e `lightning_threshold` | testes unitários | M | [ ] |

Gate M1: critérios de aceite de `docs/specs/motion-zones.md`, sem `allow(dead_code)` novo.

## M2 — Escala e GPU (0.10)
Depende de 0.4 e 0.5.
| # | Tarefa | Critério de saída | Tam. |
|---|---|---|---|
| 2.1 | Spike barato: `gst-plugin-va`, `vah264dec` na iGPU Intel, CPU com o caminho RGBA atual | número comparado ao baseline | P |
| 2.2 | Em câmeras reais, conferir qual decoder o `decodebin` escolhe e se o `Via` mostra GPU | Inspector correto | P |
| 2.3 | **Caminho NV12 + shader** (widget wgpu próprio): `appsink` em NV12, conversão na GPU (0,94 s contra 2,27 s por stream medidos) | 16 × 1080p no orçamento; fallback RGBA mantido | G–XG |
| 2.4 | Zero-copy / PRIME offload (renderizar na NVIDIA) | só se 2.3 não bastar | XG |
| 2.5 | Topologia B (ADR 0008): pipeline *detect* sempre ligado no sub + *display* só para o visível | decidido com 0.3, 0.4 (**D1**, **D2**) | G |
| 2.6 | Ligar `streaming` (pausa em cena estática) | só se o baseline mostrar ganho; senão remover o módulo | M |
| 2.7 | Decidir `hw_encoder`: provavelmente **remover**, pois 3.1 elimina o reencode | módulo removido ou ligado | P |
| 2.8 | Teste de estresse: 16 fontes sintéticas + tempestade de reconexões | sem vazamento de threads/memória em 1 h | M |

Gate M2: 16 câmeras dentro do orçamento do baseline. Risco principal: 2.3.

## M3 — Gravação e histórico (0.11) — só se D1 = NVR
| # | Tarefa | Critério de saída | Tam. |
|---|---|---|---|
| 3.1 | **Gravação sem reencode** (ADR 0007): `rtph264depay ! h264parse ! splitmuxsink` com `tee` antes do decoder (troca o `decodebin` por cadeia manual) | arquivo tocável, sem CPU de encode, com áudio, H.265 | G |
| 3.2 | SQLite (`rusqlite` bundled, WAL, thread própria): `events` e `segments`; reconciliação na partida | queda no meio de um segmento não deixa órfão | G |
| 3.3 | Retenção por modo (contínuo × movimento) e limpeza por espaço; apagar arquivo e depois a linha | disco nunca passa do limite | M |
| 3.4 | Timeline persistente: clique no evento abre o trecho | busca por câmera e intervalo | M |
| 3.5 | Reprodução embutida (seek, velocidade) e exportar clipe (remux) | clipe tocável sem reencode | G |
| 3.6 | Pré-roll: ring buffer de GOPs codificados (depende de 3.1) | clipe começa no keyframe anterior ao evento | G |

## M4 — Detecção de objetos (0.12) — só se D1 = NVR
Depende de 0.6, 1.2 e D3.
| # | Tarefa | Critério de saída | Tam. |
|---|---|---|---|
| 4.1 | `feature = "detect"` com `ort` (CUDA/TensorRT), pré/pós-processamento e NMS | inferência correta em fixtures | G |
| 4.2 | Thread de inferência, fila limitada (descarta o mais antigo), métricas | nunca bloqueia UI nem appsink | M |
| 4.3 | Gatilho só com movimento e dentro das zonas, sobre a região recortada | GPU ociosa sem movimento | M |
| 4.4 | Eventos por label/score/zona no timeline e SQLite; notificação por label | filtro funciona | M |
| 4.5 | UI: caixas no spotlight, filtros, revisão "Alertas × Detecções" | | G |
| 4.6 | Avaliação: conjunto de teste com regressão por IoU/score; fallback em CPU | precisão/recall registrados | M |
| 4.7 | Empacotar `libonnxruntime` por SO | | M |

Alternativa leve (D1 = video wall): consumir eventos de fora (MQTT do Frigate) e mostrá-los
na timeline, em vez de 4.1–4.6 (tarefa **M** no lugar de ~**XG**).

## M5 — Acabamento (1.0)
| # | Tarefa | Tam. |
|---|---|---|
| 5.1 | ONVIF: descoberta (WS-Discovery) e assistente de cadastro; Profile T como base | G |
| 5.2 | PTZ via `oxvif`, ligando `ptz.rs` (depende de 5.1) | M |
| 5.3 | Credenciais no keyring (`secret-service`) | M |
| 5.4 | MQTT/Home Assistant (`rumqttc`) para eventos e saúde por câmera | M |
| 5.5 | i18n (pt-BR + en) e acessibilidade | G |
| 5.6 | Empacotamento: AUR, AppImage/Flatpak, releases automáticas (`cargo-dist`) (**D4**) | M |
| 5.7 | Decidir `timelapse` e `bidirectional_audio`: ligar ou remover | P cada |
| 5.8 | Remover os 10 `allow(dead_code)` restantes e os módulos descartados | P |

## Caminho crítico
Depois de M0, `0.1 → 0.4 → M2` (escala) e `0.6 → M4` (IA) andam em paralelo. Ordens que
não podem inverter: 0.5 antes de 2.3 · 3.1 antes de 3.6 · 1.2 antes de 4.3 · 0.4 antes de
qualquer otimização.

## Sprint 1 (sem depender de câmeras reais)
1. 0.1 — conserto do `baseline.sh`.
2. 0.2 — `fmt`, `deny` e `rust-toolchain` no CI.
3. 1.1 — cegueira do `pause_hidden`, com teste.
4. 1.3 — recuperação rápida na partida.
5. 0.8 — validação de config.

## Riscos
- Os tamanhos são palpite; o baseline real (0.4) pode mudar a ordem de M2.
- Toda a GPU de exibição depende de 2.3, o item mais incerto.
- M3 + M4 equivalem a construir um NVR: se D1 for "video wall", cerca de metade do plano some.
