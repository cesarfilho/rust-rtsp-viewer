# Gap analysis: o que falta para o estado da arte (para executar depois)

Base: código atual (~16k linhas), `docs/status.md`, `docs/libraries.md`, `todo.md`, `todo_explain.md`.
Levantamento por inspeção (ls/grep/`cargo clippy`); nada foi compilado nem medido.

## Já em bom estado
- `cargo clippy`: sem avisos. Zero `unsafe`. 1 TODO/FIXME no código.
- Domínio puro e testado; `redact::mask_credentials`; recuperação de mutex envenenado; `.gitignore` protege `config.toml`.
- Documentação: README, CHANGELOG, AGENTS.md, Makefile, `config.toml.example` anotado, `specs/` e `docs/compose/`.

## Lacunas

### A. Engenharia e qualidade (baixo custo, alto retorno)
1. **Sem CI**: não há `.github/` (nem outro CI). Falta pipeline com `fmt`, `clippy -D warnings`, `test`.
2. **Sem `rustfmt.toml`, `clippy.toml`, `deny.toml`, `rust-toolchain.toml`**: sem `cargo-deny`/`cargo-audit` (licenças e CVEs) e sem toolchain fixada.
3. **Sem `tests/` de integração, `benches/` nem `examples/`**: só testes unitários (~310) e os de gravação em `ui::pipeline`.
4. **`cargo test --doc` quebrado** na máquina (`libLLVM.so`): toolchain local; decidir se o CI cobre doctests.
5. **~98 `.unwrap()` e 16 `.expect()`** em `src/` (contagem inclui código de teste; separar produção de teste antes de agir).
6. **13 `allow(dead_code)`**: mascaram API não usada; migrar para remover ao ligar cada módulo.
7. **Sem LICENSE** na raiz e sem `rust-version` no `Cargo.toml`.
8. **`clap` para um único argumento**: remover ou usar de fato; features `env` não usada.

### B. Dependências (ver `docs/libraries.md`)
9. `gstreamer*` 0.20 → 0.25 e `iced` 0.13 → 0.14, juntos, em branch própria.
10. `thiserror` 1 → 2, `toml` 0.8 → 1.1, `env_logger` → `tracing` (opcional).

### C. Funcionalidade (ver `todo.md`)
11. Nove módulos com lógica pronta e testada, mas desligados (`docs/status.md`): motion, zones, zone_editor, multi_stream, streaming, ptz, timelapse, hw_encoder, bidirectional_audio.
12. Sem: gravação por evento, retenção/limpeza de disco, timeline persistente, playback, notificações, detecção de objetos, PTZ real, re-streaming, API.

### D. Desempenho e robustez
13. **Decodificação por hardware não é configurada**: `decoder = "decodebin"` por padrão; só há tratamento especial de `avdec_h264/h265` em `pipeline.rs`. Falta suporte explícito a `vah264dec`/`nvh264dec` e diagnóstico de qual decoder foi escolhido.
14. **Cópia de frames RGBA** (~8 MiB/frame 1080p) pelo `videoconvert`+appsink: medir antes de otimizar (DMABuf/GL só se for gargalo).
15. **Sem métricas/benchmarks de referência**: não há números de CPU/banda por câmera para validar sub-stream e smart streaming.
16. **Sem teste de estresse** com muitas câmeras/reconexões (o `test-stream.sh`/`test_cam.sh` são manuais).
17. **Sem observabilidade**: logs por `log`/`CameraLogger`; sem spans por câmera, sem exportação de métricas.

### E. Segurança e configuração
18. **Credenciais em texto puro** em `config.toml` (URLs `rtsp://user:pass@`): considerar keyring/`secret-service` ou campos separados de usuário/senha.
19. **Sem validação de config com mensagens por campo** (o erro atual é `toml::from_str` genérico) e sem `--check`/dry-run.
20. Arquivos sensíveis/soltos na raiz: `cameras_joinville.json`, `config copy.toml`, screenshot — estão no `.gitignore`, mas o diretório não é repo git; organizar.

### F. UX, acessibilidade e distribuição
21. **Textos fixos em português**, sem camada de i18n (ex.: "sem câmeras" em `toolbar.rs`).
22. **Acessibilidade**: sem navegação por teclado completa documentada nos menus, sem alto contraste testado (existem temas, incluindo amoled).
23. **Empacotamento**: só `Makefile`/`install-deps.sh`; sem AppImage/Flatpak/`.deb`/AUR, sem releases automáticas, sem `cargo-dist`.
24. **Documentação faltante**: ADRs, specs por feature, diagrama de fluxo de dados/threads, guia de testes, formatos de dados (`view.toml`, futuro arquivo de eventos), glossário.

## Prioridade sugerida
1. A (CI, fmt/clippy/deny, LICENSE) — 1 dia, sem risco.
2. B.9 (upgrade gstreamer + iced) — isolado.
3. D.15 (linha de base de desempenho) — antes de qualquer otimização.
4. C.11 (ligar motion → zones → streaming → multi_stream).
5. E.18–19 (credenciais, validação de config).
6. D.13 (decoder por hardware).
7. C.12 e F — conforme demanda.

## Verificação
Cada item entregue: `cargo build` sem avisos, `cargo test`, `cargo clippy -- -D warnings`; para D, medir CPU/banda antes e depois.
