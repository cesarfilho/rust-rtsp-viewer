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
| D1 | ~~Posicionamento: video wall nativo ou NVR completo~~ → **decidido em 2026-10-05: NVR completo, com UX/UI muito bem definida** | (liberou M3 e M4) |
| D2 | Câmeras reais: quantas, modelos, se aceitam 2 sessões RTSP, se têm sub-stream | 0.3, 0.4, 2.5 | **Respondida:** 1 Intelbras local, as demais são HLS públicas remotas.
| D3 | Licença do modelo de detecção (YOLO da Ultralytics é AGPL; o projeto também é, confirmar) | 4.1 |
| D5 | Vídeo ao vivo no cliente: sessão própria (A), redistribuição pelo daemon (B, recomendada) ou memória compartilhada (C) — ADR 0010 | M2.5 (2.5.4), 2.5 |
| D6 | Instalar `nvidia-container-toolkit` para o container usar a GTX 1650 (decodificação/YOLO em CUDA). Sem isso: VA-API na iGPU ou CPU | 2.5.10, M4 em GPU |
| D4 | Windows/macOS: manter só "compila" (ADR 0001) ou subir o nível | 5.6 |

## M0 — Fundação e medição (0.8.x)
| # | Tarefa | Critério de saída | Tam. | Estado |
|---|---|---|---|---|
| 0.1 | Corrigir `baseline.sh` (`pgrep -f`) e testar | roda contra o app aberto e gera CSV | P | [x] |
| 0.2 | CI: `cargo fmt --check`, `deny.toml` (licenças, CVEs), `rust-toolchain.toml`; `cargo test --doc` | CI falha se formatação/licença/CVE falhar | P | [x] |
| 0.3 | Medir sessões RTSP por modelo (`check_rtsp_sessions.sh`), preencher o ADR 0008 | nº de sessões por modelo (**D2**) | P | [x] 4 sessões na principal da Intelbras sem recusa; HLS sem limite. Ver `docs/baseline.md` |
| 0.4 | Baseline real em 1/4/16 câmeras, main × sub → `docs/baseline.md` | CPU, RSS, fps, banda, VRAM por cenário (**D2**) | M | [x] 1 Intelbras + 11 HLS medidos (`docs/baseline.md`); 16 câmeras simultâneas não (só há 12) |
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
| 1.2 | Ramo de detecção reduzido (ADR 0005): `tee → leaky queue → videorate → videoscale → ~320×180 GRAY8 → appsink` no lugar do `capture_frame` em resolução cheia | CPU dentro do orçamento medido em 0.4 | M | [~] ramo feito. **Defeito achado e corrigido em 2026-10-05:** o `detect_sink` bloqueava o preroll e congelava o pipeline de forma intermitente (agora `async=false`, com teste de regressão);  falta medir CPU com 16 câmeras (0.4) |
| 1.3 | Recuperação rápida na partida: retentar em segundos, não só após os 12 s de graça | câmera que falha ao iniciar tenta de novo em ~1 s (limitado pela fonte, não pelo app) | P | [x] |
| 1.4 | Áudio usa o sub (ou só abre a sessão quando ouvido) | sem sessão extra com a câmera na grade | P | [~] o áudio já só abre sessão quando ouvido; falta usar o sub, e muitos sub-streams não têm áudio: decidir com 0.3 (D2) |
| 1.5 | Zonas: manter `zones.toml` e atualizar a spec (em vez de `[[cameras.zones]]`) | spec e código coerentes | P | [x] |
| 1.6 | Opcional: inércia/loitering e `lightning_threshold` | testes unitários | M | [~] `lightning_threshold` feito; falta inércia/loitering |

Gate M1: critérios de aceite de `docs/specs/motion-zones.md`, sem `allow(dead_code)` novo.

## M2 — Escala e GPU (0.10)
Depende de 0.4 e 0.5.
| # | Tarefa | Critério de saída | Tam. |
|---|---|---|---|
| 2.1 | Spike barato: `gst-plugin-va`, `vah264dec` na iGPU Intel, CPU com o caminho RGBA atual | número comparado ao baseline | P |
| 2.2 | Em câmeras reais, conferir qual decoder o `decodebin` escolhe e se o `Via` mostra GPU | Inspector correto | P |
| 2.3 | **Caminho NV12 + shader** (widget wgpu próprio): `appsink` em NV12, conversão na GPU (0,94 s contra 2,27 s por stream medidos) | 16 × 1080p no orçamento; fallback RGBA mantido | G–XG | [~] **parte do daemon feita:** sem janela não há conversão RGBA (−57% de CPU, `docs/baseline.md`); falta NV12 + shader na janela |
| 2.4 | Zero-copy / PRIME offload (renderizar na NVIDIA) | só se 2.3 não bastar | XG |
| 2.5 | Topologia B (ADR 0008): pipeline *detect* sempre ligado no sub + *display* só para o visível | decidido com 0.3, 0.4 (**D1**, **D2**) | G |
| 2.6 | Ligar `streaming` (pausa em cena estática) | só se o baseline mostrar ganho; senão remover o módulo | M |
| 2.7 | Decidir `hw_encoder`: provavelmente **remover**, pois 3.1 elimina o reencode | módulo removido ou ligado | P |
| 2.8 | Teste de estresse: 16 fontes sintéticas + tempestade de reconexões | sem vazamento de threads/memória em 1 h | M | [~] `tests/stress.rs`: 5 min OK (7.482 ciclos, sem vazamento de threads/fds, RSS em platô); falta a corrida de 1 h (`RRV_STRESS_SECS=3600`) |

Gate M2: 16 câmeras dentro do orçamento do baseline. Risco principal: 2.3.

## M2.5 — Motor sem janela, em Docker (0.10.x) — ADR 0010
O NVR grava e detecta com a janela fechada. **Vem antes do M3**: gravação, SQLite, retenção e IA
passam a viver no daemon; construí-las dentro de `update.rs` e migrar depois custa muito mais.
| # | Tarefa | Critério de saída | Tam. | Estado |
|---|---|---|---|---|
| 2.5.1 | Desacoplar o iced do motor: `Handle`/`Bytes` fora de `bridge`/`pipeline` (hoje ~5 pontos) | `bridge` e `pipeline` compilam sem `iced` | P | [x] também moveu `CameraStatus` para `domain/` e separou `sample_status`/`CameraInfo::apply`; guarda em `tests/engine_isolation.rs` |
| 2.5.2 | Extrair a orquestração de `ui/update.rs` (reconexão, backoff, fila de partida, movimento, gravação por evento, notificações, eventos) para um módulo de motor sem `App` | o cliente atual usa o motor e todos os testes seguem verdes | G | [x] 2026-10-05: `engine::Engine` (+ `EngineEvent`); 27 testes novos; falta só o construtor `Engine::new` (vai com 2.5.3) |
| 2.5.3 | Workspace Cargo: `rrv-core` (domain + motor), `rrv-daemon`, cliente | `cargo build --workspace`; mesmo comportamento | M | [x] 2026-10-05: `rrv-core` (388 testes) + cliente (49); `Engine::new` e o crate `rrv-daemon` feitos (2.5.6). **MSRV subiu para 1.92** (glib/gstreamer 0.25) |
| 2.5.4 | Vídeo ao vivo do daemon para o cliente (**D5**; medir antes com 0.3); porta RTSP local publicada pelo container | cliente mostra 16 câmeras com 1 sessão RTSP por câmera | G | [ ] adiada: a sessão própria (D5-A) basta: a Intelbras aceita 4 sessões |
| 2.5.5 | IPC por socket Unix (`0600`): comandos, eventos, versão do protocolo | cliente liga/desliga gravação, edita zonas, recebe eventos | G | [x] 2026-10-05: `rrv_core::ipc` (protocolo, tratador puro, servidor, cliente) + `rrvctl`; 21 testes + 5 com o daemon real; verificado no Docker (rrvctl do host controla o daemon no contêiner). Falta a janela usar o canal (2.5.7) |
| 2.5.6 | `rrv-daemon` headless (binário sem iced), com shutdown limpo (finaliza segmentos). **Achado em 2026-10-05:** hoje um SIGTERM com gravação em curso deixa o arquivo com 0 bytes e ilegível (o app só finaliza pelo `Ctrl+Q`/Drop); `docker stop` envia SIGTERM | grava e detecta sem janela; SIGTERM fecha os arquivos (teste: matar durante a gravação e tocar o arquivo) | M | [x] 2026-10-05: `crates/rrv-daemon`, `Engine::step`/`shutdown`; teste de processo com SIGTERM durante a gravação (sai 0, arquivo tocável). Achou e corrigiu 2 congelamentos de pipeline |
| 2.5.7 | Cliente com estados de daemon (conectado, iniciando, ausente → motor embutido) e **spec de UX** (`docs/specs/ux-daemon.md`) | UX escrita antes do código; contraste testado | M | [x] 2026-10-05: spec aprovada e implementada (`DaemonLink`, `DaemonState`, `display_only`, chip/menu/banner/confirmações). Verificado na tela com daemon real: motor local, conectado + REC pelo daemon, `SIGSTOP` → banner, `SIGCONT` → reconecta. Critérios 3–9 cobertos por testes (77 na janela, 8 do link, 5 de display-only); contraste dos novos estados testado em todos os temas; **não verificado na tela**: a confirmação ao sair (só em teste) |
| 2.5.8 | Segredos fora do `config.toml` (Docker secrets/variáveis; keyring no cliente — antecipa 5.3) | senha fora do `config.toml`, do IPC e da imagem | M | [x] 2026-10-05: `${NOME}` (ambiente ou /run/secrets), percent-encoding no usuário/senha, Strict/Lenient, compose com `secrets`; 9+3 testes e 4 de processo (a senha não vaza no log, nem com RUST_LOG=debug). **Falta o keyring da janela (5.3)** e o IPC nunca carrega URL |
| 2.5.9 | **Docker**: `Dockerfile` multi-estágio (usuário não-root, GStreamer + x264), `compose.yaml` (`restart: unless-stopped`, `network_mode: host`, volumes `/data` `/state`, config `:ro`, `TZ`), `healthcheck` e build da imagem no CI | `docker compose up -d` sobe o NVR; `kill -9` no processo o reinicia sem segmento corrompido | M | [x] 2026-10-05: `Dockerfile`, `compose.yaml`, `.dockerignore` (exclui `config.toml`), healthcheck por batimento, job `docker` no CI. Verificado no Docker: `healthy`, `docker stop` → SIGTERM → exit 0, arquivo tocável e do UID certo. Imagem ~977 MB (otimizar) |
| 2.5.10 | GPU no container: VA-API por `/dev/dri` (Intel) agora; NVIDIA só com **D6** | `Decoder` no Inspector mostra GPU dentro do container | M | [x] 2026-10-05: `compose.vaapi.yaml`, `vah264dec (GPU)` verificado no contêiner com a iGPU (renderD129), decodificador exposto no IPC e no `rrvctl`. **Não reduz a CPU hoje** (71% × 74% de um núcleo, 4×1080p30): o custo é a conversão RGBA (tarefa 2.3). Detalhes em `docs/gpu-container.md` |
| 2.5.11 | Notificação com a janela fechada: webhook/MQTT de saída (antecipa 5.4) | aviso de movimento chega sem a janela aberta | M | [x] 2026-10-05: `[webhook]` (json/ntfy) via `curl`, fila limitada, URL nunca no log, desligamento descarta a fila; 9 testes + 2 de processo com receptor HTTP. **Não é MQTT**: MQTT ficou para o 5.4 |

Gate M2.5: fechar a janela não interrompe uma gravação em curso; matar o container com `kill -9`
o reinicia e não deixa segmento corrompido (testes em `tests/`); `docker compose up -d` do zero
funciona em uma máquina limpa.

## M3 — Gravação e histórico (0.11)

Spec de UX aprovada: `docs/specs/ux-historico.md` (padrões: movimento + pré-roll 5 s, retenção 7 dias, reprodução embutida).
| # | Tarefa | Critério de saída | Tam. |
|---|---|---|---|
| 3.1 | **Gravação sem reencode** (ADR 0007): `rtph264depay ! h264parse ! splitmuxsink` com `tee` antes do decoder (troca o `decodebin` por cadeia manual) | arquivo tocável, sem CPU de encode, com áudio, H.265 | G |
| 3.2 | SQLite (`rusqlite` bundled, WAL, thread própria): `events` e `segments`; reconciliação na partida | queda no meio de um segmento não deixa órfão | G |
| 3.3 | Retenção por modo (contínuo × movimento) e limpeza por espaço; apagar arquivo e depois a linha | disco nunca passa do limite | M |
| 3.4 | Timeline persistente: clique no evento abre o trecho | busca por câmera e intervalo | M |
| 3.5 | Reprodução embutida (seek, velocidade) e exportar clipe (remux) | clipe tocável sem reencode | G |
| 3.6 | Pré-roll: ring buffer de GOPs codificados (depende de 3.1) | clipe começa no keyframe anterior ao evento | G |

## M4 — Detecção de objetos (0.12)
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

Descartada com D1 = NVR: consumir eventos de fora (MQTT do Frigate) em vez de 4.1–4.6. O MQTT
entra só como integração de saída (5.4).

## M5 — Acabamento (1.0)
| # | Tarefa | Tam. |
|---|---|---|
| 5.1 | ONVIF: descoberta (WS-Discovery) e assistente de cadastro; Profile T como base | G |
| 5.2 | PTZ via `oxvif`, ligando `ptz.rs` (depende de 5.1) | M |
| 5.3 | Credenciais no keyring (`secret-service`) — antecipada para 2.5.8 | M |
| 5.4 | MQTT/Home Assistant (`rumqttc`) para eventos e saúde por câmera | M |
| 5.5 | i18n (pt-BR + en) e acessibilidade | G |
| 5.6 | Empacotamento: AUR, AppImage/Flatpak, releases automáticas (`cargo-dist`) (**D4**) | M |
| 5.7 | Decidir `timelapse` e `bidirectional_audio`: ligar ou remover | P cada |
| 5.8 | Remover os 10 `allow(dead_code)` restantes e os módulos descartados | P |

## Princípio de UX/UI (decorre de D1)
O produto é um NVR completo **e** precisa de UX/UI muito bem definida: cada tela nova do M3/M4
(linha do tempo, reprodução, revisão de eventos, caixas de detecção, retenção) tem **spec de
interface escrita antes do código** (`docs/specs/ux-*.md`): fluxo, estados vazio/erro/carregando,
atalhos, contraste (testes de tema) e critério de aceite visual. Reaproveitar a base atual:
`command_menu` único, `ThemeColors`, pip de status, `view::pinned`, ícones `icons::FONT`.

## Caminho crítico
Depois de M0, `0.1 → 0.4 → M2` (escala) e `0.6 → M4` (IA) andam em paralelo, mas **M2.5 antes de M3** (o motor sem janela é onde M3/M4 vivem). Ordens que
não podem inverter: 0.5 antes de 2.3 · 3.1 antes de 3.6 · 1.2 antes de 4.3 · 0.4 antes de
qualquer otimização.

## Sprint 1 (sem depender de câmeras reais) — concluída
1. 0.1 — conserto do `baseline.sh`.
2. 0.2 — `fmt`, `deny` e `rust-toolchain` no CI.
3. 1.1 — cegueira do `pause_hidden`, com teste.
4. 1.3 — recuperação rápida na partida.
5. 0.8 — validação de config.

## Riscos
- Os tamanhos são palpite; o baseline real (0.4) pode mudar a ordem de M2.
- Toda a GPU de exibição depende de 2.3, o item mais incerto.
- M3 + M4 equivalem a construir um NVR (D1 decidido): é a maior parte do trabalho restante e o maior risco de prazo.
