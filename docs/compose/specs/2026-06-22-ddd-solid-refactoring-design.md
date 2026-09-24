# Refatoração DDD/SOLID/TDD — Design Spec

## [S1] Problem

O projeto `rust-rtsp-viewer` funciona mas viola princípios SOLID e DDD:
- **Domain impuro**: `domain/recording.rs` contém I/O (`fs`, `env`, `SystemTime`); `domain/source.rs` depende de `config::CameraConfig` (infraestrutura).
- **Enums em vez de polimorfismo**: `Camera` enum exige match exhaustivo — adicionar tipo requer modificar todos os callers (OCP).
- **God modules**: `CameraSlot` (928 linhas, 3 responsabilidades), `nvr/mod.rs` (1489 linhas, 6+ responsabilidades).
- **Sem testes**: 0 testes em nvr, camera_slot, gtk_app — lógica testável não testada.

## [S2] Solution Overview

Refatoração incremental em 4 fases, cada uma commitável e testável:
1. Domain purity (DIP + SRP)
2. Polimorfismo com traits (OCP + LSP + ISP)
3. Split monolitos (SRP)
4. TDD coverage

## [S3] Phase 1 — Domain Purity

### [S3.1] Remove I/O from domain/recording.rs

**Atual:** `ensure_dir_exists()` (line ~509), `generate_filename()` (line ~482) chamam `std::fs` e `std::env::var`.

**Mudança:**
- Domain define apenas `RecordingConfig` (value object) e `RecordingState`.
- `generate_filename` → infrastructure (ou recebe `&Path` como parâmetro).
- `ensure_dir_exists` → infrastructure.
- Domain contém apenas lógica pura: `format_duration()`, `elapsed_secs()`, `should_rotate()`.

### [S3.2] Replace config dependency in domain/source.rs

**Atual:** `source.rs:3` importa `crate::config::CameraConfig`.

**Mudança:**
- Criar `domain::source::SourceDescriptor` como value object puro:
  ```rust
  pub struct SourceDescriptor {
      pub kind: SourceKind,
      pub url: String,
      pub latency_ms: u32,
      pub cache_seconds: u32,
      pub decoder: String,
      pub use_uridecodebin: bool,
      pub do_retransmission: bool,
  }
  ```
- `SourceConfig::from_camera_config` → `SourceConfig::from_descriptor`.
- Infrastructure faz a conversão `CameraConfig → SourceDescriptor`.

### [S3.3] Tests

- `RecordingConfig::default()` — verifica defaults.
- `format_duration()` — testa formatação HH:MM:SS.
- `SourceDescriptor` — testa detecção de tipo (RTSP vs HLS vs File).

## [S4] Phase 2 — Polimorfismo

### [S4.1] CameraSource trait

```rust
pub trait CameraSource {
    fn label(&self) -> &str;
    fn metrics(&self) -> Arc<Metrics>;
    fn pipeline(&self) -> &gst::Pipeline;
    fn is_live(&self) -> bool;
    fn audio_volume(&self) -> Option<f32>;
    fn recording_session(&self) -> &Mutex<RecordingSession>;
    fn toggle_recording(&self);
}
```

- `RtspCamera`, `HlsCamera`, `FileCamera` implementam `CameraSource`.
- `Camera` enum → `Box<dyn CameraSource>` (ou enum com trait delegation).
- Elimina match exhaustivo em nvr/mod.rs, keyboard.rs, tick logic.

### [S4.2] OverlayRenderer trait

```rust
pub trait OverlayRenderer {
    fn render(&self, metrics: &Metrics, diagnostics: &[Hint]) -> OverlayState;
}
```

- `FullOverlay` e `CompactOverlay` implementam.
- Substitui `match layout { Full => ..., Compact => ... }`.

### [S4.3] LayoutStrategy trait

```rust
pub trait LayoutStrategy {
    fn arrange(&self, cells: &[CameraCell], active: &[bool], selected: Option<usize>);
}
```

- `GridLayout` e `FlexLayout` implementam.

### [S4.4] Tests

- Mock `CameraSource` para testar tick logic sem pipeline real.
- Testar `FullOverlay::render()` e `CompactOverlay::render()`.

## [S5] Phase 3 — Split Monolitos

### [S5.1] Split CameraSlot (928 → 4 módulos)

| Novo módulo | Responsabilidade | Linhas estimadas |
|-------------|-----------------|:----------------:|
| `camera/pipeline.rs` | Pipeline lifecycle, cache, state | ~250 |
| `camera/recording_ctrl.rs` | Recording start/stop/toggle | ~80 |
| `camera/ui_state.rs` | Badges, labels, overlays | ~150 |
| `camera/mod.rs` | Struct CameraSlot + delegação | ~100 |

### [S5.2] Split nvr/mod.rs (1489 → 6 módulos)

| Novo módulo | Responsabilidade | Linhas estimadas |
|-------------|-----------------|:----------------:|
| `nvr/cell.rs` | CameraCell struct + builder | ~150 |
| `nvr/sidebar.rs` | SidebarRow struct + builder | ~150 |
| `nvr/info_panel.rs` | InfoPanel struct + update | ~200 |
| `nvr/tick.rs` | TickContext + tick loop | ~300 |
| `nvr/layout.rs` | arrange(), calc_grid() | ~100 |
| `nvr/mod.rs` | run_main_window orchestrator | ~200 |

### [S5.3] Tests

- Testar `calc_grid()` isoladamente.
- Testar `PipelineManager::cache_delta()`.

## [S6] Phase 4 — TDD Coverage

### [S6.1] Domain tests (já existentes, expandir)

- `diagnostics.rs`: ~30 testes (manter).
- `overlay.rs`: ~40 testes (manter).
- `recording.rs`: adicionar testes para `format_duration`, `should_rotate`.
- `snapshot.rs`: ~27 testes (manter).

### [S6.2] Infrastructure tests (novos)

- `reconnect.rs`: Testar `ReconnectState::tick()` com cenários:
  - FPS=0 por 15s → Reconnect
  - FPS=0 por 10s → None
  - Backoff due → Reconnect
  - uridecodebin → None (skip watchdog)
- `cache_controller.rs`: Testar adaptive cache logic:
  - Q > 85% por 2 ticks → grow
  - Q < 25% → shrink
  - Error burst → grow
- `camera_slot.rs`: Testar `get_queue_cache()`, `get_queue_pct()`.

### [S6.3] Meta

- Cobertura mínima: 80% em domain, 60% em infrastructure lógica.
- Todos os testes devem ser unitários (sem GTK/GStreamer real).

## [S7] Migration Strategy

- Cada fase é um PR separado.
- Fase 1 e 2 podem ser feitas em paralelo (independentes).
- Fase 3 depende de Fase 2 (traits precisam existir antes do split).
- Fase 4 pode ser feita após qualquer fase (adicionar testes incrementalmente).
- Manter todos os 282 testes existentes passando após cada fase.
- `cargo build` deve compilar sem erros após cada commit.
