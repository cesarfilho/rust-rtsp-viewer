# rust-rtsp-viewer

Visualizador de streams RTSP/HLS construído em Rust com **Iced** + **GStreamer**. Suporta grade multi-câmera, painel lateral de métricas ao vivo, reconexão automática com backoff, gravação por segmentos, snapshots e áudio por câmera.

---

## Instalação

### Pré-requisitos

- Rust 1.85+ (edition 2024)
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
  gstreamer1.0-libav

# Arch Linux
sudo pacman -S gstreamer gst-plugins-base gst-plugins-good gst-plugins-bad gst-libav

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

O Iced viewer lê toda a configuração do `config.toml`. Não há flags CLI posicionais — tudo é configurado via arquivo. O buffering do stream é gerenciado pelo GStreamer; o único ajuste exposto é `latency_ms` (→ `rtspsrc latency`).

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
```

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
(`📷 ⏺ ♪ ⤢`) aparece no canto — as ações antes ficavam numa barra acima da grade
que roubava altura de vídeo.

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
│   ├── hw_encoder.rs           — seleção de encoder por hardware
│   ├── metrics.rs              — Metrics (contadores atômicos)
│   ├── motion.rs               — detecção de movimento por diferença de frames
│   ├── multi_stream.rs         — seleção main/sub stream
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
│   ├── reconnect.rs            — watchdog de FPS + lógica de backoff
│   └── recording_paths.rs      — criação de diretórios
└── ui/                         — frontend Iced
    ├── app.rs                  — struct App, new_app(), PendingBurst
    ├── state.rs                — Toast, BackoffState, ContextMenu
    ├── message.rs              — enum Message
    ├── update.rs               — update() + resolução de teclado
    ├── subscription.rs         — teclado + tick de frames
    ├── bridge.rs               — GStreamerBridge (bus, métricas, frames, gravação)
    ├── pipeline.rs             — start_rtsp/hls/file, branch de gravação, sondas
    ├── video_widget.rs         — integração com iced::widget::image
    ├── zone_editor.rs          — canvas do editor de zonas (ainda não conectado)
    ├── theme.rs                — temas
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
6. **Editor de zonas não conectado**: `ui/zone_editor.rs` e `domain/{zones,motion}.rs`
   compilam e são testados, mas ainda não têm ponto de entrada na interface.

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
