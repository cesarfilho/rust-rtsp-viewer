# Gap analysis: o que falta para o estado da arte

Revisado em 2026-10-06 (substitui a versão de 2026-10-05, que descrevia o app de antes do daemon e do M3). Legenda: ✅ feito ·
🟡 parcial · ❌ aberto. O que fazer com cada ❌ está em `docs/plano-restante.md`.

## Situação por categoria

### A. Engenharia e qualidade
| Item | Estado |
|---|---|
| CI (fmt, clippy `-D warnings`, testes, `cargo deny`, MSRV) | ✅ |
| `deny.toml`, `rust-toolchain.toml` | ✅ (`rustfmt.toml`/`clippy.toml` não são necessários) |
| Testes de integração (daemon, canal de controle, reconexão, estresse de 1 h) | ✅ |
| `unwrap`/`expect` fora de testes; `allow(dead_code)` | ✅ nenhum fora de testes; zero `allow(dead_code)` |
| Módulos sem chamador | ✅ removidos (`streaming`, `timelapse`, `bidirectional_audio`, `hw_encoder`, `ptz`) |
| Diagrama de fluxo de dados/threads | ❌ |

### B. Dependências
| Item | Estado |
|---|---|
| `gstreamer*` 0.25 | ✅ na master |
| `iced` 0.14 | 🟡 spike pronto em `spike/deps-upgrade`; falta rebase e validação na tela (fase B) |
| `thiserror` 2, `toml` 1, `env_logger` → `tracing` | ❌ sem urgência |

### C. Funcionalidade
| Item | Estado |
|---|---|
| Gravação por evento com pré-roll e pós-roll; sem reencode (RTSP H.264/H.265) | ✅ |
| Áudio na gravação | ✅ opcional (`record_audio`); AAC em mkv/mp4, G.711 só em mkv |
| Histórico persistente (SQLite), retenção, aviso de disco | ✅ |
| Reprodução, linha do tempo, vários canais, clipes | ✅ |
| Notificações (desktop e webhook) | ✅ |
| Sub/main stream | ✅ |
| Detecção de objetos/pessoas | ❌ (fase C) |
| ONVIF (descoberta), MQTT/Home Assistant | ❌ (fase D); PTZ fora do plano |
| Re-streaming, API HTTP | ❌ sem demanda |

### D. Desempenho e robustez
| Item | Estado |
|---|---|
| Decodificação por hardware | ✅ no daemon pela iGPU Intel (CPU 88% → 47%); NVIDIA preparada, não testada |
| Conversão RGBA na janela | ❌ é o que resta de custo na janela (2.3 NV12 + shader, fase B) |
| Daemon sem RGBA | ✅ CPU −57% com 11 câmeras |
| Baseline com câmeras reais | ✅ 1, 4 e 16 câmeras (`docs/baseline.md`) |
| Teste de estresse | ✅ 1 h, 91 mil reconexões, 16 fontes, sem vazamento |
| Observabilidade | 🟡 diagnóstico por câmera na UI e `rrvctl status`; sem exportação de métricas |

### E. Segurança e configuração
| Item | Estado |
|---|---|
| Segredos fora do `config.toml` no daemon (`${NOME}`, Docker secrets) | ✅ |
| Senhas da janela no chaveiro | ❌ (fase D) |
| Validação de config por campo, `--check` | ✅ |
| Credenciais mascaradas em logs/erros | ✅ |
| Socket de controle `0600`, protocolo versionado | ✅ |

### F. UX, acessibilidade e distribuição
| Item | Estado |
|---|---|
| i18n | ❌ textos em português no código (fase D) |
| Acessibilidade | 🟡 contraste WCAG por tema, teclado; sem leitor de tela |
| Empacotamento | 🟡 imagem Docker e `Makefile`/`.desktop`; falta AUR, AppImage/Flatpak e releases automáticas (fase D) |

## Medições
GPU e CPU: `docs/gpu-container.md` e `docs/baseline.md`. O que a medição ensinou: o custo dominante era a **conversão de cada
quadro para RGBA**, não o H.264; tirá-la do daemon cortou 57% da CPU, e só depois a GPU passou a render (outros 47%). A janela
ainda converte, e é por isso que a 2.3 existe.

## Comparação com o estado da arte (pesquisa de 2026-10-05; fontes de terceiros)
| Categoria | Referências | Onde este app está |
|---|---|---|
| NVR open source com IA | Frigate, Viseron, ZoneMinder, Shinobi | À frente em latência de visualização e diagnóstico por câmera; atrás em IA de objetos e acesso remoto |
| VMS comercial | Blue Iris, Milestone, UniFi Protect | Atrás em busca unificada e video wall corporativo |
| Gateways de stream | go2rtc, Scrypted | Não é gateway |
| NVR minimalista em Rust | Moonfire NVR (grava sem decodificar, SQLite) | Mesma abordagem de gravação, com janela nativa |
| Padrões | ONVIF Profile T (base de 2026) | Sem ONVIF; falta a descoberta |

Fontes: [Frigate 0.17](https://github.com/blakeblackshear/frigate/discussions/22137),
[go2rtc](https://go2rtc.com/), [Moonfire NVR](https://github.com/scottlamb/moonfire-nvr),
[ONVIF Profiles](https://www.forasoft.com/blog/article/onvif-profiles-in-security-systems).
