# Changelog

Todas as mudanças notáveis neste projeto serão documentadas neste arquivo.

O formato é baseado em [Keep a Changelog](https://keepachangelog.com/pt-BR/1.0.0/).

## [Não lançado]

### ✨ Adicionado

- **Gravar só quando há um objeto** (`[detect] record = true`): a gravação começa quando uma classe de `[detect] labels`
  aparece (dentro das zonas) e para `motion_post_roll_secs` depois da última vez que foi vista, com o pré-roll de
  `[recording]`. Sombra, folha e chuva movem a cena mas não gravam. Vale com `on_motion = false`; a retenção a trata como
  gravação de evento (modo `detection`, mesmos dias da de movimento).
- **Snapshot da detecção** (`[detect] snapshot = true`): um JPEG do quadro inteiro com as caixas desenhadas a cada evento de
  detecção, em `<gravações>/snapshots/<câmera>/<AAAA-MM-DD>/<HH-MM-SS>_<classe>.jpg`. O daemon guarda só uma referência ao
  último quadro decodificado e converte numa thread à parte.
- **Botão Iniciar / Parar detecção** na janela (com um daemon que detecta objetos): a gravação e as fotos por detecção
  só acontecem com ela iniciada. O botão vira "Parar" quando o daemon confirma. O daemon começa com a detecção parada e
  lembra a escolha depois de reiniciar. Também `rrvctl detect on|off`.
- **Objetos parados não são eventos**: uma caixa da mesma classe vista no mesmo lugar em 3 minutos diferentes dentro de
  30 minutos, espalhados por 10 minutos ou mais (uma placa que o modelo lê como pessoa toda vez que outra coisa se mexe),
  deixa de gerar evento, gravação e foto por 24 h. Os pontos fixos aprendidos sobrevivem a um reinício
  (`static_spots.json`). As primeiras vezes ainda contam; alguém esperando alguns minutos no mesmo lugar não vira "fixo".
- **A foto da detecção é o quadro que o modelo viu**, não o mais novo: o daemon guarda os últimos 4 s (referências, um
  quadro a cada 200 ms) e escolhe pelo tempo do quadro. Antes, quem andava já tinha saído da caixa e a foto parecia um
  falso positivo.
- **Detecção só em algumas câmeras** (`[detect] cameras = ["Garagem"]`, pelo label ou name): as outras continuam com
  movimento e eventos, mas não ocupam o modelo (rios e árvores se mexem o tempo todo). `--check` avisa nome desconhecido.
- **Gravações como pasta de rede (SMB)**: serviço `samba` no `compose.yaml` (imagem mínima própria, `docker/samba/`), só
  leitura, SMB2/3, sem convidado; usuário `rrv` e senha em `secrets/smb_password`. `\\<host>\gravacoes`.

### 🔧 Alterado

- `compose.yaml`: o daemon usa `restart: always`.
- **Gravações em pastas por câmera e dia**: `<gravações>/<câmera>/<AAAA-MM-DD>/<HH-MM-SS>-000.mkv` (hora local, como os
  snapshots), em vez de todos os arquivos soltos na raiz com a hora em UTC. A pasta é escolhida a cada segmento: uma
  gravação que passa da meia-noite continua na pasta do dia novo.

### 🐛 Corrigido

- **O daemon do Docker reiniciava sem parar depois de um reboot**: o contêiner sobe antes do login, o Docker criava
  `$XDG_RUNTIME_DIR/rrv` como root e o socket não abria. O socket agora fica em `~/.local/state/rust-rtsp-viewer/run/`
  (no disco), e a janela / `rrvctl` o acham sozinhos quando não há um daemon nativo.

## [0.11.0] - 2026-10-06

### ✨ Adicionado

- **A janela em outra máquina, pela LAN (IP)**: o daemon escuta em TCP (`[daemon] listen`) e a janela / `rrvctl` conectam com
  `tcp://host:porta` e um token (desafio-resposta HMAC-SHA256; o token não trafega). O daemon serve os vídeos gravados por HTTP
  (`Range`, URLs assinadas e curtas) e a janela baixa os clipes exportados. **O tráfego não é criptografado**: só numa LAN de
  confiança ou por VPN.

### 🐛 Corrigido

- **Modo flex**: ao trocar de câmera o vídeo aparecia só na miniatura e a área grande ficava preta (o mesmo vídeo era desenhado
  duas vezes e dividia o retângulo da GPU). A miniatura da câmera principal agora só mostra o nome, e as demais se renovam a cada 60 s.
- **Vista Gravações**: a linha "Comparar" quebra de linha e os controles do player ficam em duas linhas.

## [0.10.0] - 2026-10-06

Detecção de objetos, ONVIF, chaveiro, idiomas e o vídeo na GPU (NV12), sobre o iced 0.14.

### ✨ Adicionado

- **Detecção de objetos** (YOLO11 da Ultralytics, ONNX Runtime carregado em execução): `[detect]` (`enabled`, `model`, `backend`
  cpu/cuda/auto, `min_score`, `iou`, `labels`, `cooldown_secs`). Só roda onde há movimento e dentro das zonas; o evento
  `detection` (rótulo, confiança, caixa, zona) vai ao histórico (schema v3), aos avisos e ao webhook. `rrvctl history --label person`.
  Daemon com `make bin FEATURES=detect`; `scripts/fetch-model.sh` baixa o modelo (SHA-256 fixo) e a libonnxruntime;
  imagens Docker `detect-cpu` e `detect-cuda` (`compose.detect.yaml`, `compose.detect-cuda.yaml`). Medido: CUDA (GTX 1650)
  4,6 ms a 320 px e 9,7 ms a 640 px; CPU 27 / 68 ms (`docs/spike-0.6-ort.md`).
- **Caixas dos objetos** no spotlight e no player de gravações, botão "Objeto:" na lista de eventos da vista Gravações.
- **ONVIF** (só descoberta e cadastro): `rrvctl discover [--user U]` acha as câmeras (multicast + varredura unicast da sub-rede) e
  imprime o trecho do `config.toml` com o stream principal e o sub-stream; a senha vai como `${SEGREDO}`.
- **Assistente "Adicionar câmera…"** no menu `⋯`: procura as câmeras ONVIF, pede usuário e senha, lê os streams, guarda a senha no chaveiro e dá o trecho do `config.toml` para copiar.
- **Chaveiro do sistema** (Secret Service) como terceira origem de `${NOME}`: `rrvctl secret set|check|delete`,
  `rrvctl discover --store-secret`. A janela lê a senha da câmera do chaveiro, sem ela no `config.toml`.
- **Idiomas**: pt-BR e inglês, trocados no menu `⋯` → Aparência (ou `language = "en"`); `tests/i18n_scan.rs` impede texto fixo.
- **MQTT** opcional (`[mqtt]`): estado de cada câmera e eventos, com descoberta do Home Assistant; só `mqtt://` (sem TLS).
- **Vídeo na GPU**: widget `shader` com NV12 (1,5 byte por pixel em vez de 4; YUV→RGB na GPU), RGBA como reserva
  (`RRV_DISPLAY_FORMAT=rgba`).
- `make bin` deixa janela, daemon e `rrvctl` em `./bin/`.

### 🔧 Alterado

- **iced 0.14** (wgpu 27). A interface é desenhada na iGPU Intel (`WGPU_POWER_PREF=low`); a GTX 1650 clareava as cores. O estado anterior
  (iced 0.13) fica na branch `iced-0.13-final`.
- O vídeo deixou de usar `iced::widget::image` (no 0.14 imagens > 2 MiB carregam de forma assíncrona e piscavam).

## [0.9.0] - 2026-10-06

O app passa a ser um **NVR que grava e detecta com a janela fechada**: um daemon (`rrv-daemon`, no Docker) faz o trabalho
de vídeo e a janela é um cliente dele. Só Linux.

### ✨ Adicionado

- **Daemon sem janela** (`crates/rrv-daemon`) e a CLI **`rrvctl`** (`status`, `record`, `enable|disable`, `zones`, `events`,
  `history`, `export`). Canal de controle por socket Unix `0600`, JSON por linha, protocolo versionado. A janela conecta, mostra o
  estado (chip, banner de contato perdido, confirmações) e só aplica o que o daemon confirma.
- **Docker**: `compose.yaml` (rede host, UID/GID, healthcheck, `docker stop` finaliza as gravações), segredos (`${NOME}` do
  ambiente ou de `/run/secrets`, senha codificada na URL), **webhook** (`json` ou `ntfy`) para o aviso com a janela fechada.
- **GPU**: decodificação pela iGPU Intel (`compose.vaapi.yaml`, CPU do daemon −47%); NVIDIA preparada por CDI
  (`compose.nvidia.yaml`, `scripts/check-nvidia-host.sh`; **ainda não testada**).
- **Gravação sem reencode** para câmeras RTSP H.264/H.265 (começa no keyframe, ~0,1% de CPU), **pré-roll** (`[recording]
  motion_pre_roll_secs`, padrão 5) e **áudio opcional** (`record_audio`, desligado por padrão; AAC em mkv/mp4, G.711 só em mkv).
- **Histórico persistente** (SQLite): segmentos e eventos; **retenção** (`[retention]`: `motion_days` 7, `manual_days`,
  `max_disk_percent` 80); **aviso de disco** (< 10% livre) por log, webhook e janela; **proteger** trechos.
- **Vista Gravações** (tecla `t`): linha do tempo por câmera (zoom, deslocamento, arrastar), player (seek, 0,5×–4×, quadro a
  quadro), **até 4 câmeras lado a lado**, lista de eventos, marcar I/O e **exportar clipe** `.mp4` sem reencode, atualização automática.
- `scripts/baseline-docker.sh` e `docs/baseline.md` (1/4/16 câmeras, CPU × GPU).
- **`--check`** e validação do `config.toml` (chaves com erro de digitação, faixas, URLs, duplicatas).

### 🔧 Alterado

- **Workspace** `rrv-core` (motor, sem `iced`) / `rrv-daemon` / janela; GStreamer 0.25 (MSRV 1.92).
- O daemon **não converte mais para RGBA** (CPU −57% com 11 câmeras); decodificadores de software limitados a 2 threads
  (memória por câmera ~190 → ~85 MiB). 16 câmeras: 65% de um núcleo (39% com a iGPU).
- **Só Linux** (ADR 0001 revisada). CI com `fmt`, `clippy`, `cargo-deny` e MSRV.

### 🐛 Corrigido

- **Os arquivos de gravação agora levam o nome da câmera**: duas câmeras que começavam no mesmo segundo geravam o mesmo nome
  e uma sobrescrevia a outra.
- Pipeline congelava de forma intermitente (sink do ramo de detecção e `splitmuxsink` assíncronos).
- Áudio que o muxer não aceita (G.711 em mp4) derrubava a câmera por um erro de pipeline; agora a gravação sai só com vídeo.
- Segmentos sem vídeo (a gravação parou antes do primeiro keyframe) não ficam no histórico.
- Câmeras fora da página ficavam cegas com `pause_hidden`; recuperação rápida na partida; `scripts/baseline.sh`.

### 🗑️ Removido

- Módulos que ninguém usava: `streaming`, `timelapse`, `bidirectional_audio`, `hw_encoder`, `ptz`, `AudioController`;
  todos os `allow(dead_code)`.

## [0.8.0]

### ✨ Adicionado

- **Sub-stream por câmera** (`sub_url` em `[[cameras]]`): os blocos da grade decodificam o
  stream de baixa resolução; spotlight, destaque do Flex e gravação usam o principal. A
  gravação sempre sai no stream principal (o app troca antes de começar a gravar).
- **Decoder em uso no Inspector** (`Decoder` e `Via` CPU/GPU), descoberto no pipeline em
  execução.

### 🐛 Corrigido

- **Bitrate do Inspector** mostrava a vazão do frame RGBA decodificado, não a do stream;
  agora conta os bytes comprimidos na entrada do decoder.
- **Resolução/codec velhos** no Inspector depois de reconectar ou trocar de stream.

## [0.7.2]

### 🐛 Corrigido

- **Câmera presa em "Reconectando" para sempre**: um stream que falhava logo depois
  de iniciar (câmera offline no start, playlist HLS ainda não pronta) nunca era
  retentado — o backoff só era armado dentro de `reconnect_camera`. Agora o retry é
  agendado quando a câmera fica offline após o período de graça, e desarmado a cada
  reconstrução (evita rebuild a cada tick).
- **Credenciais vazavam no log** dos pipelines de áudio (`audio.rs`): a URL e o erro
  do parser agora passam por `mask_credentials`.
- **URLs sem escape nos pipelines de áudio**: `"`, `&`, `!` ou espaço quebravam o
  `parse_launch`; agora usam `quote_launch_value` (movido para `infrastructure::launch`).
- **Ajuda (`?`) com setas como quadrados vazios**: `←`/`→` usam a fonte embutida.
- **Ajuda agora é modal para o teclado**: `Esc` a fecha e `f`/`s`/`r` não agem mais
  por trás dela.

### 🔧 Alterado

- Planos de ferramentas locais (`.mimocode/`, `.opencode/`) saíram do versionamento.
- `AGENTS.md`: removida a nota obsoleta sobre `cargo test --doc`; documentadas as
  regras de retry, ajuda modal e `quote_launch_value`.

## [0.7.1]

### 🔧 Alterado

- `AGENTS.md` revisado contra o código: variáveis de ambiente (`RUST_LOG`), seções
  de configuração (`[notifications]`, `[motion]`), descrição de `stop_recording`,
  âncora do menu de contexto, pacote `gstreamer1.0-plugins-ugly` e tabela de domínio.

## [0.7.0]

### ✨ Adicionado

- **Zonas de movimento no editor**: linhas até o cursor, fechar clicando no 1º
  ponto, cursor de mira; recusa zonas sem área e vértices duplicados; zonas por
  câmera em `zones.toml`.
- **`[motion]` configurável** (`enabled`, `threshold`, `contour_area`,
  `sample_stride`) — a seção existia no código mas nunca era lida.
- **Notificações de desktop** (`[notifications]`) para movimento e câmera offline,
  com cooldown por câmera/tipo; gravação automática por movimento
  (`[recording] on_motion`).
- **Ícones das funções no spotlight** (snapshot, gravar, áudio, zonas) com tooltip.
- **Fonte de ícones embutida** (DejaVu Sans) — acaba com os quadrados vazios.
- `LICENSE` (AGPL-3.0-or-later), `THIRD_PARTY_NOTICES.md`, `SECURITY.md`, CI.

### 🐛 Corrigido

- **Movimento com zonas**: o nível era medido sobre o quadro inteiro, então uma
  zona pequena nunca atingia `contour_area`; zonas desativadas/degeneradas
  desligavam a detecção.
- **Gravação**: `stop()` agora aguarda o finalizador assíncrono (segmento não é
  mais truncado) e `start_recording` desfaz os elementos/pad do `tee` em falha.
- **Menus**: `Container::align_*` define tamanho no iced 0.13; o menu mudava de
  tamanho e seguia o mouse. Novo helper `pinned` (também corrige badges, toasts e
  barras). Cliques nas barras do spotlight não atravessam para o editor de zonas.
- **Temas**: contraste revisado (OpenCode, Light, Dark, AMOLED, Cosmic) com teste
  de regressão; texto sobre emblemas coloridos escolhe preto/branco.

### 🔧 Alterado

- MSRV documentado corretamente: Rust **1.88** (let-chains), não 1.85.

### 🛡️ Estabilidade

- **Mutex poisoning não é mais panic**: todos os `.lock().unwrap()` em
  `grid_app.rs` e `camera_slot.rs` foram substituídos por
  `unwrap_or_else(|e| e.into_inner())`, garantindo que o lock nunca
  seja ponto de pânico mesmo após uma thread panificar enquanto segura
  o mutex.
- **Build limpo**: removido warning de `unused import` em `audio.rs:235`
  e warning de `unused_mut` em `camera_slot.rs:363`.

### ✨ Adicionado

- **Modo grade multi-câmera**: novo módulo `infrastructure::grid_app` e
  `infrastructure::camera_slot` para exibir múltiplos feeds RTSP/HLS em
  grid. Ativa automaticamente quando o `config.toml` contém uma seção
  `[[cameras]]`. O layout do grid é auto-calculado (1×1, 2×1, 2×2, 3×2,
  3×3, etc.). Tecla `m` para mudo / `r` para gravar no slot selecionado.
  Badges de status visual: ♪ AUD (verde), ● REC (vermelho), mudo (♪✕).
  Barra de atalhos fixa no rodapé.
- **Resolução de alias para câmeras**: o CLI aceita `camN`, `name` ou
  `label` do config.toml em vez da URL completa. Ordem de resolução:
  nome > label (case-insensitive, ignorando espaços) > ordinal 1-based.
  Novo flag `--list` que lista todas as câmeras configuradas com
  senhas mascaradas.
- **Áudio por câmera (item 11)**: pipeline de áudio independente por
  câmera (rtspsrc → decodebin → audioconvert → volume → autoaudiosink).
  Controlado pelo TICK no modo grid ou pela sidebar + tecla `m` no modo
  single. Config `[audio]` em config.toml com campos `enabled` e
  `volume`.
- **Pipeline HLS**: câmeras com URL HTTP(S) abrem no modo `uridecodebin`
  automaticamente (detectado pelo schema). Grid de 1 célula para HLS.
- **Refinamento geral de UX/UI**: nova paleta de cores mais suave
  (`#4ADE80` / `#FACC15` / `#FB923C` / `#EF4444` / `#94A3B8`) — mais
  legível sobre fundos de vídeo brilhantes ou escuros do que os tons
  puros de 0xFF. Cor de seção dim (`#475569`) para os `─── HEADER ───`
  recuarem atrás dos valores.
- **Badge de saúde no header do overlay**: glyph entre colchetes
  (`[✓]`, `[·]`, `[!]`, `[‼]`, `[✕]`) colorido por severidade, dando
  o estado geral do stream em uma única posição. Derivado de
  `diagnostics::overall_severity`, que considera FPS, latência, jitter,
  perda de pacotes, erros de decoder e reconexões.
- **Dicas diagnósticas inline (`→ causa`)**: quando uma métrica está
  em estado de alerta, o overlay adiciona linhas `→ Loss: Wi-Fi/MTU
  — testar com cabo` etc. As dicas são ordenadas por severidade
  (pior primeiro) e limitadas a 3 no layout full / 2 no compact.
  Implementadas em `src/diagnostics.rs` com testes puros.
- **Módulo `diagnostics`** (novo, `src/diagnostics.rs`): funções puras
  `diagnose(metrics) -> Vec<Hint>` e `overall_severity(metrics) ->
  Severity` que olham para o snapshot de métricas e produzem (a)
  uma severidade global Healthy/Degraded/Warning/Critical/Stalled e
  (b) uma lista ordenada de `Hint { metric, severity, cause }`.
  Cobre FPS=0, latência > 5s, jitter > 20ms, perda > 1%, erros de
  decoder > 5, reconexões ≥ 3, etc. 11 unit tests.
- **Banner de startup ASCII**: novo módulo `src/banner.rs` com
  `render_banner(version, url, latency, cache, decoder, ...)` que
  imprime um cabeçalho com `╔═╗ rust-rtsp-viewer v0.1.0 ╚═╝` e o
  resumo da configuração resolvida. Substitui os 5 `println!`
  separados do `main.rs`.
- **Summary de sessão na saída**: ao encerrar (Ctrl+C), imprime
  uma tabela `┌──── session summary ────┐` com uptime, total de
  frames, FPS médio, erros (totais e de decoder) e reconexões.
  `render_session_summary` é pura e testável.
- **SIGUSR1 → dump de stats no log**: envie `kill -USR1 <pid>` e o
  app loga uma linha com FPS, Lat, Jit, Loss, Q, Drp, DecE, Rc, F e
  BR. Útil quando o app roda sob systemd/journald e o overlay não
  é visível. Implementado via `libc::signal` + `AtomicBool` global,
  consumido pelo loop principal. 5 unit tests em `banner::render_stats_dump`.
- **Logs coloridos com timestamp curto**: `env_logger` agora usa um
  format customizado com `HH:MM:SS`, cor por nível (ciano para INFO,
  amarelo para WARN, vermelho para ERROR, cinza para DEBUG) e
  alinhamento de 5 caracteres para o nome do nível.
- **Legenda inline no modo compact**: ao iniciar com `--compact-overlay`,
  uma linha de legenda com as abreviações (`Lat latency · Jit jitter
  · Lss loss · ...`) é impressa no log logo após o banner.
- **Atalhos de teclado (item 5)**: novo módulo `domain::keybindings`
  com `KeyAction` (TogglePause / TakeSnapshot / ToggleRecord /
  ToggleFullscreen / Quit) + trait `KeyHandler` + `KeyMap<H>` de
  keyval → action + `KeyHandlerError` tipado. Adaptador GTK
  (`infrastructure::gtk_keys::dispatch`) fecha o evento para
  keys bound e propaga unbound (`Tab` continua funcionando). 14
  unit tests. **Bindings**: `Space`/`k` → pause/resume, `F12`/`s`
  → snapshot (item 8 stub), `r` → record (item 9 stub), `F11` →
  fullscreen (item 10 stub), `q` → quit. Gateado por
  `AppConfig.enable_keybindings` (default `true`).
- **Detecção de cena noturna (item 6)**: novo `Hint::Night` no
  `diagnostics::diagnose`. Dispara quando `is_live &&
  last_avg_luma < 20/255` e há frames sendo decodificados
  (não confundir com câmera travada). Severidade `Degraded`.
  O texto da `cause` mostra a banda de luma (≤ 3, ≤ 8, ≤ 13,
  ≤ 17, ≤ 19) para o operador distinguir "muito escuro" de
  "quase escuro" sem sair do sidebar. Suprimido em offline
  e em stall. 6 unit tests novos + 5 em
  `metrics::snapshot_recent_avg_luma` para a freshness check.
- **Detecção de violação / tamper (item 7)**: novo
  `Hint::Tamper` em `diagnostics::diagnose`. Dois modos:
  **coberta** (luma ≤ 5/255 sustentado) e **ofuscada**
  (luma ≥ 250/255 sustentado). O discriminador de "violação
  real" é o `last_scene_change_unix_secs`: precisa estar
  estático por ≥ 30s (`TAMPER_STATIC_SECS`). Cenas de noite
  têm variação contínua (carros passando, galhos, troca de
  lâmpada) — elas nunca atingem 30s de variação zero, então
  não disparam o alarme. Suprimido offline. Severidade
  `Warning` (evento de segurança, mas o operador deve
  verificar — uma pedra na câmera pode voltar ao normal).
  8 unit tests novos. Constantes em
  `diagnostics::TAMPER_STATIC_SECS = 30`,
  `TAMPER_LUMA_LO = 5`, `TAMPER_LUMA_HI = 250`.
- **Snapshot (item 8)**: nova tecla F12 (ou `s`)
  que captura o frame atual e salva como PNG
  (`rust-rtsp-viewer-YYYY-MM-DD-HHMMSS-NNN.png`)
  num diretório configurável. Nova tabela
  `[snapshot]` em `config.toml` (campos `dir`,
  `quality`, `burst_count`). Pipeline ganhou um
  tee (`t_snap`) com 2 branches: a branch de
  display vai para `gtksink` (como antes); a
  branch de snapshot vai para um `appsink
  name=snapshot_sink max-buffers=1 drop=true`,
  que sempre entrega o frame mais recente.
  Notificação visual transient (3s, top-right)
  confirma o salvamento com nome do arquivo e
  tamanho. Dependências novas: `gstreamer-video
  0.20` (para `VideoInfo::from_caps`) e
  `image 0.25` com feature só `png` (encoder
  PNG pure-Rust, evita o gdk-pixbuf que não
  tem `from_bytes`). Total: 27 testes novos em
  `domain::snapshot`, 4 no writer, 4 de
  pipeline. Constantes: `BURST_INTERVAL_MS = 100`
  entre frames de burst.
- **Recording (item 9)**: nova tecla `r` que
  inicia/para a gravação do stream num arquivo
  MKV (ou MP4). Pipeline ganhou uma 3ª tee branch
  (recording): `queue → matroskamux (ou mp4mux) →
  filesink name=recording_sink`. O elemento
  `filesink` é controlado dinamicamente — em
  `idle` está em NULL; ao iniciar, o
  `RecordingSession` seta a propriedade `location`
  e a transição para PLAYING; ao parar, seta de
  volta para NULL para finalizar o arquivo
  (escreve o trailer do muxer). Nova tabela
  `[recording]` em `config.toml` (campos
  `dir`, `max_segment_duration_secs`,
  `max_segment_size_bytes`, `container`). Sidebar
  mostra `● REC  HH:MM:SS  segment NNN` na seção
  STATS quando gravando. Notificação visual
  transient (3s) confirma start/stop. Total: 32
  testes novos em `domain::recording`, 4 no
  `infrastructure::recording`, 1 de pipeline.
  Constantes: `DEFAULT_MAX_SEGMENT_DURATION_SECS =
  600`, `DEFAULT_MAX_SEGMENT_SIZE_BYTES = 1 GiB`.
- **Fullscreen (item 10)**: nova tecla F11 que
  alterna a janela principal entre modo
  windowed e fullscreen. Implementado via
  `GtkWindowController` (adapter GTK3 para o
  trait `WindowController` em
  `domain::keybindings`). O controller é um
  `Rc<RefCell<WeakRef<gtk::Window>>>` — o
  `attach()` é chamado em `connect_activate`
  (após a janela ser construída), e o `apply()`
  é chamado pelo tick loop quando a ação
  `ToggleFullscreen` é drenada da fila de
  keypresses. O domínio já tinha
  `WindowState` + `WindowController` + 4 testes
  do item 10 (stubs); este item 10 completa a
  infra (`gtk::Window::fullscreen()` /
  `unfullscreen()`) e adiciona 4 testes novos
  cobrindo: estado inicial `Windowed`,
  `apply()` sem janela attached (track
  state only), `apply()` idempotente (mesmo
  estado), e toggle round-trip.

### 🐛 Corrigido

- **Argumento `--cache`/`RTSP_CACHE_SECONDS`/`cache_seconds` ignorado**: o

- **Pipeline com `uridecodebin`**: novo modo opcional `uridecodebin` (single-
  element URI + demux + decode) como alternativa à cadeia clássica
  `rtspsrc → <decoder>`. Ativa via `--uridecodebin` / `--no-uridecodebin`,
  `RTSP_USE_URIDECODEBIN=true` ou `use_uridecodebin = true` no TOML. Útil
  para câmeras com múltiplos streams (áudio+vídeo) ou SDPs com
  particularidades que confundem o autoplugging do `decodebin`. Quando
  ativo, a opção `decoder` é ignorada (o `uridecodebin` escolhe sozinho).
  Inclui preflight que valida a presença do elemento no registry e
  teste de regressão.
- **Overlay reescrito com layout categorizado (NETWORK / BUFFER / DECODE
  / IMAGE / STATS)** e duas variantes:
  - `--overlay` (padrão): layout **full** com seções marcadas por
    `─── HEADER ───`, ~14-18 linhas, ideal para diagnóstico detalhado.
  - `--compact-overlay`: layout **compact** denso, multi-coluna, ~7
    linhas, ideal para janelas pequenas. Selecionável também por
    `RTSP_COMPACT_OVERLAY=true` ou `compact_overlay = true` no TOML.
- **Novas métricas de overlay**:
  - `Jit: <ms>` — jitter EMA (variação de inter-arrival vs período
    esperado de frame), colorido verde/amarelo/vermelho.
  - `Loss: <%>` ou `N/A` — perda de pacotes agregada dos
    `rtpjitterbuffer` (obtida via signal `rtspsrc::new-jitterbuffer`
    e polled a cada 2s). Mostra `N/A` em cinza quando stats
    indisponíveis (RTP por UDP, ou rtspsrc ainda não expôs).
  - `Dec: <ms>` — tempo de decodificação EMA, calculado casando PTS
    entre probe pré-decodificação e pós-decodificação.
  - `Drp: <n>` — frames dropados no decoder (`drop-frame` na
    `videodecodebin3`).
  - `DecE: <n>` — contagem de mensagens `GST_MESSAGE_ERROR` /
    `WARNING` originadas de elementos de decoder (filtradas por
    factory name contendo "dec" ou pelo `--decoder` configurado).
  - `Rc: <n> (<duração>)` — número de reconexões + duração da
    última reconexão (`Playing → non-Playing → Playing`).
  - `Lum: <avg>/σ<stddev>` — luma média e desvio padrão da imagem
    (subamostrado em grid 8×8), colorido por brilho e
    representatividade da cena.
  - `Stale: <tempo>` — tempo desde a última mudança de cena
    detectada (stddev subindo = cena em movimento).
- **Módulo `image_quality`**: funções puras para cálculo de luma
  (`average_luma_subsampled`), desvio padrão (`stddev_luma_subsampled`)
  e detecção de cena estática (`is_static_sample`). 6 unit tests
  cobrindo subamostragem, bordas, padding e thresholds.
- **Detecção de formato YUV**: helper `is_y_first_format()` que cobre
  I420/YV12/I422/Y42B/I444/Y444/NV12/NV21/GRAY8/GRAY16. Formatos que
  não expõem Y em primeiro plano (RGB, BGR) são silenciosamente
  pulados pelo sampler para evitar leitura errada.
- **Latency real**: `Lat: <ms>` agora usa `gst::query::Latency` real
  no pipeline em vez de `pipeline.latency()` (que retornava o
  *latency configurado*, sempre `GST_CLOCK_TIME_NONE` já que nunca
  chamamos `set_latency()`, fazendo o overlay mostrar "Lat:
  measuring…" para sempre).
- **Q com histerese**: a cor de `Q` (fila do decoder) agora aplica
  histerese de 3 ticks consecutivos em nível vermelho, evitando
  falsos positivos em quedas momentâneas. Quando o stream não está
  `live`, `Q` é mostrado em cinza (a fila é meaningless sem fluxo).
- **Refinamento de cores do `Q`**: faixas atualizadas — verde
  (≥75%), amarelo (50-74% e 86-100% warmup), vermelho com histerese
  (<50% por ≥3 ticks).

### 🐛 Corrigido

- **"Lat: measuring…" travado**: a métrica de latência nunca saía
  do estado "measuring" porque o código lia `pipeline.latency()`,
  que em gstreamer-rs retorna o *latency configurado* via
  `gst_pipeline_set_latency()` — que nunca é chamado. Substituído
  por `gst::query::Latency` real enviado ao pipeline.
- **Q ficando vermelho sem motivo**: o limite vermelho de `Q`
  disparava em qualquer queda momentânea da fila (ex.: uma
  correção de QoS). Adicionada histerese de 3 ticks e cinza
  quando `!live` (evita ruído visual durante reconexão).
- **Argumento `--cache`/`RTSP_CACHE_SECONDS`/`cache_seconds` ignorado**: o
  pipeline sempre iniciava em 5s independente do valor passado pelo usuário.
  Agora o valor configurado é usado como **cache inicial** e como **reset**
  após erros isolados.
- **Inconsistência entre range documentado (2-5s) e range real (5-10s)**:
  todo o código, documentação e overlay foram realinhados para a faixa
  dinâmica **[2s, 10s]**.
- **Thresholds de cor do overlay calibrados para 5-10** (verde ≤6, amarelo
  7-8, vermelho ≥9). Recalibrados para a nova faixa 2-10 (verde ≤5,
  amarelo 6-7, vermelho ≥8).
- **`Q:` (queue fill) calculado em buffers, não em tempo** — apesar do
  CHANGELOG e comentários dizerem `current-level-time / max-size-time`, a
  implementação usava `current-level-buffers / max-size-buffers`. Agora
  o Q% está em **nanossegundos / nanossegundos**, mesma unidade do
  `Cch: Ns`, então passam a ser diretamente comparáveis.
- **State-change "Stream stable, reducing cache" respeitava `min_cache`
  (2) em vez de `initial_cache`** — quando o pipeline ia para Playing,
  o handler sempre reduzia o cache em 1s, mesmo que o valor atual já
  fosse o que o usuário configurou (ex.: config=5s, virava 4s sem
  motivo). Agora só reduz se `current > initial_cache`, e a mensagem
  de log mostra de onde vem e para onde vai.

### ✨ Adicionado (continuação)

- **Latência real (não aproximada)**: o overlay agora exibe a latência
  end-to-end medida em runtime via `Pipeline::latency()` (inclui
  rtspsrc + fila + decoder). Sem `~` no prefixo. Antes da primeira
  medição, exibe `Lat: measuring…`.
- **Indicador de crescimento do cache** (`Cch: 6s ↑1`): sempre que
  o cache atual está acima do `initial_cache` configurado, mostra a
  seta e o delta em segundos — fica explícito quando a lógica
  dinâmica precisou crescer o buffer.

### 🎬 Side panel GTK3 (substitui o compositor anterior)

- **Substituição do compositor+side-bin pelo `gtksink` + `GtkTextView`**.
  O `compositor ! videoscale ! capsfilter ! autovideosink` que tentava
  fixar a janela em 1664×720 (split-screen) foi **abandonado**:
  `xvimagesink` ignorava o canvas do compositor e a abordagem ficou
  flaky quando `videoscale+capsfilter` foi introduzida. A nova
  implementação:
  - Pipeline termina com `gtksink name=video_sink sync=false` (já
    disponível no `gstreamer1.0-gtk3`, instalado por padrão).
  - A janela GTK3 (`gtk::Application` + `gtk::Window`) hospeda um
    `gtk::Box(horizontal)` com o widget de vídeo (embeddable via
    `gst::glib::Object` property) à esquerda e um `gtk::TextView`
    (read-only, monospace, ~360px) à direita.
  - `gtk_widget_unparent` no `connect_activate` desacopla o widget
    da janela interna do `gtksink` antes de re-embedar (evita o
    `Gtk-CRITICAL: gtk_box_pack: _gtk_widget_get_parent (child) == NULL`).
  - Loop principal GTK3 (`gtk::glib::timeout_add_local` + bus
    watch com `add_watch_local`) substitui o `while let` legado;
    ambos os callbacks compartilham `Arc<Mutex<LoopState>>`.
- **Stack atualizado**: `gtk = "0.18"` + `glib = "0.18"` (em vez de
  `gtk4 = "0.11"` + `glib = "0.20"`, que não tinha `gtk4paintablesink`
  empacotado no Ubuntu 24.04). `gst::glib` (0.17, re-exportado pelo
  gstreamer 0.20) coexiste com `gtk::glib` (0.18) — a primeira é
  usada pelo `add_watch_local` (closure retorna `gst::glib::source::Continue(true)`),
  a segunda pelo `timeout_add_local` (closure retorna `glib::ControlFlow::Continue/Break`).
- **Dependência de sistema nova**: `libgtk-3-dev` + `pkg-config`
  (apt). O `gstreamer1.0-gtk3` (com `gtksink`/`gtkglsink`) já vinha
  instalado.
- **Indicador de instabilidade** no `●`:
  - verde = live, sem issues
  - laranja = live, com issues (dropped > 0, reconnects > 0,
    cache > initial, ou last_error definido)
  - cinza = não-live (paused/buffering/erro fatal)
- **Teto do cache dinâmico subido de 10s → 30s**, dando folga real
  em redes instáveis sem congelar a reprodução.

### 🎨 Ajustado

- **Overlay flush à esquerda**: removido o `xpad=6` do `textoverlay`
  (mantido `ypad=6` para respiro vertical). Agora a primeira coluna
  do overlay encosta na borda esquerda do vídeo, sem 6px de
  margem.
- **Intervalo de atualização do overlay**: 500ms → **1000ms**.
  Reduz overhead de renderização e mantém a sensação de "tempo real"
  sem flicker. Bitrate, FPS, latência real e demais métricas
  continuam sendo lidas a cada ciclo do bus loop (mais frequente
  que o display, só a pintura do texto-over é que vai a 1s).
- **Range dinâmico do cache ampliado**: `[2, 30]s` → **`[0, 60]s`**.
  - `0s` desabilita o buffer pré-decoder (mínima latência
    possível, ao custo de stutter em redes ruins).
  - `60s` cobre desde redes instáveis até cenários extremos de
    reconexão.
- **Handler de SIGINT agora é idempotente**: usa `compare_exchange`
  atômico para garantir que apenas o primeiro Ctrl+C dispara os
  logs e o `set_state(Null)`. Pressionar Ctrl+C várias vezes
  (intencionalmente ou por repique do terminal) não gera mais
  linhas duplicadas "Received interrupt signal" / "set to Null
  state".
- **Diagnóstico de decoders no startup** (`src/diagnostics.rs`):
  percorre o registry do GStreamer logo após `gst::init()` e
  reporta os decoders H264/H265 disponíveis. Se nenhum decoder
  H264 estiver presente, emite um aviso com os comandos de
  instalação (`gstreamer1.0-libav`, `gstreamer1.0-plugins-bad`,
  `gstreamer1.0-vaapi`) e uma linha em `eprintln!` para o
  usuário ver mesmo sem `RUST_LOG=info`.
- **Detecção específica de `not-linked`**: o handler de `Error`
  do bus loop agora inspeciona o debug string por `not-linked`
  e, quando encontra, emite um warning direcionado com a
  hipótese de decoder ausente e o comando de instalação
  correspondente. O `last_error` (mostrado no overlay) também
  passa a ser `"No decoder for stream codec (not-linked)"` em
  vez do genérico `"Internal data stream error."`, deixando
  claro o motivo no vídeo.
- **Decoder configurável** via CLI/env/TOML:
  - CLI: `--decoder <name>` (ex.: `decodebin`, `decodebin3`,
    `avdec_h264`).
  - Env: `RTSP_DECODER=<name>`.
  - TOML: `decoder = "<name>"` em `config.toml`.
  - Default: `decodebin` (mais estável que `decodebin3`; veja
    `BUGFIX_DECODEBIN3.md` para a motivação). `decodebin3` ainda
    é suportado, basta passar `--decoder decodebin3` ou
    `RTSP_DECODER=decodebin3`.
  - Validação fail-fast: se o decoder não existir no registry
    do GStreamer, o app aborta com mensagem clara
    (`Decoder '<name>' not found in GStreamer plugin registry`)
    em vez de erro críptico de `parse_launch`.
- **Default do cache reduzido para 0s**: o `cache_seconds` padrão
  (CLI/env/TOML) agora é `0` em vez de `3`. O sistema começa sem
  buffer pré-decoder e cresce automaticamente na primeira
  evidência de instabilidade (erro, QoS drop, Q acima do target).
  Quem quiser um piso explícito (ex.: `cache_seconds = 2` para
  sempre ter pelo menos 2s) pode setar no config.
- **Target do Q% ajustado para 80%** (cache "sempre cheio"):
  a lógica proativa agora mantém a queue em ~80% de
  preenchimento com dead zone de ±5% (75-85%):
  - Q > 85% → cache cresce 1s (queue quase estourando)
  - Q < 75% → cache encolhe 1s (cache subutilizado)
  - 75-85% → dead zone, sem ação
  - Cores do overlay recalibradas: verde em 75-85%, amarelo
    em 50-75% e 85-100%, vermelho em <50% (cache
    subutilizado) ou >100% (estouro).
- **Fase de warmup adicionada**: ao iniciar, o sistema espera
  a queue encher até 80% do cache (ou 5s de timeout) antes
  de aplicar o controlador dinâmico. Isso dá tempo do buffer
  acumular dados reais antes do Q-target entrar em ação.
  - Log no fim do warmup: `Warmup complete: queue reached
    X% of cache (Ns) after Ys`.
  - Comportamento detalhado:
    1. Warmup: cache fica no `initial_cache` (não encolhe),
       espera Q ≥ 80% (queue enchendo com dados reais).
    2. Pós-warmup: controlador 80% alvo entra em ação
       (Q > 85% cresce, Q < 75% encolhe).
- **Bug no parser de args posicionais**: o valor consumido por
  `--decoder <name>` estava sendo interpretado como URL porque
  o filtro só removia args começando com `--`. Agora flags que
  recebem valor (`--decoder`) também descartam o argumento
  seguinte.
- **Reset agressivo para `min_cache` no primeiro erro**: o reset voltava
  para o piso (que era 5s) e ignorava a preferência do usuário. Agora
  reseta para o `cache_seconds` configurado.

### ✨ Adicionado

- **Cache proativo (não apenas reativo)**:
  - Em **QoS drops** (frames dropados pelo sink) → `cache +1s`
    (throttle 1s para evitar saltos).
  - Quando a **queue fill ≥ 80%** → `cache +1s` (sink/redes gargalando).
  - Quando a **queue fill < 20%** → `cache -1s` (cache sobredimensionado,
    devolve folga).
- Novo campo `initial_cache` em `AppConfig` para carregar o valor
  configurado pelo usuário até o bus loop.

### ❌ Removido

- **Overlay de texto/relógio sobre o vídeo**: removido o caminho
  legado que renderizava o overlay diretamente em cima do vídeo
  com `textoverlay`/`clockoverlay`. As estatísticas são renderizadas
  **somente** no painel lateral GTK3 (sempre presente, ~360px de
  largura), portanto o vídeo nunca é oclusado. Mudanças:
  - **Pipeline**: removidos os elementos `clockoverlay` e
    `textoverlay`; a saída agora é `gtksink` (ou um
    `videoscale → capsfilter → gtksink` quando `enable_fit_window`).
  - **Enum `OverlayPosition` removido** (`src/overlay.rs`): as
    variantes `Over` e `Side` e os seus métodos
    (`as_str`/`FromStr`/`Default`) deixam de existir — só há um
    layout (sidebar).
  - **Flags CLI removidos**: `--overlay`, `--no-overlay`, `--clock`,
    `--no-clock`, `--overlay-position`. Permanece
    `--compact-overlay`/`--no-compact-overlay` para escolher o
    estilo (full vs compact) do **sidebar**.
  - **Variáveis de ambiente removidas**: `RTSP_ENABLE_OVERLAY`,
    `RTSP_ENABLE_CLOCK`, `RTSP_OVERLAY_POSITION`.
  - **Campos de configuração removidos** (`config.toml`): as chaves
    `enable_overlay`, `enable_clock`, `overlay_position` são
    ignoradas (e o `config.toml.example` deixa de documentá-las).
  - **Campos `AppConfig` removidos**: `enable_overlay: bool` e
    `overlay_position: OverlayPosition` deixam de existir.
  - **Campo `App.text_overlay` removido**: o `Option<gst::Element>`
    que era `pipeline.by_name("textoverlay0")` foi removido, junto
    com a resolução que dependia de `OverlayPosition`.
  - **Sidebar sempre presente**: `App.sidebar` agora é
    `gtk::TextView` (não mais `Option<gtk::TextView>`). O
    `connect_activate` constrói o `GtkScrolledWindow` que envolve o
    `GtkTextView` incondicionalmente; em `update_overlay`, o
    `set_text` roda em todo tick.
  - **Quebrando para quem usava `Over` mode**: o vídeo não recebe
    mais texto sobreposto. Para inspeção de métricas em telão,
    prefira o sidebar em janela maximizada; para overlays
    compactos, use `--compact-overlay` (mantido).
  - **Tests**: novo teste de regressão
    `pipeline_never_paints_text_on_video` em
    `src/pipeline.rs` valida que a string do pipeline nunca
    contém `textoverlay` nem `clockoverlay`. Testes que
    dependiam de `OverlayPosition` foram removidos /
    atualizados.

## [0.3.0] - 2026-06-04

### 🚀 Overlay totalmente dinâmico

Todos os campos do overlay agora são lidos em tempo real do pipeline
GStreamer através de probes, queries e mensagens do bus.

#### Adicionado

- **Resolução real do stream** (ex.: `1920×1080`) via probe de caps no
  `predec_queue` src pad.
- **Codec detectado** (ex.: `H264`, `H265`) via `encoding-name` no RTP caps.
- **Framerate** lido da fração GStreamer (`30fps`, `29.97fps`, etc.).
- **Bitrate real** (kbps) calculado a partir de bytes contados na probe
  da queue, com janela móvel de 500ms.
- **Fill level da queue** (%) lido de `current-level-time` / `max-size-time`.
- **Uptime** (`HH:MM:SS`) desde o início do streaming.
- **Host:porta** parseado da URL (sem credenciais).
- **Protocolo de transporte** (TCP/UDP) exibido a partir do pipeline.
- **Frames dropados** contabilizados via `MessageView::Qos` (GStreamer QoS).
- **Contador de reconexões** detectado em transições `Playing → *`.
- **Último erro** exibido quando o pipeline entra em estado de falha.
- **Cor dinâmica do cache** (verde ≤6s, amarelo 7-8s, vermelho ≥9s).
- **Realce** de `Dropped` e `Recon` em laranja quando ≠ 0.
- **Flag `--clock` / `--no-clock`** + `RTSP_ENABLE_CLOCK` + `enable_clock`
  no `config.toml` para reativar o `clockoverlay` opcional.

#### Refatoração

- Código monolítico de 754 linhas em `main.rs` quebrado em 5 módulos:
  - `src/main.rs` (orquestração, ~190 linhas)
  - `src/app.rs` (struct `App` + bus loop + probe installation)
  - `src/config.rs` (`Config` + `from_file`)
  - `src/pipeline.rs` (`build_pipeline_string`)
  - `src/metrics.rs` (`Metrics` thread-safe + `StreamInfo`)
  - `src/overlay.rs` (`OverlayState` + `build_overlay_text` puro + 8 testes)
- Flags CLI agora são filtradas (não são mais interpretadas como URL).
- Bugfix: `Qos::stats()` retorna `GenericFormattedValue` (não `u64` direto).
- Bugfix: `current-level-time` é `guint64` (não `gint64`).

### 🧪 Testes

- 8 testes unitários em `overlay::tests` cobrindo formatação de uptime,
  truncamento e snapshots completos do overlay (LIVE/OFFLINE, com/sem codec,
  resolução parcial, etc).

## [0.2.0] - 2026-05-26

### 🚀 Adicionado

- **Configuração flexível via múltiplas fontes**
  - Suporte a argumentos de linha de comando
  - Suporte a variáveis de ambiente (`RTSP_URL`, `RTSP_LATENCY_MS`)
  - Suporte a arquivo de configuração TOML (`config.toml`)
  - Sistema de prioridade: CLI > Env Var > Config File > Defaults

- **Logging avançado**
  - Integração com crate `env_logger`
  - Níveis de logging configuráveis (debug, info, warn, error)
  - Controle via variável de ambiente `RUST_LOG`
  - Logs detalhados para debugging

- **Interrupção graciosa**
  - Handler para sinal Ctrl+C (SIGINT)
  - Shutdown seguro do pipeline GStreamer
  - Mensagens informativas ao interromper

- **Validação de URL**
  - Validação de formato de URL RTSP
  - Suporte a esquemas `rtsp://` e `rtsps://`
  - Verificação de host obrigatório
  - Mensagens de erro claras

- **Latência configurável**
  - Parâmetro de latência via linha de comando
  - Variável de ambiente `RTSP_LATENCY_MS`
  - Configuração no arquivo TOML
  - Valor padrão: 100ms

- **Sistema de Cache (2-5 segundos)**
  - Bufferização automática do stream
  - Pipeline com elemento `queue` para buffering
  - Configurável via CLI, env var ou config file
  - Validação automática (clamp 2-5s)
  - Valor padrão: 3 segundos
  - Melhora estabilidade em redes instáveis
  - Reduz travamentos e buffering

- **Overlay de Informações**
  - Clock em tempo real no vídeo (AAAA-MM-DD HH:MM:SS)
  - Informações do stream (latência, cache, modo)
  - Configurável: ativar/desativar
  - Fundo sombreado para legibilidade
  - Fonte monoespaçada profissional
  - Elementos: `clockoverlay` + `textoverlay`

- **Documentação completa**
  - README.md detalhado com instruções de uso
  - Exemplos de configuração
  - Guia de instalação para múltiplas plataformas
  - Tabela de dependências
  - Seção de segurança

- **Scripts de suporte**
  - `install-deps.sh`: Instalação automática de dependências
  - `check-system.sh`: Verificação de dependências instaladas
  - Suporte a Debian/Ubuntu, Arch Linux e macOS

- **Makefile completo**
  - 30+ comandos para facilitar desenvolvimento
  - Build, run, test, lint, doc e deployment
  - Cores e output formatado
  - Alvos para CI/CD e release
  - Watch mode para desenvolvimento contínuo

- **Arquivos de exemplo**
  - `config.toml.example`: Exemplo de arquivo de configuração
  - `.env.example`: Exemplo de variáveis de ambiente

### 🔒 Segurança

- **Remoção de credenciais hardcoded**
  - Removida URL com credenciais do código fonte
  - URL padrão genérica: `rtsp://127.0.0.1:554/stream`
  - Uso recomendado de variáveis de ambiente ou config file

- **Proteção de arquivos sensíveis**
  - `.gitignore` atualizado para excluir `config.toml` e `.env`
  - Prevenção de commit acidental de credenciais

### 🛠️ Melhorias Técnicas

- **Novas dependências**
  - `log = "0.4"`: Facade de logging
  - `env_logger = "0.10"`: Logger baseado em ambiente
  - `ctrlc = "3.4"`: Captura de sinais Ctrl+C
  - `url = "2.5"`: Validação e parsing de URLs
  - `toml = "0.8"`: Parsing de arquivos TOML
  - `serde = "1.0"`: Serialização/deserialização

- **Melhor tratamento de erros**
  - Mensagens de erro mais descritivas
  - Logging de erros com contexto
  - Validação prévia de parâmetros

- **Código mais limpo**
  - Remoção de variáveis globais não utilizadas
  - Melhor organização do código
  - Comentários e documentação inline

### 📝 Documentação

- README.md completo com:
  - Visão geral do projeto
  - Instruções de instalação detalhadas
  - Exemplos de uso
  - Tabela de configuração
  - Arquitetura do pipeline
  - Seção de segurança
  - Melhorias futuras
  - Agradecimentos

### 📊 Estatísticas

- **Linhas de código**: 86 → 305 (+255%)
- **Arquivos novos**: 12
  - README.md
  - QUICKSTART.md
  - MAKEFILE_GUIDE.md
  - MAKEFILE_EXAMPLES.md
  - CACHE_GUIDE.md
  - CACHE_QUICK_REF.md
  - OVERLAY_GUIDE.md
  - TROUBLESHOOTING.md
  - OPTIMIZATIONS.md
  - PROJECT_STRUCTURE.md
  - CHANGELOG.md
  - BUGFIX_OVERFLOW.md
  - Makefile
  - config.toml.example
  - .env.example
  - install-deps.sh
  - check-system.sh
  - test-stream.sh
- **Dependências adicionadas**: 6 crates
- **Scripts utilitários**: 3
- **Makefile**: 360+ linhas com 30+ comandos
- **Documentação total**: 2,600+ linhas

---

## [0.1.0] - 2026-05-26

### Adicionado

- Visualizador básico de streams RTSP
- Pipeline GStreamer: `rtspsrc → decodebin → autovideosink`
- Suporte a URL via linha de comando
- Tratamento básico de erros
- Monitoramento de estado do pipeline
- Cleanup gracioso ao encerrar

---

## Bugfixes — Post-Mortem Resumido

Esta seção consolida os antigos `BUGFIX_*.md` (removidos) em uma tabela
única para referência histórica. Cada entrada cita o arquivo original,
a causa raiz e a correção aplicada.

| Arquivo | Sintoma | Causa Raiz | Correção |
|---|---|---|---|
| `BUGFIX_DECODEBIN3.md` | `Delayed linking failed` / `not-linked (-1)` ao conectar RTSP | `decodebin` legado falha em linking dinâmico com H.264/H.265 de câmeras IP modernas | Substituído por `decodebin3` no pipeline |
| `BUGFIX_GRACEFUL_SHUTDOWN.md` | Ctrl+C não desligava o viewer | Bus loop bloqueava em `iter_timed` e só checava `running` quando chegava mensagem | Trocado para wait assíncrono via channel que acorda no sinal |
| `BUGFIX_OVERFLOW.md` | `cache_seconds * 1_000_000_000` overflow acima de ~4.29s | Expressão inferida como `u32` (32 bits não cabem nanossegundos) | Cast explícito para `u64` na conversão para nanossegundos |
| `BUGFIX_OVERLAY_PROPERTIES.md` | Pipeline falhava ao criar overlays | Propriedade inexistente `valign` no `clockoverlay`/`textoverlay` | Renomeado para a propriedade correta `valignment` |
| `BUGFIX_OVERLAY_UPDATE_AND_FREEZING.md` | Overlay travava e vídeo engasgava em stream estável | Atualização do overlay atrelada ao bus loop; sem mensagens, ficava obsoleta | Desacoplada: overlay atualiza em timer próprio; pipeline rebalanceado |
| `BUGFIX_WINDOW_CLOSED_AND_WARNINGS.md` | Warning de variável não usada + auto-reconectar em janela fechada | `start_time` não usado; lógica tratava "Output window was closed" como erro de stream | Removida a variável; special-case para fechar imediatamente em vez de retunar o cache |
