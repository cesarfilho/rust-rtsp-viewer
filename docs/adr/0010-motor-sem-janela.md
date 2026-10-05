# 0010 — Motor sem janela: daemon + cliente
**Status:** Aceita (decisão do dono, 2026-10-05: o NVR continua gravando e detectando sem a janela aberta).
O **desenho técnico** abaixo é **Proposta** e depende da medição de sessões RTSP (D2, tarefa 0.3).

Substitui o ADR 0004 ("sem serviço headless").

## Contexto
Um NVR grava 24 h, com ou sem alguém olhando. Hoje tudo vive no processo da janela: fechar o app
para a gravação, o movimento e as notificações. Com D1 = NVR completo (ADR 0003, plano M3/M4), isso
não serve.

## Decisão
Separar em dois processos:
- **`rrv-daemon`** (sem interface): captura as câmeras, grava (3.1), detecta movimento e objetos
  (M1/M4), escreve o SQLite (3.2), aplica retenção (3.3), reconecta, notifica. **Roda em container
  Docker** (decisão do dono, 2026-10-05), com `restart: unless-stopped`; um serviço de usuário do
  systemd fica como alternativa para quem não usa Docker.
- **Cliente (a janela Iced atual)**: assiste ao vivo, revisa o histórico, edita zonas e
  configuração. Conecta ao daemon por socket local. Fechar a janela **não** para o daemon.

Sem daemon rodando, a janela continua funcionando sozinha (motor embutido, como hoje), para não
quebrar quem só quer ver as câmeras.

## O que o código já tem a favor
- `domain/` é puro (só `domain/view.rs` usa iced) e `infrastructure/` não depende de iced.
- `ui/bridge.rs` e `ui/pipeline.rs` (2,2 mil linhas) só tocam o iced em ~5 pontos: o `Handle` de
  imagem e `Bytes` (agora o crate `bytes`).

## O que dá trabalho
A **orquestração** está em `ui/update.rs` misturada com o estado de interface (`App`):
`reconnect_camera`, `BackoffState`, `sync_active_streams`, `drain_start_queue`,
`detect_camera_motion`, `drive_motion_recording`, `toggle_camera_recording`, `push_event`,
`notify_desktop`. Extrair isso para um crate de motor, sem `App`, é o grosso do esforço.

## Partes do desenho (a decidir)
1. **Estrutura**: workspace Cargo com `rrv-core` (domain + pipeline + orquestração, sem iced),
   `rrv-daemon` e `rust-rtsp-viewer` (cliente).
2. **IPC** (comandos e eventos): socket Unix local, permissão `0600`, mensagens versionadas.
   Candidatos: JSON por linha (simples, depurável) ou gRPC (`tonic`, tipado, mais pesado).
   Recomendação: **JSON por linha**, com o "contrato de eventos" do ADR 0006.
3. **Vídeo ao vivo no cliente** (a decisão com mais consequência):
   - **A. O cliente abre a própria sessão RTSP** na câmera. Simples, mas conta como sessão extra
     (ADR 0008 / tarefa 0.3) e dobra a banda.
   - **B. O daemon redistribui** por um RTSP local (`gstreamer-rtsp-server`), uma sessão por câmera
     na câmera e N consumidores locais. Respeita o limite de sessões, centraliza credenciais.
   - **C. Memória compartilhada** com quadros decodificados. Rápido, mas acopla os dois processos.
   Recomendação: **B**, com **A** como fallback quando o daemon não está rodando.
4. **Credenciais**: com dois processos, URL com senha em texto puro piora; sobe a prioridade do
   keyring (5.3).
5. **Supervisão e recuperação**: watchdog do daemon, `Restart=on-failure`, e reconciliação de
   segmentos órfãos na partida (3.2).
6. **Config**: o `config.toml` passa a ser lido pelo daemon; o cliente edita via IPC, sem escrever
   arquivo por baixo.

## Docker (decisão do dono: o serviço roda em container, se possível)
A janela **não** vai para o container (precisa de Wayland/GPU de tela); só o daemon.

| Tema | Desenho |
|---|---|
| **Imagem** | Build em múltiplos estágios (Rust) → runtime enxuto (Debian) com GStreamer `good/bad/ugly` + `libav` e `x264`. Usuário não-root. Imagem construída no CI. |
| **Rede** | `network_mode: host` é o mais simples: RTP/UDP de volta das câmeras não atravessa bem a NAT do bridge. Alternativa: forçar RTSP sobre TCP (`protocols=tcp`) e publicar só a porta do RTSP local (8554, D5-B). |
| **Volumes** | `/data` (gravações), `/state` (SQLite, zonas), `config.toml` somente leitura. Retenção por espaço (3.3) olha o volume `/data`. |
| **Credenciais** | Nunca na imagem. Docker secrets ou variáveis do compose; keyring do SO não existe no container, então 2.5.8 vira "segredos fora do `config.toml`". |
| **IPC** | Socket Unix num volume compartilhado com o host (`/run/rrv/rrv.sock`, `0600`, mesmo UID do dono) **ou** TCP em `127.0.0.1`. Recomendação: socket Unix. |
| **GPU** | **VA-API** (Intel iGPU) via `devices: /dev/dri` + grupo `render`: possível já. **NVIDIA** exige `nvidia-container-toolkit`, **não instalado nesta máquina** (decisão D6). Sem GPU o container decodifica em CPU. |
| **Notificação de desktop** | `notify-send` não existe no container. O daemon publica o evento no IPC e a janela notifica; com a janela fechada falta canal, então **webhook/MQTT de saída (5.4) sobe de prioridade** (ou um helper leve no host). |
| **Operação** | `restart: unless-stopped`, `healthcheck` (daemon responde no IPC e cada câmera tem quadro recente), logs em stdout (`docker logs`), `TZ` definido. |
| **Windows/macOS** | Docker Desktop roda o daemon (Linux) e a janela fica nativa: **resolve** a maior parte do ADR 0001 para o motor. |

## Consequências
- Nova etapa **M2.5 — Motor sem janela** antes do M3 (ver `docs/plano-de-execucao.md`); M3 e M4
  passam a viver no daemon.
- Reintroduz `gstreamer-rtsp-server` (saíra do roadmap pelo ADR 0004).
- Entregáveis novos: `Dockerfile`, `compose.yaml`, `.dockerignore` e build da imagem no CI.
- Limite do Docker: sem `nvidia-container-toolkit`, decodificação e YOLO na GTX 1650 ficam fora do container.
- A UX/UI do cliente ganha estados novos: "daemon desconectado", "daemon iniciando", gravação
  contínua indicada mesmo com a janela fechada.
- Risco: é um refactor de ~4,6 mil linhas de UI/motor misturadas. Fazê-lo **antes** de empilhar
  M3/M4 sobre `update.rs` é bem mais barato do que depois.
