# Plano do que resta (refeito em 2026-10-06)

Substitui, para o que ainda está aberto, as tabelas de `plano-de-execucao.md` (que fica como histórico: tudo o
que está `[x]` lá está feito e commitado). Estado real do código: M0, M1, M2.5 (daemon + Docker + janela) e M3
(gravação sem reencode, histórico, retenção, pré-roll, áudio, vista Gravações com player, vários canais e clipes)
estão **feitos**. Falta o que está abaixo.

**Tamanhos** (palpite relativo): **P** ≤ 1 dia · **M** 2–4 dias · **G** 1–2 semanas · **XG** > 2 semanas.
**Quem**: `eu` = faço sozinho · `eu + você` = preciso da sua validação na tela ou de uma ação sua no host.

## Ordem sugerida
1. **Fase A**: fechar o M3 e limpar (nada depende de decisão sua). Entrega a v0.9.
2. **Fase B**: iced 0.14 e NV12 na janela (precisa de você na tela).
3. **Fase C** (IA) e **Fase D** (acabamento) são independentes entre si; podem andar em qualquer ordem depois da B,
   ou em paralelo com ela. Recomendo **D antes de C** se o objetivo é uma 1.0 usável; **C antes de D** se a IA é o
   que mais importa.

---

## Fase A — Fechar o M3 e limpar (≈ 1–2 semanas, tudo `eu`)

| # | Tarefa | Tam. | Critério de saída |
|---|---|---|---|
| A1 | ✅ **feito** — **Áudio G.711 e mp4**: testar `audio/x-alaw` e `audio/x-mulaw` (sintético: `audiotestsrc ! alawenc`) e o contêiner mp4 no `record_audio`; se o `splitmuxsink` recusar o formato, gravar sem áudio e avisar no log | P | teste automático com cada codec; o caso recusado não derruba a gravação |
| A2 | ✅ **spike feito, inconclusivo, sem `keyframe_every_secs`** (resultado abaixo) — **Spike do pré-roll curto em Smart Codec**: a Intelbras manda 1 keyframe a cada ~20 s e o pré-roll vira o GOP todo. Testar se ela atende pedido de keyframe (evento *force-key-unit* para cima → RTCP PLI/FIR). Se sim: `[recording] keyframe_every_secs` (padrão = pré-roll) | P (spike) + M | medir: com o pedido periódico o ring fica em ≈ 5 s e o bitrate sobe menos de X%; se a câmera ignora, documentar e fechar |
| A3 | ✅ **feito** (65% de CPU, 39% com a iGPU; 14/16 ao vivo) — **Baseline com 16 câmeras** (hoje só há 12): 4 sessões da Intelbras + 11 HLS + 1 HLS, daemon headless, CPU/GPU, com e sem movimento | P | linha nova em `docs/baseline.md`/`.csv`; confirma o orçamento do plano (0.4) |
| A4 | ✅ **feito** (aceito o reencode; ADR 0007) — **Gravação de HLS/arquivo sem reencode ou assumir o reencode**: o HLS (`uridecodebin3`) não tem tap codificado. Decidir: (a) aceitar `x264enc` só para as câmeras HLS (públicas, uso de visualização) e documentar; (b) tap pós-`hlsdemux`. Recomendo (a). MJPEG idem | P | decisão registrada no ADR 0007 |
| A5 | ✅ **feito, por busca automática** (sem diálogo de pasta: `$RRV_RECORDINGS`, `./recordings`, `~/Videos`; conferida por um arquivo real do histórico) — **Pasta de gravações da janela**: hoje ela lê de `[recording] dir` e, com o daemon no Docker, a pessoa precisa saber apontar para a pasta do host. O daemon passa a informar o caminho que usa (`status`), e a janela, ao não achar o arquivo, mostra uma ação "Escolher a pasta" que grava a escolha | M | abrir uma gravação do daemon Docker sem editar o `config.toml` |
| A6 | ✅ **feito** — **Docs vivos**: `status.md`, `gap_analysis.md` e `roadmap.md` ainda descrevem o estado de antes do M2.5/M3; reescrever com o que existe | P | os três refletem o código; nenhum item marcado "falta" que já está feito |
| A7 | ✅ **feito, local** (tag `v0.9.0`; o push espera a sua ordem) — **Release v0.9.0**: `CHANGELOG.md`, versão no workspace, tag. **O push só sai com a sua ordem** (a master está ~90 commits à frente) | P | tag criada localmente; `cargo build --release` limpo; imagem Docker com a tag |
| A8 | **Checklist de verificação com mouse** (você, 15 min): arrastar a barra, botão Comparar, `Ctrl+Q` com gravação local, cores nos 5 temas. Eu entrego o roteiro com o que olhar em cada passo | P | você responde "ok" ou aponta o que quebrou; eu corrijo |

Riscos: A2 depende da câmera (PLI não é obrigatório no RTSP). Se ela ignorar, o pré-roll longo é um limite do
equipamento e fica documentado.

---

## Fase B — iced 0.14 e NV12 na janela (≈ 3–4 semanas, `eu + você`)

A janela é o último lugar que converte cada quadro para RGBA. No daemon a conversão já saiu (CPU −57%, com a iGPU
−47% a mais). Na janela, o ganho depende do caminho NV12 com shader.

| # | Tarefa | Tam. | Quem | Critério de saída |
|---|---|---|---|---|
| B1 | **Rebase de `spike/deps-upgrade` na master** (iced 0.14, 84 ajustes de API já conhecidos + tudo o que entrou desde: vista Gravações, canvas, vários canais). Branch nova `iced-0.14`, sem tocar a master | G | eu | `cargo build/clippy/test --workspace` verdes na branch |
| B2 | **Validação na tela** das cores (o 0.14 clareava as cores com a GTX 1650; `WGPU_POWER_PREF=low` na iGPU resolvia): 5 temas × grade, spotlight, vista Gravações, menus. Eu capturo a região da janela; você olha | P | eu + você | você aprova ou lista as diferenças |
| B3 | **Decisão do merge**: o 0.14 entra na master, ou a master fica no 0.13 até a 2.3 | P | você | decisão registrada |
| B4 | **Script de medição da janela** (`scripts/baseline-window.sh`): CPU/RSS do processo da janela, com N câmeras, grade e spotlight | P | eu | número de referência *antes* da 2.3 |
| B5 | **2.3, NV12 + shader**: `appsink` em NV12; widget wgpu próprio (texturas Y e UV, conversão na GPU); o RGBA fica como fallback e para snapshots/detecção; a vista Gravações usa o mesmo widget | G–XG | eu + você | janela com 16 × 1080p dentro do orçamento medido em B4; imagem idêntica ao RGBA (comparação por captura); sem regressão nos testes |
| B6 | **2.4 zero-copy / PRIME** (renderizar na NVIDIA) | XG | — | **só se a B5 não bastar**; hoje não planejada |

Riscos: B5 é o item de maior risco do projeto (widget wgpu próprio dentro do iced). Se B1 mostrar que o 0.14 muda
a API de `shader`, a B5 só começa depois da B3. Plano B da B5: reduzir a resolução decodificada no grid (já há
sub-stream por câmera) e deixar o NV12 para depois.

---

## Fase C — IA: detecção de objetos (≈ 6–8 semanas, M4)

D3 está decidida: **YOLO da Ultralytics** (AGPL, compatível com a AGPL-3.0 do projeto). Conferir a licença do peso
exato ao baixá-lo.

| # | Tarefa | Tam. | Quem | Critério de saída |
|---|---|---|---|---|
| C0 | **0.6, spike do `ort`**: YOLO-n a 320 e 640 em (1) CPU (já medido: 38–117 ms), (2) OpenVINO na iGPU Intel (sem `sudo`), (3) CUDA na GTX 1650 (só se a D6 estiver feita). Escolher o backend | M | eu (+ você: D6) | `docs/spike-0.6-ort.md` com ms/inferência e VRAM por backend e a recomendação |
| C1 | **4.1 módulo `detect`** (feature `detect`): pré-processamento, inferência, NMS; `scripts/fetch-model.sh` com SHA-256 (o peso não entra no repositório) | G | eu | testes com imagens de referência: caixas dentro de uma tolerância de IoU |
| C2 | **4.2 thread de inferência**: fila limitada que descarta o mais antigo, métricas (ms, fila, descartes) | M | eu | nunca bloqueia o `appsink` nem a janela (teste com inferência lenta simulada) |
| C3 | **4.3 gatilho**: só roda com movimento e dentro das zonas, sobre a região recortada do ramo de detecção | M | eu | GPU/CPU ociosas sem movimento (medido) |
| C4 | **4.4 eventos por rótulo**: schema v3 do histórico (caixa, rótulo, score, zona), `[detect] labels`/`min_score`, notificação e webhook por rótulo, cooldown por rótulo | M | eu | filtro por rótulo funciona de ponta a ponta no `rrvctl history` |
| C5 | **4.5 interface**: caixas sobre o vídeo no spotlight e no player, filtro por rótulo na lista de eventos, revisão "Alertas × Detecções" | G | eu + você | você revisa a interface na tela; contraste dos novos estados nos 5 temas |
| C6 | **4.6 avaliação**: conjunto pequeno com gabarito (suas câmeras, anonimizado), regressão por IoU/score, precisão/recall registrados, fallback em CPU | M | eu + você (as imagens) | números em `docs/` e um teste que falha se piorarem |
| C7 | **4.7 empacotar** o `libonnxruntime` na imagem Docker (variantes CPU / OpenVINO / CUDA) | M | eu | `docker compose` sobe a detecção sem instalar nada no host |

Dependências: C1→C2→C3→C4→C5; C6 e C7 em paralelo com a C5. A C0 decide o backend e portanto o Dockerfile da C7.

---

## Fase D — Acabamento para a 1.0 (≈ 4–6 semanas, M5)

Decididos em 2026-10-06: **só Linux** (ADR 0001) e **sem PTZ**. O ONVIF fica só para descoberta e cadastro.

| # | Tarefa | Tam. | Quem | Critério de saída |
|---|---|---|---|---|
| D1 | **5.1 ONVIF**: descoberta (WS-Discovery) e assistente de cadastro; Profile T como base, sem PTZ. Teste na Intelbras (a maioria suporta ONVIF) | G | eu | "Adicionar câmera" lista as da rede e preenche a URL e o sub-stream |
| D2 | **5.3 chaveiro** (`secret-service`) para as senhas da janela (o daemon já usa `${NOME}`/Docker secrets) | M | eu | senha fora do `config.toml` na janela, com fallback documentado |
| D3 | **5.4 MQTT/Home Assistant** (`rumqttc`): eventos, saúde por câmera, descoberta automática do Home Assistant | M | eu | câmeras e sensores de movimento aparecem no Home Assistant (teste com um broker local) |
| D4 | **5.5 i18n e acessibilidade**: extrair os textos (hoje em português dentro do código), pt-BR + en, foco por teclado e rótulos legíveis por leitor de tela | G | eu | alternar o idioma sem reiniciar; nenhum texto fixo fora do catálogo (teste que varre) |
| D5 | **5.6 empacotamento (só Linux)**: AUR, AppImage/Flatpak, releases automáticas (`cargo-dist`) e a imagem Docker publicada | M | eu + você (contas/chaves) | `pkgbuild` instala e roda; release de teste no GitHub |

---

## Decisões e ações que só você pode tomar

| # | O quê | Libera |
|---|---|---|
| **GPU NVIDIA** (você quer) | Rodar no host, **sem reiniciar o Docker** (o `restart` derrubaria os seus outros contêineres): `sudo pacman -S nvidia-container-toolkit` e `sudo nvidia-ctk cdi generate --output=/etc/cdi/nvidia.yaml`; depois `scripts/check-nvidia-host.sh`. Você pode rodar aqui com `! sudo ...` | C0 (backend CUDA) e a decodificação NVDEC |
| **B3** | Merge do iced 0.14, depois da validação na tela | B5 |
| **Push** | Quando enviar a master (~90 commits) e a tag v0.9.0 | A7 |

**Decididas (2026-10-06):** só Linux (D4, ADR 0001 revisada) · sem PTZ (5.2 removida) · a rede da Intelbras (Wi-Fi, ~25% de
perda) fica como está: é o motivo de ela ter pouco fps e os testes dela saem ruidosos; use as medições das câmeras HLS e do
sintético como referência.

## Adiado de propósito
- **2.5.4** (vídeo do daemon redistribuído para a janela): a janela abre sessão própria, e a Intelbras aceitou 4.
  Reabrir só se algum modelo de câmera limitar a 1–2 sessões.
- **2.4** (zero-copy): só se a B5 não bastar.
- **Vários servidores**: fora do escopo (o canal de controle é um socket Unix, só local).
