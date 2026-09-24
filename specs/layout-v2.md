# NVR Layout v2 — Especificação de UI/UX

Baseado em pesquisa dos projetos open-source Frigate (33.8k★), camera.ui (1k★),
Shinobi (~8k★), ZoneMinder (~5k★), OpenNVR e melhores práticas de VMS design.

## 1. Arquitetura da Janela

```
┌──────────────────────────────────────────────────────────┐
│  TOOLBAR (contextual, 32px)                              │
│  [≡] [Grade] [Flex] [Seq] │ [🔍 filtro] [⛶] [⚙] [✕]   │
├────────────┬───────────────────────────┬─────────────────┤
│            │                           │                 │
│  SIDEBAR   │   LIVE VIEW (Grid/Flex)   │  INFO PANEL     │
│  esquerda  │                           │  (opcional)     │
│  260px     │                           │  280px          │
│            │                           │                 │
│  ┌──────┐  │  ┌────┬────┬────┐         │  ┌───────────┐  │
│  │ 📹   │  │  │    │    │    │         │  │  Câmera    │  │
│  │ Cam 1│  │  ├────┼────┼────┤         │  │  atual:    │  │
│  │ ●    │  │  │    │    │    │         │  │  stats     │  │
│  ├──────┤  │  └────┴────┴────┘         │  ├───────────┤  │
│  │ 📹   │  │                           │  │  Timeline  │  │
│  │ Cam 2│  │                           │  │  compacta  │  │
│  │ ○    │  │                           │  └───────────┘  │
│  ├──────┤  │                           │                 │
│  │ ...  │  │                           │                 │
│  └──────┘  │                           │                 │
├────────────┴───────────────────────────┴─────────────────┤
│  STATUS BAR (20px): 4/6 ativas | Grade 2×2 | CPU 12%     │
│  | Mem 45% | uptime 2h34m                                 │
└──────────────────────────────────────────────────────────┘
```

## 2. Painéis

### 2.1 Toolbar Superior (32px)

| Região | Conteúdo | Detalhes |
|--------|----------|----------|
| Esquerda | Logo + modo layout | `Grade` / `Flex` / `Sequência` — GtkToggleButton com exclusão mútua |
| Centro | Ações rápidas | `📷 Snapshot` (F12) · `🔴 Record` (r) · `🔊 Áudio` (m) |
| Direita | Controles de janela | `🔍` filtro · `⛶` fullscreen (F11) · `⚙` menu config · `✕` quit |

**Comportamento**:
- Botões de layout alternam entre si (set_active(false) no outro) — já implementado para Grade/Flex.
- Separadores verticais entre seções (GtkSeparator).
- Toolbar colapsa para linha única em janelas estreitas (<800px).

### 2.2 Sidebar Esquerda (260px)

```
 ┌──────────────────────────────┐
 │ 📹 CÂMERAS           🔍 │   │ ← header + search
 ├──────────────────────────────┤
 │ ☑ 📹 Sala           ● RTSP   │
 │    fps:30  lat:45ms          │ ← métricas (hover ou expand)
 ├──────────────────────────────┤
 │ ☐ 📹 Garagem         ○ FILE  │
 │    ─                        │
 ├──────────────────────────────┤
 │ ☑ ☁️ HLS IP          ● HLS   │
 │    fps:15  lat:120ms  ⚠     │
 ├──────────────────────────────┤
 │ ☑ 📹 Quintal         ● RTSP  │
 │    🔴 gravando  00:02:34     │ ← indica gravação ativa
 └──────────────────────────────┘
```

#### Elementos por linha de câmera:

| Elemento | Tipo | Descrição |
|----------|------|-----------|
| Checkbox | `GtkCheckButton` | Mostra/esconde no live view |
| Ícone | `GtkLabel` | 📹 RTSP · ☁️ HLS · 📁 File |
| Nome | `GtkLabel` | monospace 9pt, cor `#d4d4d4` |
| Status dot | `GtkLabel` | ● live (verde #22c55e) · ○ offline (#666) · ⚠ reconnect (#eab308) · 🔴 recording (#ef4444) |
| Badge tipo | `GtkLabel` | RTSP / HLS / FILE (caps 7pt, borda 1px) |
| Métricas | `GtkLabel` (2ª linha) | fps, latência, estado — mostra no hover ou toggle |

#### Comportamento:
- **Clique no nome**: seleciona câmera no live view (borda verde + info panel atualiza)
- **Clique direito**: menu contextual
- **Duplo clique**: fullscreen popup
- **Hover >1s**: expande métricas se não estiver sempre visível
- **Campo de busca**: filtra lista por nome/tipo em tempo real (GtkSearchEntry)

#### Menu contextual (right-click):

```
📷  Tirar Snapshot
🔴  Iniciar Gravação   │ se aplicável
⛶  Fullscreen
🔄  Reconectar          │ se offline
📋  Copiar URL
━━━━━━━━━━━━━━━━━━━━
❌  Remover da Vista    │ só oculta, não para pipeline
```

### 2.3 Live View (central, expande)

#### 2.3.1 Modos de Layout

| Modo | Descrição | Atalho |
|------|-----------|--------|
| **Grade** | Grid uniforme N×M, auto-calc, até 16 câmeras (4×4) | `Tab` |
| **Flex** | 1 main + N thumbnails verticais (300px) | `Tab` |
| **Sequência** | 1 câmera por vez em fullscreen, rodízio automático | `Tab` |

#### 2.3.2 Grade Uniforme

- `calc_grid()` atualizada com suporte até 16:

```rust
fn calc_grid(n: usize) -> (i32, i32) {
    match n {
        0 | 1 => (1, 1),
        2 => (2, 1),
        3 | 4 => (2, 2),
        5 | 6 => (3, 2),
        7 | 8 | 9 => (3, 3),
        10 | 11 | 12 => (4, 3),
        13 | 14 | 15 | 16 => (4, 4),
        _ => { let c = (n as f64).sqrt().ceil() as i32; (c, (n as i32 + c - 1) / c) }
    }
}
```

- Células sempre homogêneas (row/column homogeneous = true)
- Espaçamento 2px entre células

#### 2.3.3 Flex

- 1 câmera principal à esquerda (expande) + N thumbnails verticais em coluna à direita
- Câmera principal selecionável: clique na thumbnail a promove a main
- Thumbnails têm 300px de largura, altura igual entre si (vexpand=true)
- Main tem `vexpand=true` e `hexpand=true`

#### 2.3.4 Sequência

- Mostra 1 câmera por vez ocupando todo o live view
- Timer (Glib) alterna entre câmeras a cada N segundos (padrão 10s)
- Barra de progresso no canto inferior direito (aresta 3px, 0→100%)
- Pause no hover do mouse
- Próximo/anterior com scroll do mouse
- Ordem configurável: sequencial ou aleatória

#### 2.3.5 Célula de Câmera

```
┌──────────────────────────────┐
│ Sala               [●●●]     │ ← overlay nome (sup-esq)
│                              │
│         [VÍDEO LIVE]         │ ← gtksink widget
│                              │
│  fps:30  lat:45ms            │ ← HUD (inf-esq, compact)
│                    🔴 00:02  │ ← recording (inf-dir)
└──────────────────────────────┘
```

**Overlays no vídeo** (mínimos, sem clock/textoverlay):

| Posição | Conteúdo | Estilo |
|---------|----------|--------|
| Sup-Esq | Nome da câmera + status glyph | BG semi-transparente `rgba(0,0,0,0.55)`, monospace 9pt |
| Inf-Esq | Métricas compactas (fps/latência) | Só aparece se compact_overlay=true ou no hover |
| Inf-Dir | Indicador de gravação (🔴 + tempo) | Pulsando via CSS animation |

**Bordas da célula por estado:**

| Estado | Cor da borda |
|--------|-------------|
| Normal | `#141414` (1px) |
| Selecionada | `#22c55e` (3px) |
| Erro/Offline | `#ef4444` (2px) |
| Reconnect | `#eab308` (2px) |
| Gravando | `#ef4444` (3px) — apenas se não selecionada |

### 2.4 Info Panel (direita, opcional)

Implementado via GtkPaned (redimensionável). Oculta por padrão.

Mostra informações detalhadas da **câmera atualmente selecionada**:

```
┌──────────────────────────┐
│ Sala                     │ ← nome em destaque
│ ● LIVE · RTSP            │ ← status + tipo
├──────────────────────────┤
│ 📊 Métricas              │
│ FPS real:        29.8    │
│ Latência:        42 ms   │
│ Jitter:           3 ms   │
│ Packet loss:     0.01%   │
│ Bitrate:        2.4 Mbps │
│ Resolução:  1920×1080    │
│ Codec:     H264 High@L4  │
│ Uptime:         2h 34m   │
├──────────────────────────┤
│ 🩺 Diagnósticos          │
│ ▸ Latência ok            │
│ ▸ Sem perda de pacotes   │
│ ▸ ────                   │
├──────────────────────────┤
│ ⏱ Timeline (60s)        │
│ ┌────────────────────┐   │
│ │ █▁▁▃▁█▁▁▃▁▁▁▁▁▃▁▃ │   │ ← mini gráfico de atividade
│ └────────────────────┘   │
│ ▸ 3 eventos de motion    │
│ ▸ 1 detecção de pessoa   │
└──────────────────────────┘
```

### 2.5 Status Bar (inferior, 20px)

```
4/6 câmeras ativas | Grade 2×2 | CPU 12% | Mem 512MB/8GB | 🟢 Todos online | uptime 2h34m
```

- Monospace 8pt, cor `#4a4a4a`, fundo `#050505`
- Atualizado a cada 2s via `glib::timeout_add`
- Origem dos dados: `/proc/stat` + `sysinfo()` para CPU/mem (ou placeholder)

## 3. Interações

### 3.1 Teclado (globais)

| Tecla | Ação |
|-------|------|
| `Space` | Selecionar/desselecionar câmera sob mouse |
| `Enter` | Fullscreen popup da câmera selecionada |
| `Escape` | Sair de fullscreen / fechar popup |
| `Tab` | Ciclar modos de layout: Grade → Flex → Sequência |
| `1`–`9` | Selecionar câmera por índice na sidebar |
| `s` / `F12` | Snapshot da câmera atual/selecionada |
| `r` | Toggle gravação (câmera atual ou todas) |
| `m` | Toggle mute global |
| `f` / `F11` | Toggle fullscreen da janela |
| `q` / `Ctrl+Q` | Quit (confirma se gravando) |
| `/` | Focar campo de busca na sidebar |
| `?` | Mostrar ajuda de teclado (popup) |

### 3.2 Mouse

| Ação | Resultado |
|------|-----------|
| Clique na célula | Seleciona câmera (borda verde) |
| Duplo clique na célula | Popup fullscreen da câmera |
| Clique direito na célula | Menu contextual |
| Scroll no modo Sequência | Próximo/anterior |
| Clique na thumbnail (Flex) | Promove a main |
| Arrastar sidebar | Reordenar câmeras |
| Arrastar divisor do info panel | Redimensionar |

### 3.3 Estados da Câmera

| Estado | Dot | Borda | Ação automática |
|--------|-----|-------|-----------------|
| `Connecting` | ⏳ | Azul | Placeholder + spinner |
| `Live` | ● verde #22c55e | Normal | Stream normal |
| `Reconnecting` | ⚠ amarelo | Amarela 2px | Tentativa N/5, backoff |
| `Offline` | ○ cinza | Vermelha 2px | Último frame congelado |
| `Recording` | 🔴 vermelho | Normal + HUD | Indicador pulsando |
| `Finished` (File) | ⏹ cinza | Normal | "Concluído" |

## 4. Configuração (config.toml)

### 4.1 Layout persistente

```toml
[cameras]
order = ["Sala", "Garagem", "Quintal"]  # ordem na sidebar e grid

[cameras.layout]
mode = "grid"                      # "grid" | "flex" | "sequence"
sequence_interval_secs = 10

[cameras.layout.grid]
cols = 0  # 0 = auto

[cameras.layout.flex]
main_camera = 0  # índice no order[] ou nome exato

[cameras.layout.sequence]
order = []  # vazio = ordem do array cameras.order
shuffle = false
```

### 4.2 Configuração de UI

```toml
[ui.sidebar]
visible = true
width = 260

[ui.info_panel]
visible = false
width = 280

[ui]
compact_overlay = false  # já existe
```

## 5. Plano de Implementação

### Fase 1 — Corrigir e estabilizar ✅ (parcial)
- [x] Consertar Flex mode (câmeras não aparecem) — `show_all()` + `queue_resize`
- [ ] Garantir que gtksink embarca em todos os modos de layout
- [ ] Validar troca de layout repetida (Grid→Flex→Grid) sem perda de vídeo

### Fase 2 — Sidebar aprimorada
- [ ] Status dot por câmera (live/offline/reconnect/recording)
- [ ] Segunda linha com métricas compactas
- [ ] Campo de busca/filtro
- [ ] Menu de contexto (right-click)

### Fase 3 — Info Panel (direita)
- [ ] GtkPaned entre live view e info panel
- [ ] Exibir métricas detalhadas da câmera selecionada
- [ ] Mini timeline com activity sparkline

### Fase 4 — Sequência
- [ ] Timer para rodízio entre câmeras
- [ ] Barra de progresso no HUD
- [ ] Pause no hover

### Fase 5 — Overlays e HUD
- [ ] HUD compacto no canto inferior (fps/latência)
- [ ] Indicador de gravação pulsando
- [ ] Borda colorida por estado da pipeline

### Fase 6 — Popup fullscreen
- [ ] Nova GtkWindow ao duplo clique
- [ ] Tecla Esc fecha popup
- [ ] Snapshot/Recording no popup

### Fase 7 — Polimento
- [ ] Smart streaming (snapshot quando idle)
- [ ] Drag & drop reorder na sidebar
- [ ] Ajuda de teclado (`?`)
- [ ] Salvar estado do layout em config

## 6. Arquivos Afetados

| Arquivo | Mudança |
|---------|---------|
| `src/infrastructure/main_window.rs` | Reescrita parcial (sidebar, info panel, overlays, modos) |
| `src/infrastructure/mod.rs` | Adicionar módulos |
| `src/infrastructure/info_panel.rs` | **NOVO** — painel direito |
| `src/infrastructure/layout_preset.rs` | **NOVO** — serialização de layout |
| `src/domain/overlay.rs` | Novos tipos para HUD compacto |
| `src/domain/mod.rs` | Adicionar layout_preset |
| `src/config.rs` | Novos campos de UI/layout |
| `src/main.rs` | Passar config de layout |
| `config.toml.example` | Atualizar exemplos |
| `specs/layout-v2.md` | Esta especificação |
