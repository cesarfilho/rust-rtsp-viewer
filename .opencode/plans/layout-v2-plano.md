# Plano de correção — Layout v2

Baseado no `specs/layout-v2.md` e no código atual.

## Fase 0 — Bugs críticos (agora)

- [x] **Info panel não colapsa com F3** — `GtkPaned` mantém o espaço mesmo com `set_visible(false)`. Corrigido: usa `paned.set_position(99999)` para colapsar e restaura posição ao mostrar. Feito em `main_window.rs:661-670`.
- [x] **Sidebar não colapsa corretamente com F2** — Corrigido: ao esconder, salva a posição do paned e avança o divisor em 260px (sidebar width) para que o display ocupe o espaço. Ao mostrar, restaura a posição salva. Linhas 700-709.
- [x] **Overlay de nome some ao trocar layout** — Células (`CellEntry`) são criadas uma vez com os labels fixos; `idle_add_local_once` re-adiciona os mesmos widgets após o vídeo ficar pronto. `arrange()` só move `ev_box` entre containers, nunca destrói/re-cria overlays. `show_all()` após reparenting garante visibilidade. Não é bug com o código atual.

## Fase 1 — Info Panel (direita)

- [x] GtkPaned collapsible — feito junto com o bug do F3.
- [x] Sparkline FPS — `gtk::DrawingArea` com Cairo renderizando os últimos 64 frames (~16s) da câmera selecionada. Buffer `VecDeque<f64>` por câmera, atualizado a cada tick. Gradiente verde sob a curva, linha verde #22c55e. `queue_draw()` no tick e ao trocar seleção. `cairo-rs = "0.18"` adicionado ao `Cargo.toml`.

## Fase 2 — Overlays nas células de vídeo

- [x] Nome da câmera no canto superior esquerdo com classe CSS `.cell-name`
- [x] Métricas compactas (fps/latência) no canto inferior esquerdo — `hud_lbl` atualizado a cada 250ms no tick loop
- [x] Indicador de gravação pulsando no canto inferior direito — `rec_lbl` com 🔴 + elapsed (HH:MM:SS)
- [x] Borda colorida por estado — classes `.cell-frame-selected` (verde 3px), `.cell-frame-offline` (vermelho 2px), `.cell-frame-recording` (vermelho 2px) gerenciadas no tick loop

## Fase 3 — Validação de troca de layout

- [x] Garantir que vídeo não trava ao alternar Grid→Flex→Grid — `arrange()` só move `ev_box` entre containers; overlay/video permanecem intactos dentro do `ev_box`. `show_all()` + `queue_resize()` em cada `ev_box` + toplevel garantem re-alocação correta. `grid_cont` e `flex_cont` são pré-criados e nunca removidos do `display_box`, evitando reparenting de gtksink.
- [x] Clicar na thumbnail do Flex promove à câmera principal — `flex_main_idx: Rc<Cell<usize>>` rastreia a câmera principal no Flex. `arrange()` usa `flex_main_idx` (se ativa) ou `visible[0]` como main. Click handler no `ev_box` verifica `LayoutMode::Flex && flex_main_idx != i` e re-arranja com a câmera clicada como nova main. Implementado em `main_window.rs:536-549`.

## Fase 4 — Estado da pipeline

- [x] Status dot na sidebar: ● live (verde) / 🔴 recording (vermelho) / ⚠ reconnect (laranja, `reconnect_count > 0`) / ○ offline (cinza, `reconnect_count == 0`). Stats label mostra fps=0 quando offline.
- [x] Badge de reconexão na célula — `hud_lbl` mostra "⚠ RECONECTANDO" (laranja) quando `!is_live && reconnect_count > 0`, "○ OFFLINE" (vermelho) quando `!is_live && reconnect_count == 0`.
- [ ] Último frame congelado quando offline — **deferido**: requer appsink de captura contínua + `GtkImage` no overlay. Incompatível com reconnection RTSP (pipeline precisa ir a NULL/READY para resetar `rtspsrc`).

## Fase 5 — Keyboard shortcuts

- [x] `Space` — toggle selection: se algum estiver selecionado, desseleciona (esconde info panel); se nenhum, seleciona a primeira câmera ativa.
- [x] `Enter` — toggle fullscreen (mesmo que F11).
- [x] `Escape` — exit fullscreen (se estiver em fullscreen).
- [x] `Tab` — ciclar Grade ↔ Flex (aciona os botões da toolbar, que disparam `arrange()`).
- [x] `1-9` — selecionar câmera por índice (1 = primeira, 9 = nona). Só seleciona se a câmera estiver ativa (checkbox marcada).
- [x] `q` — `gtk::main_quit()`, encerra o app.
- [ ] `/` — focar busca — **deferido**: não há barra de busca atualmente.

## Fase 6 — Status bar

- [x] Uptime total do app — exibido como `HH:MM:SS` a partir do `Instant::now()` no tick loop.
- [x] "✓ Todos online" (verde) / "N offline" (vermelho) — contagem de câmeras com `is_live == false` no tick loop. Atualizado a cada 250ms.
- [x] Barra reformatada: estado + uptime + atalhos compactos (`s`nap, `r`ec, `F2`side, `F3`info, `Tab`layout, `Enter`full, `q`quit).


opencode -s ses_12086cf0dffeofC6b2vq2FmH5o