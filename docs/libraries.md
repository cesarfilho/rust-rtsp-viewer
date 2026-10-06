# Proposta de bibliotecas

Versões consultadas com `cargo search` em 2026-09-24. **Compatibilidade, maturidade e
licenças não foram verificadas** — cada item marcado "avaliar" precisa de um spike
antes de virar dependência. Nenhuma mudança foi feita no `Cargo.toml`.

## 1. Atualizar o que já existe

| Crate | Atual | Última | Notas |
|---|---|---|---|
| `gstreamer`, `gstreamer-app` | 0.20 | 0.25.x | Salto de várias versões; a API mudou (ex.: builders, `glib` renomeado). Ganha correções e suporte às versões novas do GStreamer. Exige revisar `pipeline.rs`/`bridge.rs` |
| `iced` | 0.13 | 0.14.0 | Breaking changes na API de `application`/widgets; `advanced::image::Bytes` (citado no AGENTS.md) pode mudar |
| `thiserror` | 1 | 2.0 | Migração simples |
| `toml` | 0.8 | 1.1 | Migração provavelmente simples |
| `env_logger` 0.10 | | | Ver item 3 |
| `clap` | 4 (`derive`,`env`) | 4.6 | Só há um argumento posicional. Pode ser trocado por `std::env::args()` e remover a dependência |

Recomendação: atualizar `gstreamer*` e `iced` **juntos numa branch própria**, antes de
qualquer feature nova, para não misturar refatoração com funcionalidade.

## 2. Para as features do `todo.md`

| Necessidade | Candidato | Por quê / risco |
|---|---|---|
| Detecção de objetos | `ort` 2.0.0-rc.13 (ONNX Runtime) | Padrão de fato para YOLO/ONNX, com aceleração (CUDA/OpenVINO/TensorRT via execution providers). Ainda é *release candidate*. Alternativa 100% Rust: `tract-onnx` 0.23 (sem dependência nativa, mais lento). `ndarray` para pré/pós-processamento |
| Modelos prontos | `usls` 0.2.0-alpha | Coleção de modelos sobre ONNX Runtime; é *alpha* — avaliar só como referência |
| Movimento mais robusto (subtração de fundo, contornos) | `opencv` 0.100 | Poderoso, mas exige OpenCV instalado e compilação lenta. Só se a diferença de luma atual (`motion.rs`) não bastar; para o caso simples, **manter o código próprio** |
| Descoberta ONVIF | `oxvif` 0.17 | Cliente ONVIF assíncrono (só descoberta e cadastro; PTZ foi removido do plano). Maturidade não avaliada. Evitar `onvif-rs`. |
| Timeline/eventos persistentes | `rusqlite` 0.40 | Consultas por câmera/intervalo, relação evento → segmento gravado. Alternativa mais simples: JSONL append-only, sem dependência |
| MQTT (Home Assistant) | `rumqttc` 0.25 | Cliente MQTT assíncrono, o mais usado |
| API HTTP / eventos | `axum` 0.8 | Sobre `tokio`, que já é dependência via iced |
| Re-streaming RTSP | `gstreamer-rtsp-server` 0.25 | Já dentro do ecossistema GStreamer, sem processo externo. Alternativa: rodar `go2rtc`/MediaMTX como sidecar |
| WebRTC / two-way talk | `gstreamer-webrtc` 0.25 | Idem. Two-way talk via RTSP backchannel é outro caminho, precisa de investigação |
| Notificações desktop | `notify-rust` 4.18 | D-Bus puro em Rust |
| Espaço em disco (limpeza de gravações) | `sysinfo` 0.39 | Alternativa menor: `rustix::fs::statvfs` (Linux/Unix). Escolher conforme se Windows importa |
| Logs estruturados | `tracing` + `tracing-subscriber` | Spans por câmera/pipeline ajudam a depurar reconexões. Hoje usa `log` + `CameraLogger` próprio; migrar é opcional |

## 3. O que NÃO recomendo
- **`retina`** (cliente RTSP puro em Rust): substituiria `rtspsrc`, mas você perderia
  jitterbuffer, retransmissão e a integração com os decoders/HW do GStreamer. O AGENTS.md
  diz que o cache/latência é inteiramente do GStreamer; manter assim.
- **`wgpu` direto** para renderizar vídeo: o iced já usa wgpu; só faria sentido se o custo
  de copiar frames RGBA (~8 MiB por frame 1080p) virar gargalo medido. Antes disso, medir.
- **`opencv`** como dependência geral, pelo custo de build/instalação.

## 4. Ordem sugerida
1. Spike: subir `gstreamer` 0.25 + `iced` 0.14 numa branch (`cargo build` sem warnings, 310 testes verdes).
2. Persistência de eventos: começar com JSONL; migrar para `rusqlite` se as consultas pedirem.
3. `sysinfo`/`rustix` para limpeza por espaço.
4. `ort` em um spike isolado: rodar YOLO 320x320 num frame estático e medir tempo por inferência.
5. `oxvif` para a descoberta ONVIF (sem PTZ).
6. `rumqttc`/`axum`/`gstreamer-rtsp-server` só com demanda de integração real.
