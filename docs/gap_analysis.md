# Gap analysis: o que falta para o estado da arte

Revisado em 2026-10-05 (v0.8.0). Base: código atual (~18 mil linhas, 387 testes),
`docs/status.md`, `docs/libraries.md`, ADRs e uma pesquisa de mercado (seção final).
Legenda: ✅ feito · 🟡 parcial · ❌ aberto.

## Já em bom estado
- `cargo clippy --all-targets -- -D warnings` limpo; zero `unsafe`; zero `unwrap()`/`expect()`
  fora de testes; 1 `TODO` no código.
- Domínio puro e testado; `redact::mask_credentials` em todo log com URL (inclusive nos
  pipelines de áudio, corrigido na 0.7.2); `quote_launch_value` em todo `parse_launch`.
- CI (`.github/workflows/ci.yml`: clippy + testes), LICENSE AGPL-3.0, `rust-version = 1.88`.
- Documentação: README, CHANGELOG, AGENTS.md, 9 ADRs, spec de zonas, `config.toml.example`.
- Validação de interface feita na prática em 2026-10-05 (grade, flex, sidebar, spotlight,
  menu de contexto, editor de zonas, snapshot, gravação, atalhos, `Ctrl+Q`) com fontes HLS
  sintéticas. **Não** houve teste com RTSP real nem com 16 câmeras.

## Lacunas

### A. Engenharia e qualidade
| # | Item | Estado |
|---|---|---|
| 1 | CI | ✅ clippy + `cargo test --lib --bins` em Ubuntu 24.04. Falta `cargo fmt --check` (o componente está instalado, mas não é executado) |
| 2 | `rustfmt.toml`, `clippy.toml`, `deny.toml`, `rust-toolchain.toml` | ❌ nenhum existe; sem `cargo-deny`/`cargo-audit` |
| 3 | Testes de integração, `benches/`, `examples/` | ❌ só testes unitários (os de gravação rodam GStreamer real, ~6 s) |
| 4 | `cargo test --doc` | ✅ funciona (a nota antiga sobre `libLLVM` era do toolchain local, já corrigido) |
| 5 | `unwrap`/`expect` | ✅ nenhum fora de testes |
| 6 | `allow(dead_code)` | 🟡 10 restantes (lista em `docs/status.md`) |
| 7 | LICENSE e `rust-version` | ✅ |
| 8 | `clap` para um único argumento | ❌ |

### B. Dependências (ver `docs/libraries.md`)
| # | Item | Estado |
|---|---|---|
| 9 | `gstreamer*` 0.20 → 0.25 e `iced` 0.13 → 0.14, juntos, em branch própria | ❌ (o GStreamer instalado nesta máquina é o 1.28.6) |
| 10 | `thiserror` 2, `toml` 1, `env_logger` → `tracing` | ❌ |

### C. Funcionalidade
| # | Item | Estado |
|---|---|---|
| 11 | Módulos desligados | 🟡 ligados: `motion`, `zones` + editor, `multi_stream`. Faltam `streaming`, `timelapse`, `hw_encoder`, `bidirectional_audio` |
| 12 | Gravação por evento | ✅ `[recording] on_motion` com pós-roll. ❌ sem pré-roll (ADR 0007) |
| 12 | Notificações | ✅ `notify-send` com cooldown |
| 12 | Retenção e limpeza de disco | ❌ (só os logs têm retenção) |
| 12 | Timeline persistente, busca e reprodução | ❌ o timeline é só memória; não há player embutido |
| 12 | Detecção de objetos/pessoas | ❌ (ADR 0003) |
| 12 | ONVIF (descoberta), re-streaming, API/MQTT | ❌ (PTZ removido do plano em 2026-10-06) |
| 12 | Sub/main stream | ✅ `sub_url` (0.8.0) |

### D. Desempenho e robustez
| # | Item | Estado |
|---|---|---|
| 13 | Decodificação por hardware | ❌ **hoje é 100% CPU** (`decodebin` escolhe `avdec_h264`). O Inspector agora mostra o decoder e se roda em CPU/GPU. Ver "Medições de GPU" abaixo: trocar o decoder sozinho **piora** |
| 14 | Cópia de frames RGBA | ❌ continua sendo o gargalo (ver medições) |
| 15 | Baseline de desempenho | 🟡 há medições de decode (abaixo) e `scripts/baseline.sh`, mas nada com câmeras reais |
| 16 | Teste de estresse com muitas câmeras/reconexões | ❌ |
| 17 | Observabilidade | 🟡 diagnóstico por câmera na UI (FPS, bitrate comprimido, jitter, perda, decoder); sem spans nem exportação de métricas |
| — | Reconexão de câmera que falha logo após iniciar | ✅ corrigido na 0.7.2 (antes ficava em "Reconectando" para sempre) |
| — | Áudio usa a URL principal mesmo com a câmera no sub | ❌ abre uma sessão extra na câmera |

### E. Segurança e configuração
| # | Item | Estado |
|---|---|---|
| 18 | Credenciais em texto puro no `config.toml` | ❌ (keyring/`secret-service`); logs e erros já mascaram |
| 19 | Validação de config com mensagem por campo, `--check` | ✅ `config_check` + `--check` (typos, faixas, URLs, duplicatas, grupos, tema) |
| 20 | Arquivos sensíveis na árvore | ✅ `config.toml` e settings locais no `.gitignore`; planos de ferramentas locais saíram do versionamento |

### F. UX, acessibilidade e distribuição
| # | Item | Estado |
|---|---|---|
| 21 | i18n | ❌ textos fixos em português |
| 22 | Acessibilidade | 🟡 temas com teste de contraste WCAG; ajuda modal e navegação por teclado; sem leitor de tela |
| 23 | Empacotamento | 🟡 `Makefile`, `.desktop` e ícone SVG; sem AppImage/Flatpak/`.deb`/AUR nem releases automáticas |
| 24 | Documentação | ✅ ADRs, specs, AGENTS.md; ❌ diagrama de fluxo de dados/threads |

## Medições de GPU (2026-10-05)

Máquina: Intel UHD (TigerLake, renderiza a UI) + NVIDIA GTX 1650 4 GB (driver 610.57,
plugin `nvcodec` presente). GStreamer 1.28.6. `gst-plugin-va` **não** instalado (sem
`vah264dec`). ONNX Runtime, CUDA e cuDNN **não** instalados.

600 frames de 1080p30 H.264 4:2:0 sintético (20 s de vídeo), CPU-segundos (user+sys) por
stream. Vídeo sintético e fácil de comprimir: câmeras reais custam mais na CPU.

| Caminho | CPU (s) |
|---|---|
| **Atual:** `avdec_h264` + `videoconvert` para RGBA | 2,27 |
| `avdec_h264`, só I420 | 1,55 |
| `nvh264dec`, sem baixar o frame | 0,32 |
| `nvh264dec` + baixar em NV12 | 0,94 |
| `nvh264dec` + `glcolorconvert` + `gldownload` RGBA | 1,59 |
| **`nvh264dec` + `videoconvert` RGBA (CPU)** | **4,5 a 4,9** |

Conclusões:
- O gargalo é **trazer o frame de volta à CPU em RGBA** para o `iced::image` (iced 0.13
  só aceita bytes na CPU), não o decode.
- Trocar para `nvh264dec` mantendo o caminho atual **dobra** o uso de CPU. Não configurar
  `decoder = "nvh264dec"` por enquanto.
- Ganho real exige baixar em **NV12** e converter em shader (widget wgpu próprio) ou
  zero-copy. Neste notebook híbrido o app renderiza na Intel e a NVDEC está na NVIDIA, então
  zero-copy exigiria renderizar na NVIDIA (PRIME offload).
- NVDEC não decodifica H.264 4:4:4 (a Turing só suporta 4:2:0); câmeras reais usam 4:2:0.
- Alternativa barata: instalar `gst-plugin-va` e decodificar na iGPU Intel (Quick Sync),
  deixando a NVIDIA livre para IA.
- Não medido: VRAM com 16 streams (4 GB é o limite; sub-stream ajuda), ms/inferência.

## Comparação com o estado da arte (pesquisa de 2026-10-05)

Fontes de terceiros; nada foi testado na prática.

| Categoria | Referências | Onde este app está |
|---|---|---|
| NVR open source com IA | Frigate 0.17 (classificação treinada localmente, GenAI nos eventos, retenção em camadas, config pela UI), Viseron, ZoneMinder, Shinobi | Atrás em IA de objetos, histórico e acesso remoto |
| VMS comercial | Blue Iris, Milestone XProtect, Genetec, UniFi Protect | Atrás em busca unificada, linha do tempo e video wall corporativo |
| Gateways de stream | go2rtc (WebRTC < 1 s, MSE ~0,5 s), Scrypted | Não é gateway; consome RTSP/HLS direto |
| NVR minimalista em Rust | Moonfire NVR (grava sem decodificar, SQLite, MP4 por intervalo) | Referência para a gravação sem reencode (ADR 0007) |
| Padrões | ONVIF Profile T (base de 2026), S (aposentado após 31/03/2027), M (metadados de analytics) | Sem ONVIF: só URL manual |

Pontos fortes: latência de visualização nativa, diagnóstico por câmera dentro da UI,
robustez (zero `unsafe`, credenciais mascaradas, EOS correto na gravação), zonas de movimento
com editor visual.

Decisão de posicionamento em aberto: competir com o Frigate em IA é caro; um nicho mais
defensável é ser o melhor **video wall nativo e leve**, consumindo eventos de fora.

## Prioridade sugerida
1. Decisão de posicionamento (video wall × NVR) — define o resto.
2. `cargo fmt --check` no CI, `deny.toml`, `rust-toolchain.toml` — baixo custo.
3. Medir com câmeras RTSP reais (decode, sub/main, reconexão) antes de otimizar.
4. Decoder de hardware: `gst-plugin-va` (simples) ou NV12 + shader (grande).
5. SQLite de eventos + reprodução (ADR 0006), depois detecção de objetos (ADR 0003).
6. ONVIF (descoberta), credenciais no keyring, validação de config.
7. Upgrade `gstreamer`/`iced` em branch própria.

## Verificação
Cada item entregue: `cargo build` sem avisos, `cargo test`, `cargo clippy --all-targets -- -D warnings`;
para itens de desempenho, medir CPU/banda/VRAM antes e depois.

Fontes: [Frigate 0.17](https://github.com/blakeblackshear/frigate/discussions/22137),
[comparação Frigate × Blue Iris × Scrypted](https://www.privacysmarthome.com/guides/frigate-vs-blue-iris-vs-scrypted-best-local-nvr-2026/),
[go2rtc](https://go2rtc.com/), [Moonfire NVR](https://github.com/scottlamb/moonfire-nvr),
[ONVIF Profiles](https://www.forasoft.com/blog/article/onvif-profiles-in-security-systems),
[Milestone XProtect 2026 R1](https://www.ifovea.com/milestone-xprotect-2026-r1-cloud-vms-alternative/),
[nvh264dec + udmabuf](https://github.com/museslabs/phonto/issues/36).
