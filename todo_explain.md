# TODO explicado — ideias inspiradas no Frigate

Explicação de cada item do `todo.md`: o que é, como o Frigate faz, por que vale a pena
e onde encaixa neste projeto. Referência: https://docs.frigate.video/

---

## Fase 1 — ligar o que já existe

### Detecção de movimento (`domain/motion.rs`)
- **O que é:** comparar frames consecutivos e detectar quando algo mudou na cena.
- **No Frigate:** é um filtro barato que decide *quando* vale a pena rodar detecção de
  objetos. Parâmetros: `threshold` (sensibilidade de luminância, 1–255, padrão 30),
  `contour_area` (tamanho mínimo da área em movimento, padrão 10), `lightning_threshold`
  (ignora mudanças bruscas da cena inteira, como troca IR/cor ou movimento de PTZ) e
  *motion masks* (regiões ignoradas, como timestamp na imagem ou vegetação).
- **Aqui:** o módulo com a lógica de frame-difference já existe, mas nenhum código o
  chama e não há seção `[motion]` no config. `EventType::Motion` existe na timeline e
  nunca é emitido.
- **Por que importa:** é a base de quase todo o resto (gravação por evento, smart
  streaming, birdseye, alertas).

### Zonas (`ui/zone_editor.rs`)
- **O que é:** polígonos desenhados sobre a imagem da câmera que limitam onde eventos contam.
- **No Frigate:** *inertia* exige que o objeto permaneça na zona por N frames seguidos
  (evita falso positivo por oscilação); *loitering* exige tempo mínimo parado na zona
  antes de alertar. Zonas também podem ser obrigatórias para gerar alertas.
- **Aqui:** o editor em canvas está compilado, mas nenhuma view ou mensagem o usa
  (`allow(dead_code)`). Falta uma view, mensagens em `message.rs` e persistência no config.

### Sub-stream no grid / main-stream no spotlight (`domain/multi_stream.rs`)
- **O que é:** cada câmera oferece dois streams: um de baixa resolução (sub) e um de alta (main).
- **No Frigate:** o stream leve serve para detecção e o de alta qualidade para gravação.
- **Aqui:** no grid com muitas câmeras, decodificar o sub-stream reduz muito CPU/rede;
  ao entrar em spotlight, troca-se para o main. Precisa de `sub_url` em `[[cameras]]` e
  lógica de troca em `update::sync_active_streams`. Hoje só existe o enum `StreamQuality`.

### Smart streaming
- **O que é:** economizar banda/CPU mostrando imagem estática quando não há atividade.
- **No Frigate:** exibe um snapshot atualizado a cada minuto quando ocioso e passa para
  live ao detectar movimento ou objeto.
- **Aqui:** combina com `pause_hidden` (já existe para câmeras fora da página) e com a
  detecção de movimento. `domain/streaming.rs` já descreve algo parecido ("pausar decode
  em cena estática") mas está desligado.

---

## Fase 2 — gravação inteligente

### Retenção por modo em `[recording]`
- **O que é:** política sobre *o que* manter gravado.
- **No Frigate:** `all` (tudo), `motion` (só segmentos com movimento ou objetos),
  `active_objects` (só segmentos com objetos realmente em movimento). Alertas e detecções
  podem ter retenção independente.
- **Aqui:** hoje a gravação é manual (tecla `r`) e guarda tudo. Precisa de metadados por
  segmento (teve movimento?) para decidir o que apagar.

### Pré/pós-captura (ring buffer)
- **O que é:** gravar alguns segundos *antes* do evento e alguns depois.
- **No Frigate:** padrão de 5 s, com pré-captura de até 60 s.
- **Aqui:** é o item mais delicado. O AGENTS.md diz que o ramo de gravação só fica ligado
  ao `tee` durante a gravação, para não gastar um núcleo por câmera com x264. Pré-captura
  exige manter um buffer circular (por exemplo, quadros já codificados em memória),
  o que vai contra essa regra e precisa de decisão de design.

### Limpeza por espaço em disco
- **O que é:** apagar os segmentos mais antigos quando o disco está enchendo.
- **No Frigate:** com menos de ~1 hora de espaço restante, apaga as gravações mais
  antigas independentemente da retenção configurada.
- **Aqui:** não existe nada parecido para gravações (só `retention_days` de logs).
  Pode ser uma tarefa periódica sobre o diretório de `recording_paths.rs`.

### Export de clipes
- **O que é:** extrair um trecho (ex.: de um evento) para um arquivo permanente.
- **No Frigate:** exportação pela UI/API, com argumentos FFmpeg customizáveis.
- **Aqui:** depende da timeline persistente ligada aos segmentos.

---

## Fase 3 — revisão e histórico

### Review items (Alerts vs Detections)
- **O que é:** agrupar vários eventos sobrepostos em um único item de revisão.
- **No Frigate:** um item cobre todo o período em que houve objetos ativos numa câmera.
  *Alerts* são alta prioridade (pessoa, carro); *Detections* são de menor prioridade.
- **Aqui:** `domain/timeline.rs` já tem `EventTimeline` e `EventType`; o agrupamento e a
  severidade seriam uma camada por cima.

### Timeline persistente com playback/seek
- **O que é:** histórico de eventos que sobrevive ao fechar o app e leva ao vídeo gravado.
- **Aqui:** hoje a timeline é só em memória, sem vínculo com arquivos. Salvar em
  `~/.local/state/rust-rtsp-viewer/` (como `view.toml`) com caminho do segmento e
  timestamp, e um player com seek. Hoje não há navegador de gravações.

### Modo Birdseye
- **O que é:** uma visão combinada que mostra só as câmeras com atividade.
- **No Frigate:** modos `continuous`, `motion` (movimento nos últimos 30 s) e `objects`;
  ordem configurável e limite de câmeras.
- **Aqui:** reaproveita `sync_active_streams` e a paginação do grid, filtrando por
  atividade recente.

---

## Fase 4 — avançado

### Detecção de objetos (ONNX/YOLO)
- **O que é:** identificar pessoa, carro, animal etc. com um modelo de ML.
- **No Frigate:** modelos de 320x320 (YOLOv9 small é o ponto de partida); recorta a região
  de movimento e amplia antes de rodar o detector. Suporta Coral, OpenVINO, TensorRT, ONNX.
- **Aqui:** é o maior esforço: nova dependência (por exemplo `ort`), thread de inferência,
  fila compartilhada entre câmeras e decisão de hardware. Só vale depois de movimento e zonas.

### PTZ ONVIF + autotracking (`domain/ptz.rs`)
- **O que é:** controlar câmeras PTZ e fazê-las seguir um objeto.
- **No Frigate:** exige ONVIF com movimento relativo; zoom desligado/absoluto/relativo,
  `return_preset` ao terminar e calibração da velocidade dos motores.
- **Aqui:** existe `PtzCommand`, mas sem cliente ONVIF. O controle manual (setas/zoom na
  UI) é um bom primeiro passo antes do autotracking.

### Áudio bidirecional e detecção de áudio
- **O que é:** falar pela câmera (two-way talk) e disparar eventos por som.
- **No Frigate:** two-way talk via WebRTC em câmeras compatíveis.
- **Aqui:** o áudio atual é só de reprodução (`infrastructure/audio.rs`).
  `bidirectional_audio.rs` guarda a configuração, sem pipeline de envio.

### Re-streaming, MQTT/notificações, API HTTP
- **Re-streaming:** o app vira um servidor RTSP/HLS que republica as câmeras, reduzindo o
  número de conexões diretas à câmera (o Frigate usa go2rtc para isso).
- **MQTT/notificações:** publicar eventos para outros sistemas (Home Assistant etc.) e
  avisar o usuário. Hoje só há toasts dentro do app.
- **API HTTP:** expor eventos, snapshots e controles para integrações externas.

---

## Ordem sugerida
1. Movimento → zonas (base para tudo).
2. Sub-stream (ganho imediato de performance no grid).
3. Retenção e limpeza de disco (evita encher o disco).
4. Timeline persistente e playback.
5. Só então detecção de objetos, PTZ e integrações.
