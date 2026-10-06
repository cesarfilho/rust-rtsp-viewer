# rust-rtsp-viewer

[![Licença: AGPL-3.0-or-later](https://img.shields.io/badge/licen%C3%A7a-AGPL--3.0--or--later-blue.svg)](LICENSE)

Visualizador de streams RTSP/HLS construído em Rust com **Iced** + **GStreamer**. Suporta grade multi-câmera, painel lateral de métricas ao vivo, reconexão automática com backoff, gravação por segmentos, snapshots e áudio por câmera, **detecção de movimento com zonas poligonais**, gravação e **notificações de desktop disparadas por movimento**, e cinco temas com contraste verificado por teste.

Plataforma-alvo: **Linux** (Wayland/X11; integração com COSMIC/GNOME).

---

## Instalação

### Pré-requisitos

- Rust 1.92+ (edition 2024)
- GStreamer 1.20+ com os seguintes plugins:

```bash
# Ubuntu / Debian
sudo apt install \
  libgstreamer1.0-dev \
  libgstreamer-plugins-base1.0-dev \
  gstreamer1.0-tools \
  gstreamer1.0-plugins-base \
  gstreamer1.0-plugins-good \
  gstreamer1.0-plugins-bad \
  gstreamer1.0-plugins-ugly \
  gstreamer1.0-libav

# Arch Linux
sudo pacman -S gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-libav

# macOS
brew install gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-libav
```

### Compilar

```bash
cargo build --release
```

O binário resultante é `target/release/rust-rtsp-viewer`.

### Integração com o desktop (Wayland / COSMIC / GNOME)

```bash
make install     # instala em ~/.local (sem root)
make uninstall
```

`make install` compila em release e instala o binário, o `.desktop`
(`assets/rust-rtsp-viewer.desktop`) e o ícone SVG. A janela declara o
`application_id` `rust-rtsp-viewer`, que casa com o nome do `.desktop` e com
o `StartupWMClass` — é isso que permite ao COSMIC mostrar o nome e o ícone
corretos na dock, na visão geral e no alternador de janelas. Prefixo do
sistema: `make install PREFIX=/usr/local` (aí sim como root).

---

## Uso

### Linha de comando

```bash
# Usar config.toml padrão (current directory)
./target/release/rust-rtsp-viewer

# Especificar arquivo de configuração
./target/release/rust-rtsp-viewer /caminho/para/config.toml
```

O Iced viewer lê toda a configuração do `config.toml`. Tudo é configurado via arquivo; a única flag é `--check`, que valida o `config.toml` (chaves com erro de digitação, valores fora de faixa, URLs, grupos) e sai sem abrir janela. O buffering do stream é gerenciado pelo GStreamer; o único ajuste exposto é `latency_ms` (→ `rtspsrc latency`).

### Arquivo de configuração (config.toml)

Crie `config.toml` no diretório de execução:

```toml
# Configurações globais (opcional — usadas como fallback)
latency_ms   = 100
decoder      = "decodebin"

# Câmeras — cada [[cameras]] é uma entrada na grade
[[cameras]]
url   = "rtsp://192.168.1.101:554/stream"
label = "Entrada"

[[cameras]]
url          = "rtsp://192.168.1.102:554/stream"
label        = "Garagem"
audio_volume = 0.8           # habilita áudio por clique
latency_ms   = 200           # substitui o global

[[cameras]]
url           = "rtsp://192.168.1.103:554/stream"
label         = "Quintal"

[[cameras]]
url   = "https://exemplo.com/stream/9980ca324f48.m3u8"
label = "Externa (HLS)"

[snapshot]
dir         = "~/Pictures/rust-rtsp-viewer"
quality     = 92
burst_count = 1

[recording]
dir                     = "~/Videos/rust-rtsp-viewer"
max_segment_duration_secs = 600
max_segment_size_bytes    = 1073741824
container               = "mkv"

[audio]
enabled = true
volume  = 0.8

# Grava sozinho enquanto há movimento (e 15 s depois do último)
# on_motion / motion_post_roll_secs ficam dentro de [recording]

[notifications]          # notify-send (libnotify)
enabled       = true
cooldown_secs = 60       # no mínimo 5

[motion]                 # ajuste fino do detector (todos opcionais)
threshold     = 25       # diferença de luma por pixel (1–255)
contour_area  = 0.005    # fração mudada que declara movimento
sample_stride = 8        # 1 pixel a cada N (1–32)
lightning_threshold = 0.8  # mudança do quadro todo (IR/exposição/PTZ) não é movimento; 0 desliga
```

`config.toml.example` traz todas as opções comentadas. **Não versione o seu
`config.toml`**: as URLs RTSP costumam conter a senha da câmera (veja
[SECURITY.md](SECURITY.md)).

### Campos por câmera

| Campo | Obrigatório | Descrição |
|-------|:-----------:|-----------|
| `url` | sim | URL `rtsp://`, `rtsps://`, `http://` ou `https://` (HLS `.m3u8`) |
| `name` | não | Apelido curto para referência interna |
| `label` | não | Rótulo no sidebar |
| `latency_ms` | não | `rtspsrc latency` — único ajuste de buffering (padrão: 100) |
| `decoder` | não | Decoder GStreamer (herda o global; padrão `decodebin`) |
| `audio_volume` | não | Habilita áudio por clique (0.0–1.0) |
| `use_uridecodebin` | não | Força `uridecodebin`. Auto-detectado para `http(s)://` |
| `do_retransmission` | não | Retransmissão RTP (herda o global; padrão `true`) |

### Modos de layout

| Modo | Descrição |
|------|-----------|
| **Grid** | Grade de câmeras — densidade automática ou fixa (2×2, 3×3, 4×4) |
| **Flex** | Uma câmera principal + faixa de miniaturas |

Alterne entre os modos com `Tab`.

Na grade, **clique** em uma célula para selecionar a câmera, **duplo-clique**
(ou `f` / `Enter`) para abrir o *spotlight*, e **clique direito** para o menu de
ações. Ao selecionar ou passar o mouse numa célula, uma fileira de ícones
(`◉ ● ♪ ▣`: snapshot, gravar, áudio, spotlight) aparece no canto — as ações antes
ficavam numa barra acima da grade que roubava altura de vídeo. No *spotlight* a
barra inferior mostra todas as funções da câmera (snapshot, gravar, áudio e
zonas de movimento), com tooltip e atalho, e destaca o que está ligado.

Os ícones usam a fonte **DejaVu Sans embutida** no binário (`assets/fonts/`),
então não dependem das fontes instaladas no sistema.

### Maximização de vídeo

A interface tem três níveis de "menos cromagem, mais vídeo":

| Modo | Como | O que faz |
|------|------|-----------|
| **Imersivo** | tecla `h` (ou `⋯ ▸ Modo imersivo`) | Esconde toolbar e sidebar; a grade vai de borda a borda. Encoste o cursor no topo para revelar um trilho flutuante com os controles; ele some sozinho. `Esc` volta. |
| **Spotlight** | duplo-clique numa célula, `f` ou `Enter` | Uma câmera ocupa a tela inteira. `←` / `→` trocam de câmera; `Esc` ou `✕` voltam à grade. |
| **Tela cheia** | `F11` | Tela cheia do sistema **e** imersivo ao mesmo tempo. |

A barra de status inferior foi removida; a saúde das câmeras (um medidor
segmentado, uma faixinha por câmera colorida pelo estado), a página e o
carrossel agora vivem na própria toolbar.

**Menu de comandos** — o botão `⋯` da toolbar e o **clique-direito** numa
célula abrem a mesma superfície, com seções (Exibição / Câmera / Aparência),
ícone à esquerda e o atalho num chip à direita. O clique-direito aparece onde
o cursor está. Densidade e tema são segmentos inline (sem menus suspensos
padrão). Na sidebar, o `⋯` que surge ao passar o mouse numa câmera abre o mesmo
menu (com Ativar/Desativar).

**Inspetor** (aba da sidebar) — cabeçalho + cards *Stream* e *Rede* com os
números que importam (fps com mini-gráfico, latência/jitter/perda coloridos por
limiar) e os diagnósticos no topo; o resto dos contadores fica dobrado em
"Avançado".

### Densidade, paginação e carrossel (modo Grid)

A toolbar (e os atalhos) controlam como as câmeras são apresentadas:

- **Densidade** — `Auto` coloca todas as câmeras numa única página com células
  o mais quadradas possível; `2×2` / `3×3` / `4×4` fixam a grade. Tecla `g`
  cicla entre elas.
- **Paginação** — com uma grade fixa, quando há mais câmeras do que células a
  grade é dividida em páginas. Navegue com `‹` / `›` na toolbar, `[` / `]` ou
  `PageUp` / `PageDown`.
- **Carrossel** — o botão `⟳` (tecla `c`) faz as páginas alternarem
  automaticamente; `+` / `−` ajustam o tempo em cada página (3–300 s). Qualquer
  navegação manual pausa o carrossel por alguns segundos.
- **Ordem das câmeras** — as setas ▲ / ▼ em cada linha da sidebar reordenam as
  câmeras; isso define quem aparece primeiro e em qual página.

Por padrão só as câmeras da página visível (mais a próxima, como prefetch) são
decodificadas — as demais ficam **pausadas** e não consomem CPU/rede. Ajuste
`[view] pause_hidden` no `config.toml` para desligar esse comportamento.

Todos esses ajustes são lembrados entre execuções em
`~/.local/state/rust-rtsp-viewer/view.toml`. As pipelines sobem de forma
escalonada no arranque (`[view] stagger_ms`), então a janela abre na hora e
cada célula mostra `CONECTANDO…` até o stream ligar.

### Movimento, zonas e notificações

A cada ~0,5 s cada câmera ao vivo tem o quadro comparado com o anterior
(diferença de luma, subamostrada). O evento **Movimento** entra na *timeline*
da sidebar apenas na **borda de subida** (movimento contínuo gera um evento só);
um stream congelado não gera movimento, e câmeras pausadas, offline ou
reconectando não são analisadas. A timeline é global (últimos 200 eventos da
última hora, com o nome da câmera; clicar abre a câmera).

**Zonas de movimento** — clique direito na câmera → *Zonas de movimento*
(ou o ícone `⬡` na barra do spotlight):

| Ação | Como |
|------|------|
| Marcar um canto | clique no vídeo |
| Concluir a zona | `Enter`, botão *Concluir*, ou clique no 1º ponto |
| Desfazer | `Backspace` (sem polígono aberto, remove a última zona) |
| Limpar tudo / sair | botão *Limpar* · `Esc` |

Com zonas ativas, **só o que muda dentro delas conta**, e `contour_area` passa a
ser a fração *da zona*. Zonas desativadas ou com menos de 3 pontos não restringem
nada; zonas sem área (pontos alinhados) são recusadas. As zonas ficam em
`~/.local/state/rust-rtsp-viewer/zones.toml`, **por nome da câmera** (nunca pela
URL, que carrega credenciais) — câmeras com o mesmo nome compartilham zonas.

`[recording] on_motion = true` grava automaticamente enquanto há movimento e
para `motion_post_roll_secs` depois do último (só para gravações que ele mesmo
iniciou). Não há pré-roll. `[notifications] enabled = true` dispara
`notify-send` para movimento e câmera offline, com intervalo mínimo por
câmera/tipo.

### Temas

`Cosmic` (padrão), `Dark`, `Light`, `AMOLED` e `OpenCode`, no menu `⋯`. Todas as
paletas passam em `ui::theme::tests::every_theme_meets_contrast_targets`
(texto ≥ 7:1, texto secundário e acentos ≥ 4,5:1, dicas ≥ 3,3:1, indicadores de
estado ≥ 3:1). Texto sobre emblemas coloridos usa `Theme::readable_on` para
escolher preto ou branco.

### Indicadores visuais

Cada célula muda de cor conforme seu estado:

| Estado | Borda | Badge |
|--------|-------|-------|
| Nenhum | transparente | — |
| Gravando | **vermelho** | `● REC 00:00:00` |
| Desconectado | inalterada | overlay `○ Reconectando…` |

### Pipeline GStreamer

Exibição:

```
rtspsrc → decoder → postdec_queue → videoconvert → capsfilter(RGBA) → tee → display_queue → appsink
```

Para HLS/HTTP, `uridecodebin3` é usado automaticamente (suporta HLS fMP4/CMAF). O áudio roda em um pipeline separado.

Gravação (anexada ao `tee` **somente enquanto grava**, para não consumir CPU ocioso):

```
tee → recording_queue → videoconvert → x264enc → splitmuxsink(muxer + filesink)
```

Ao parar, um EOS é enviado pelo branch para que o encoder esvazie e o muxer
finalize o arquivo antes de o branch ser removido.

---

## Gravar e detectar com a janela fechada (Docker)

O `rrv-daemon` é o motor sem janela: captura as câmeras, reconecta, grava e detecta movimento
mesmo com a janela fechada. Roda em Docker; só o daemon vai para o contêiner (a janela continua
nativa).

```bash
cp config.docker.toml.example config.docker.toml   # edite as câmeras (o arquivo é ignorado pelo git)
mkdir -p recordings state secrets "$XDG_RUNTIME_DIR/rrv"
printf '%s' 'a-senha-da-camera' > secrets/cam_portao_password && chmod 600 secrets/cam_portao_password
docker compose up -d --build
docker compose logs -f rrv
```

- **Gravações** em `./recordings` (ou `RRV_RECORDINGS`), **estado** (logs, zonas) em `./state`.
- **UID/GID:** os arquivos saem com o dono certo se `RRV_UID`/`RRV_GID` forem os seus (`id -u`, `id -g`);
  o padrão é 1000.
- **Rede:** `network_mode: host`, porque o RTP/UDP de volta das câmeras não atravessa bem a NAT do
  bridge do Docker.
- **Parar:** `docker compose stop` envia SIGTERM e o daemon **finaliza as gravações em curso**
  antes de sair (`stop_grace_period: 30s`). Um `kill -9` deixa o segmento aberto ilegível.
- **Saúde:** `healthcheck` do Docker lê um batimento que o laço do daemon escreve a cada segundo.
  Uma câmera fora do ar *não* deixa o contêiner doente: reiniciá-lo não a consertaria.
- **Validar a configuração** sem subir nada: `docker run --rm -v ./config.docker.toml:/config/config.toml:ro rust-rtsp-viewer/rrv-daemon:local --check`.
- **Senhas fora do `config.toml`:** qualquer URL do config (câmeras, `sub_url`, `[webhook]`) aceita
  `${NOME}`; o valor vem da variável de ambiente `NOME` ou do arquivo `/run/secrets/NOME` (é onde o
  Docker monta os *secrets*; `RRV_SECRETS_DIR` muda o diretório). Na parte `usuário:senha@` da URL o
  valor é *percent-encoded* sozinho: uma senha com `@`, `/` ou `:` não quebra a URL. `$${` escreve um
  `${` literal. Um segredo ausente impede o daemon de subir e a mensagem **nomeia o segredo, nunca o
  valor**; já o `--check` só avisa, para validar o arquivo sem ter os segredos. A senha também nunca
  vai para o log. (A janela nativa lê o mesmo `${NOME}` do ambiente; o chaveiro do sistema é a 5.3.)
- **Aviso com a janela fechada:** `[webhook] url = "https://…"` no `config.toml` faz o daemon dar um
  POST a cada movimento ou câmera offline (mesmo cooldown de `[notifications]`). `format = "json"`
  (Home Assistant, n8n, Node-RED: `camera`, `event`, `detail`, `title`, `message`, `time_unix`) ou
  `"ntfy"` (celular). A URL, que costuma levar o token, **nunca** aparece no log. Um destino fora do
  ar não atrapalha o daemon nem o desligamento. Usa o `curl`, que a imagem já traz.
- **Controle pelo socket:** o daemon abre `$XDG_RUNTIME_DIR/rrv/rrv.sock` (modo `0600`; no contêiner,
  `/run/rrv/rrv.sock` num volume compartilhado). A janela e o `rrvctl` falam com ele por aí:
  `rrvctl status`, `rrvctl record "Portão"`, `rrvctl enable|disable 2`, `rrvctl zones "Portão"`,
  `rrvctl events`. Dentro do contêiner: `docker exec rrv rrvctl status`.
- Fora do Docker: `cargo run -p rrv-daemon -- config.toml` (mesmos `--check` e `--health`).

**GPU Intel (VA-API):** `docker compose -f compose.yaml -f compose.vaapi.yaml up -d`; o `rrvctl status` mostra
`vah264dec (GPU)` ou `avdec_h264 (CPU)`. **Hoje ela não reduz a CPU** (medido: 71% × 74% de um núcleo
para 4 câmeras 1080p30), porque o custo está na conversão RGBA, não no H.264; veja
`docs/gpu-container.md`. A NVIDIA exige o `nvidia-container-toolkit`, e o aviso de movimento com a janela fechada é o `[webhook]` (abaixo); os eventos também vão para
`docker compose logs`. A imagem tem ~1 GB (plugins
`bad`/`ugly`/`libav` do GStreamer).

### A janela com o daemon

Ao abrir, a janela procura o socket do daemon (`$XDG_RUNTIME_DIR/rrv/rrv.sock`). **Sem socket, usa o
motor local** e funciona como sempre. Com socket, conecta, e **só mostra**: quem grava, detecta
movimento e avisa é o daemon (senão tudo sairia em dobro). O **chip** à esquerda do medidor de saúde
diz em que modo está; clique nele para o menu.

| Chip | Significa |
|---|---|
| `○ Motor local` | Não há daemon: a janela grava e detecta. **Fechá-la interrompe as gravações** (ela pergunta antes). |
| `● Daemon · conectado` | O daemon grava. Fechar a janela não interrompe nada. |
| `◐ Daemon · sem resposta` | Contato perdido: banner âmbar, tenta reconectar sozinha. As câmeras *podem* não estar gravando. |
| `▲ Daemon · versão incompatível` | A janela e o daemon falam versões diferentes: atualize um dos dois. |
| `■ Daemon · sem permissão` | O socket é de outro usuário. |

- Ao perder o daemon a janela **não** troca sozinha para o motor local (ele pode continuar gravando e a
  troca gravaria em dobro). O banner oferece **Reconectar agora** e **Usar motor local** (com confirmação).
- `rust-rtsp-viewer --embedded` força o motor local; `--daemon <socket>` (ou `RRV_SOCKET`) aponta o socket.
- Gravar (`r`) e salvar zonas viram pedidos ao daemon e só aparecem na tela **depois** de ele confirmar;
  se ele recusar, o editor de zonas continua aberto com o desenho.
- A aba Eventos mostra o que chegou desde que a janela conectou (o histórico persistente é do M3).

## Teclas de Atalho

| Tecla | Ação |
|-------|------|
| `Space` / `k` | Alternar seleção |
| `s` / `F12` | Capturar snapshot (PNG) |
| `r` | Iniciar / parar gravação |
| `m` | Mudo / desmudo |
| `+` / `-` | Volume up / down |
| `1`–`9` | Selecionar câmera |
| `h` | Modo imersivo (esconde toda a interface) |
| `f` / `Enter` / duplo-clique | Spotlight da câmera selecionada |
| `←` / `→` | Câmera anterior / próxima (no spotlight) |
| `Tab` | Alternar layout (grid/flex) |
| `g` | Ciclar densidade da grade (Auto/2×2/3×3/4×4) |
| `[` / `]` · `PageUp` / `PageDown` | Página anterior / seguinte da grade |
| `c` | Ligar / desligar o carrossel de páginas |
| `F2` | Mostrar / ocultar sidebar |
| `F3` | Alternar aba da sidebar |
| `F11` | Tela cheia do sistema + imersivo |
| `/` | Focar a busca de câmeras |
| `Esc` | Voltar: spotlight → imersivo → menu → busca |
| `?` | Ajuda (atalhos) |
| `Ctrl+Q` | Sair |

Enquanto a caixa de busca está focada, os atalhos de tecla única ficam
desativados para não roubarem o que você digita. Sair do app exige `Ctrl+Q`
justamente para que uma tecla solta nunca encerre a aplicação.

---

## Arquitetura

```
src/
├── bin/iced_viewer.rs          — entry point (clap, carga do config)
├── lib.rs                      — re-exports (config, domain, infrastructure, ui)
├── config.rs                   — Config + CameraConfig (deserialização TOML)
├── domain/                     — lógica pura, sem I/O
│   ├── audio.rs                — AudioConfig, AudioState
│   ├── bidirectional_audio.rs  — configuração de talk-back
│   ├── codec.rs                — enum Codec (H264/H265/Mjpeg/…)
│   ├── diagnostics.rs          — hints e severidade
│   ├── groups.rs               — agrupamento de câmeras
│   ├── metrics.rs              — Metrics (contadores atômicos)
│   ├── motion.rs               — detecção de movimento por diferença de frames
│   ├── multi_stream.rs         — seleção main/sub stream
│   ├── notify.rs               — política de notificações (cooldown, textos)
│   ├── ptz.rs                  — comandos PTZ
│   ├── recording.rs            — RecordingConfig, RecordingState
│   ├── redact.rs               — mascaramento de credenciais em logs
│   ├── snapshot.rs             — SnapshotConfig, nomes de arquivo, burst
│   ├── streaming.rs            — configuração de re-streaming
│   ├── timelapse.rs            — configuração de timelapse
│   ├── timeline.rs             — EventTimeline, TimelineEvent
│   └── zones.rs                — zonas poligonais de detecção
├── infrastructure/             — GStreamer, I/O
│   ├── audio.rs                — AudioController, build_audio_pipeline
│   ├── notify.rs               — notify-send / xdg-open
│   ├── reconnect.rs            — watchdog de FPS + lógica de backoff
│   ├── recording_paths.rs      — criação de diretórios
│   ├── view_state.rs           — view.toml (densidade, carrossel, ordem…)
│   └── zone_state.rs           — zones.toml (zonas por nome de câmera)
└── ui/                         — frontend Iced
    ├── app.rs                  — struct App, new_app(), PendingBurst
    ├── state.rs                — Toast, BackoffState, ContextMenu
    ├── message.rs              — enum Message
    ├── update.rs               — update() + resolução de teclado
    ├── subscription.rs         — teclado + tick de frames
    ├── bridge.rs               — GStreamerBridge (bus, métricas, frames, gravação)
    ├── pipeline.rs             — start_rtsp/hls/file, branch de gravação, sondas
    ├── video_widget.rs         — integração com iced::widget::image
    ├── zone_editor.rs          — canvas do editor de zonas (spotlight)
    ├── icons.rs                — fonte de ícones embutida (DejaVu Sans)
    ├── theme.rs                — temas + contraste (WCAG)
    ├── grid.rs                 — cálculo da grade
    ├── sidebar/                — cameras, info, diagnostics, timeline
    └── view/                   — composição (grid, flex, toolbar, status, overlays)
```

---

## Conhecido / Limitações

1. **Buffering delegado ao GStreamer**: não há cache de stream gerenciado pelo
   app — apenas `latency_ms` (→ `rtspsrc latency`). O `postdec_queue` fica nos
   defaults do GStreamer e só desacopla threads.
2. **Reconexão com backoff fixo**: Backoff exponencial (1→2→4→8→16→30s), sem
   variação por tipo de erro.
3. **Sem pause/resume**: Não há como pausar um stream ao vivo; `Space`/`k`
   apenas alterna a seleção da câmera.
4. **Decode time indisponível em HLS**: A medição casa PTS entre a entrada do
   decoder e o `postdec_queue`; `uridecodebin` não expõe um decoder separado
   para sondar, então a métrica fica vazia para HLS/DASH.
5. **Image quality usa RGBA**: Amostragem de luma via pesos BT.601 sobre RGBA
   em vez do plano Y nativo (custa uma conversão e trunca ~1 unidade).
6. **Zonas por nome de câmera**: câmeras com o mesmo nome compartilham zonas, e
   não há como apagar/renomear/desativar uma zona específica pela interface
   (só desfazer a última ou limpar todas).
7. **Movimento sem pré-roll**: a gravação por movimento começa quando o movimento
   é detectado; não há vídeo anterior ao gatilho.
8. **Timeline global**: sem filtro por câmera; janela fixa de 1 h.

---

## Desenvolvimento

```bash
cargo build                              # zero warnings
cargo clippy --all-targets -- -D warnings
cargo test                               # 375 testes
```

Os testes de gravação (`ui::pipeline`) rodam pipelines GStreamer reais
(`videotestsrc`), gravam arquivos de verdade e os reproduzem até o EOS para
provar que o muxer os finalizou (~6 s; exigem `x264enc`, do pacote
`gstreamer1.0-plugins-ugly`). `cargo test --doc` pode falhar em instalações
com `rustdoc` quebrado — não é problema do código. A CI (`.github/workflows`)
roda clippy e os testes a cada push. Convenções e armadilhas do código estão em
[AGENTS.md](AGENTS.md).

---

## Licença

Distribuído sob a **GNU Affero General Public License v3.0 ou posterior**
(`AGPL-3.0-or-later`) — veja [LICENSE](LICENSE). Quem executar uma versão
modificada como serviço de rede deve oferecer o código-fonte correspondente aos
usuários. Fontes e dependências de terceiros: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

---

## Troubleshooting

### Decoder não encontrado

```bash
sudo apt install gstreamer1.0-libav
gst-inspect-1.0 avdec_h264
```

### Câmera conecta no VLC mas não aqui

```toml
use_uridecodebin = true
```

### Câmera Intelbras rejeita conexão

```toml
do_retransmission = false
```
