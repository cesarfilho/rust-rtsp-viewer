//! A vista "Gravações" (plano 3.4/3.5, spec `docs/specs/ux-historico.md`): linha do tempo
//! por câmera, reprodução embutida e exportar clipe. Tudo vem do daemon pelo canal; os
//! arquivos são lidos da pasta de gravações **desta** janela (`[recording] dir`), que no
//! Docker é a pasta do host montada no contêiner.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use iced::widget::{button, canvas, column, container, row, text};
use iced::{Element, Length, Point, Rectangle, Renderer, Size, Task, mouse};

use crate::domain::timeline_view::{self, SegmentSpan, Span};
use crate::ipc::protocol::{HistoryEvent, Request, Response, SegmentInfo, WireBox};
use crate::ui::app::App;
use crate::ui::bridge::GStreamerBridge;
use crate::ui::daemon::{DaemonState, PendingRequest};
use crate::ui::message::Message;
use crate::ui::theme::{Theme, ThemeColors};
use crate::ui::video_widget::VideoWidget;
use crate::ui::view::style::{self, Intent};

const LANE_H: f32 = 34.0;
const AXIS_H: f32 = 22.0;
const GUTTER: f32 = 96.0;
/// Quanto cada `←`/`→` pula.
const SKIP_MS: i64 = 10_000;
/// Um evento abre a gravação alguns segundos antes dele, para ver o que o causou.
const EVENT_LEAD_MS: i64 = 5_000;
/// Largura da coluna de eventos.
const EVENTS_W: f32 = 270.0;

#[derive(Debug, Clone)]
pub enum RecMsg {
    Open,
    Close,
    /// Pedido de histórico: as horas olhadas para trás.
    SetSpan(i64),
    Pan(f32),
    Zoom {
        factor: f32,
        anchor: f32,
    },
    /// Clique na linha do tempo: faixa (índice em `lanes`) e instante (Unix ms).
    Clicked {
        lane: usize,
        t_ms: i64,
    },
    PlayPause,
    Skip(i64),
    Rate(f64),
    Step,
    MarkIn,
    MarkOut,
    Export,
    /// Protege / solta o segmento que está tocando.
    ToggleProtect,
    /// Clicou num evento da lista: toca de 5 s antes dele.
    EventClicked {
        camera: String,
        ts_ms: i64,
    },
    /// Mostra só os eventos de movimento.
    ToggleMotionOnly,
    /// Passa ao próximo objeto da lista (nenhum → pessoa → carro → … → nenhum): só as detecções dele.
    CycleLabel,
    /// Adiciona / tira uma câmera da comparação (canais lado a lado, no mesmo instante).
    ToggleCompare(String),
    Live,
}

/// O que está tocando.
pub struct Player {
    pub camera: String,
    pub segment: SegmentInfo,
    pub bridge: Arc<Mutex<GStreamerBridge>>,
    pub video: VideoWidget,
    pub paused: bool,
    pub rate: f64,
    /// O seek pedido antes de o arquivo informar a duração; tenta de novo a cada tick.
    pub pending_seek_ms: Option<u64>,
    /// Um canal seguidor parado de propósito porque o principal chegou ao fim (e não porque a
    /// pessoa pausou): volta a tocar quando o principal volta a andar.
    pub held: bool,
}

/// Um canal que acompanha o principal: a mesma hora, outra câmera.
pub struct Follower {
    pub camera: String,
    pub player: Option<Player>,
    /// Por que não há imagem (arquivo ausente...), para o quadro mostrar.
    pub note: Option<String>,
    /// Depois de uma falha, só tenta de novo a partir daqui (não martela o disco a 10 Hz).
    pub retry_at: Option<std::time::Instant>,
}

/// Quantos canais cabem lado a lado (o principal e mais três).
pub const MAX_CHANNELS: usize = 4;

pub struct RecordingsView {
    pub span: Span,
    pub loading: bool,
    pub error: Option<String>,
    pub segments: Vec<SegmentInfo>,
    pub events: Vec<HistoryEvent>,
    /// Uma faixa por câmera do daemon.
    pub lanes: Vec<String>,
    pub player: Option<Player>,
    pub mark_in: Option<i64>,
    pub mark_out: Option<i64>,
    pub truncated: bool,
    /// Filtro da lista de eventos: só movimento.
    pub motion_only: bool,
    /// Filtro da lista de eventos: só as detecções deste objeto (nome COCO).
    pub label_filter: Option<String>,
    /// A janela acompanha o vivo: novos segmentos entram sozinhos.
    pub follow: bool,
    /// Os canais de comparação, que seguem o instante do principal.
    pub followers: Vec<Follower>,
    /// Quando o histórico foi pedido pela última vez (para o refresco automático).
    pub refreshed_at: std::time::Instant,
}

/// De quanto em quanto tempo a vista aberta pergunta ao daemon se há segmentos novos.
const REFRESH_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

impl RecordingsView {
    /// Onde a cabeça de reprodução está (Unix ms) e em qual faixa.
    pub fn playhead(&self) -> Option<(usize, i64)> {
        let p = self.player.as_ref()?;
        let lane = self.lanes.iter().position(|l| *l == p.camera)?;
        // While a seek waits for the file to report its duration, the position is still the
        // start of the file: the instant the user asked for is the real playhead.
        let pos = match p.pending_seek_ms {
            Some(ms) => ms,
            None => p
                .bridge
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .playback_position_ms()
                .unwrap_or(0),
        } as i64;
        Some((lane, p.segment.ts_start + pos))
    }

    /// O principal e os canais que acompanham, todos os players abertos.
    pub fn players_mut(&mut self) -> impl Iterator<Item = &mut Player> {
        self.player
            .iter_mut()
            .chain(self.followers.iter_mut().filter_map(|f| f.player.as_mut()))
    }

    fn spans_of(&self, camera: &str) -> Vec<(usize, SegmentSpan)> {
        self.segments
            .iter()
            .enumerate()
            .filter(|(_, s)| s.camera == camera)
            .map(|(i, s)| {
                (
                    i,
                    SegmentSpan {
                        start: s.ts_start,
                        end: s.ts_end,
                        motion: s.has_motion,
                        protected: s.protected,
                    },
                )
            })
            .collect()
    }
}

// ───────────────────────────── update ─────────────────────────────

pub fn update(app: &mut App, msg: RecMsg) -> Task<Message> {
    match msg {
        RecMsg::Open => open(app),
        RecMsg::Close => {
            if let Some(v) = app.recordings.as_mut() {
                if let Some(p) = v.player.take() {
                    p.bridge.lock().unwrap_or_else(|e| e.into_inner()).stop();
                }
                for f in v.followers.drain(..) {
                    if let Some(p) = f.player {
                        p.bridge.lock().unwrap_or_else(|e| e.into_inner()).stop();
                    }
                }
            }
            app.recordings = None;
        }
        RecMsg::SetSpan(hours) => {
            if let Some(v) = app.recordings.as_mut() {
                v.span = Span::last_hours(now_ms(), hours);
                v.follow = true;
            }
            request_history(app);
        }
        RecMsg::Pan(f) => {
            if let Some(v) = app.recordings.as_mut() {
                v.span = v.span.panned(f).clamped_to(now_ms());
                v.follow = v.span.is_following(now_ms());
            }
            request_history(app);
        }
        RecMsg::Zoom { factor, anchor } => {
            if let Some(v) = app.recordings.as_mut() {
                v.span = v.span.zoomed(factor, anchor).clamped_to(now_ms());
                v.follow = v.span.is_following(now_ms());
            }
            request_history(app);
        }
        RecMsg::Clicked { lane, t_ms } => click(app, lane, t_ms),
        RecMsg::PlayPause => {
            if let Some(v) = app.recordings.as_mut()
                && let Some(master) = v.player.as_ref()
            {
                let paused = !master.paused;
                for p in v.players_mut() {
                    p.paused = paused;
                    let _ = p
                        .bridge
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .set_playback_paused(paused);
                }
            }
        }
        RecMsg::Skip(ms) => skip(app, ms),
        RecMsg::Rate(r) => {
            if let Some(v) = app.recordings.as_mut() {
                for p in v.players_mut() {
                    p.rate = r;
                    let _ = p
                        .bridge
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .set_playback_rate(r);
                }
            }
        }
        RecMsg::Step => {
            if let Some(p) = app.recordings.as_mut().and_then(|v| v.player.as_mut()) {
                p.paused = true;
                let _ = p
                    .bridge
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .step_frame();
            }
        }
        RecMsg::MarkIn => mark(app, true),
        RecMsg::MarkOut => mark(app, false),
        RecMsg::Export => export(app),
        RecMsg::ToggleProtect => toggle_protect(app),
        RecMsg::ToggleMotionOnly => {
            if let Some(v) = app.recordings.as_mut() {
                v.motion_only = !v.motion_only;
            }
        }
        RecMsg::CycleLabel => {
            if let Some(v) = app.recordings.as_mut() {
                let labels = detection_labels(&v.events);
                v.label_filter = next_label(v.label_filter.as_deref(), &labels);
            }
        }
        RecMsg::EventClicked { camera, ts_ms } => {
            let lane = app
                .recordings
                .as_ref()
                .and_then(|v| v.lanes.iter().position(|l| *l == camera));
            if let Some(lane) = lane {
                click(app, lane, ts_ms - EVENT_LEAD_MS);
            }
        }
        RecMsg::ToggleCompare(camera) => toggle_compare(app, camera),
        RecMsg::Live => return update(app, RecMsg::Close),
    }
    Task::none()
}

/// Why the recordings view cannot open now, and what to do about it. `None` when it can.
/// The history lives in the daemon, so this depends on the mode the window is in.
pub(crate) fn unavailable_reason(daemon: &DaemonState) -> Option<String> {
    use super::daemon::Mode;
    match daemon.mode {
        Mode::Connected => None,
        Mode::Embedded => Some(format!(
            "A vista Gravações precisa do daemon, e esta janela está no motor local (o chip da barra diz \
             \"Motor local\"). Abra a janela ligada ao daemon: RRV_SOCKET={} ou --daemon <socket>",
            daemon.socket.display()
        )),
        Mode::Connecting => Some("Conectando ao daemon… tente de novo em instantes".into()),
        Mode::Lost => Some(
            "Sem contato com o daemon agora (veja o aviso sob a barra); a vista Gravações volta quando ele responder".into(),
        ),
        Mode::Incompatible => Some(
            "A janela e o daemon falam versões diferentes do protocolo; atualize um dos dois".into(),
        ),
        Mode::NoPermission => Some(
            "Sem permissão para o socket do daemon (é de outro usuário?); a vista Gravações precisa dele".into(),
        ),
    }
}

fn open(app: &mut App) {
    if let Some(why) = unavailable_reason(&app.daemon) {
        super::update::toast(app, why);
        return;
    }
    let lanes: Vec<String> = app.daemon.cameras.iter().map(|c| c.name.clone()).collect();
    app.recordings = Some(RecordingsView {
        span: Span::last_hours(now_ms(), 6),
        loading: true,
        error: None,
        segments: Vec::new(),
        events: Vec::new(),
        lanes,
        player: None,
        mark_in: None,
        mark_out: None,
        truncated: false,
        motion_only: false,
        label_filter: None,
        follow: true,
        followers: Vec::new(),
        refreshed_at: std::time::Instant::now(),
    });
    request_history(app);
}

fn request_history(app: &mut App) {
    let Some(span) = app.recordings.as_ref().map(|v| v.span) else {
        return;
    };
    if let Some(v) = app.recordings.as_mut() {
        // Com dados na tela o refresco é silencioso (sem piscar "Carregando…").
        v.loading = v.segments.is_empty();
        v.refreshed_at = std::time::Instant::now();
    }
    super::update::send_to_daemon(
        app,
        Request::History {
            camera: None,
            from_ms: span.from,
            to_ms: span.to,
        },
        PendingRequest::History,
    );
}

/// Where the daemon's recordings may be on this machine besides `[recording] dir`: the folder the
/// compose file mounts (`RRV_RECORDINGS`, default `./recordings`) and the usual video folders.
fn candidate_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os("RRV_RECORDINGS") {
        v.push(PathBuf::from(d));
    }
    v.push(PathBuf::from("recordings"));
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        v.push(home.join("Videos"));
        v.push(home.join("Vídeos"));
    }
    v
}

/// The first of `current` and `candidates` that has the recorded file `file` in it. The check is by
/// the name of a real recording the daemon listed, so a folder that merely exists does not count.
pub(crate) fn dir_holding(
    file: &str,
    current: &std::path::Path,
    candidates: &[PathBuf],
) -> Option<PathBuf> {
    std::iter::once(current)
        .chain(candidates.iter().map(PathBuf::as_path))
        .find(|d| d.join(file).is_file())
        .map(PathBuf::from)
}

/// A resposta de `History`.
pub fn on_history(app: &mut App, result: Result<Response, String>) {
    let Some(v) = app.recordings.as_mut() else {
        return;
    };
    v.loading = false;
    let mut found_in: Option<PathBuf> = None;
    match result {
        Ok(Response::History {
            segments,
            events,
            truncated,
        }) => {
            v.segments = segments;
            // The folder in the window's config may not be where the daemon's recordings are (Docker
            // mounts them somewhere on the host): look for one that has a file the daemon listed.
            let sample = v
                .segments
                .iter()
                .find(|s| s.ts_end.is_some())
                .map(|s| s.file.clone());
            if let Some(file) = sample
                && !app.recordings_dir.join(&file).is_file()
                && let Some(found) = dir_holding(&file, &app.recordings_dir, &candidate_dirs())
                && found != app.recordings_dir
            {
                log::info!("recordings found in {}", found.display());
                app.recordings_dir = found.clone();
                found_in = Some(found);
            }
            if let Some(p) = v.player.as_mut()
                && let Some(fresh) = v.segments.iter().find(|s| s.id == p.segment.id)
            {
                p.segment = fresh.clone();
            }
            v.events = events;
            v.truncated = truncated;
            v.error = None;
            if std::mem::take(&mut app.recordings_autoplay)
                && let Some(seg) = v
                    .segments
                    .iter()
                    .rev()
                    .find(|s| s.ts_end.is_some())
                    .cloned()
            {
                let camera = seg.camera.clone();
                let start = seg.ts_start;
                play(app, camera.clone(), seg, start);
                if app.recordings_compare {
                    let others: Vec<String> = app
                        .recordings
                        .as_ref()
                        .map(|v| v.lanes.iter().filter(|l| **l != camera).cloned().collect())
                        .unwrap_or_default();
                    for o in others {
                        toggle_compare(app, o);
                    }
                }
            }
        }
        Ok(Response::Error { message }) | Err(message) => v.error = Some(message),
        Ok(other) => v.error = Some(format!("resposta inesperada: {other:?}")),
    }
    if let Some(dir) = found_in {
        super::update::toast(
            app,
            format!("Gravações do daemon encontradas em {}", dir.display()),
        );
    }
}

fn toggle_protect(app: &mut App) {
    let Some(p) = app.recordings.as_ref().and_then(|v| v.player.as_ref()) else {
        super::update::toast(app, "Toque uma gravação para protegê-la");
        return;
    };
    let (segment_id, protected) = (p.segment.id, !p.segment.protected);
    super::update::send_to_daemon(
        app,
        Request::SetProtected {
            segment_id,
            protected,
        },
        PendingRequest::Protect {
            segment_id,
            protected,
        },
    );
}

/// A resposta de `SetProtected`: só agora a janela mostra o cadeado.
pub fn on_protected(
    app: &mut App,
    segment_id: i64,
    protected: bool,
    result: Result<Response, String>,
) {
    match result {
        Ok(Response::Ok) => {
            if let Some(v) = app.recordings.as_mut() {
                for s in v.segments.iter_mut().filter(|s| s.id == segment_id) {
                    s.protected = protected;
                }
                if let Some(p) = v.player.as_mut()
                    && p.segment.id == segment_id
                {
                    p.segment.protected = protected;
                }
            }
            super::update::toast(
                app,
                if protected {
                    "Trecho protegido: a retenção não o apaga"
                } else {
                    "Trecho solto: volta a valer a retenção"
                },
            );
        }
        Ok(Response::Error { message }) | Err(message) => {
            super::update::toast(app, format!("Não consegui mudar a proteção: {message}"))
        }
        Ok(_) => {}
    }
}

/// A resposta de `ExportClip`.
pub fn on_exported(app: &mut App, result: Result<Response, String>) {
    match result {
        Ok(Response::Exported { file, bytes }) => {
            let path = app.recordings_dir.join(&file);
            super::update::toast_open_dir(
                app,
                format!("Clipe salvo: {file} ({} KiB)", bytes / 1024),
                path.parent().map(PathBuf::from),
            );
        }
        Ok(Response::Error { message }) | Err(message) => {
            super::update::toast(app, format!("Não consegui exportar: {message}"))
        }
        Ok(_) => {}
    }
}

/// Começa a tocar o instante `t_ms` da faixa `lane` (ou o próximo trecho, se cair numa
/// lacuna).
fn click(app: &mut App, lane: usize, t_ms: i64) {
    let Some(v) = app.recordings.as_ref() else {
        return;
    };
    let Some(camera) = v.lanes.get(lane).cloned() else {
        return;
    };
    let spans = v.spans_of(&camera);
    let only: Vec<SegmentSpan> = spans.iter().map(|(_, s)| *s).collect();
    let now = now_ms();
    let (pick, at, gap) = match timeline_view::segment_at(&only, t_ms, now) {
        Some(i) => (spans[i].0, t_ms, false),
        None => match timeline_view::next_after(&only, t_ms) {
            Some(i) => (spans[i].0, only[i].start, true),
            None => {
                super::update::toast(app, "Sem gravação neste instante");
                return;
            }
        },
    };
    let segment = v.segments[pick].clone();
    // The segment already playing (also when the pointer is over the gap just before it):
    // just move inside it. This is what makes dragging the playhead cheap (a seek per
    // mouse move instead of reopening the file, and no toast per move).
    if let Some(p) = app.recordings.as_mut().and_then(|v| v.player.as_mut())
        && p.camera == camera
        && p.segment.id == segment.id
    {
        let offset = (at - segment.ts_start).max(0) as u64;
        let b = p.bridge.lock().unwrap_or_else(|e| e.into_inner());
        if b.playback_duration_ms().is_some() && b.seek_ms(offset).is_ok() {
            p.pending_seek_ms = None;
        } else {
            p.pending_seek_ms = Some(offset);
        }
        return;
    }
    if gap {
        super::update::toast(app, "Sem gravação aqui; indo para o próximo trecho");
    }
    play(app, camera, segment, at);
}

/// Abre `segment` numa nova bridge, no ponto `at_ms` (Unix ms). Não mexe na vista.
fn open_player(
    recordings_dir: &std::path::Path,
    camera: String,
    segment: SegmentInfo,
    at_ms: i64,
) -> Result<Player, String> {
    let path = recordings_dir.join(&segment.file);
    if !path.exists() {
        return Err(format!(
            "Arquivo não encontrado: {}. Ajuste [recording] dir da janela para a pasta de gravações do daemon",
            path.display()
        ));
    }
    let mut bridge =
        GStreamerBridge::new(640, 360).map_err(|e| format!("Não consegui abrir o player: {e}"))?;
    bridge
        .start_file(&path.to_string_lossy())
        .map_err(|e| format!("Não consegui abrir a gravação: {e}"))?;
    let bridge = Arc::new(Mutex::new(bridge));
    let video = VideoWidget::new(bridge.clone());
    let offset = (at_ms - segment.ts_start).max(0) as u64;
    Ok(Player {
        camera,
        segment,
        bridge,
        video,
        paused: false,
        rate: 1.0,
        pending_seek_ms: (offset > 500).then_some(offset),
        held: false,
    })
}

fn stop_player(p: Player) {
    p.bridge.lock().unwrap_or_else(|e| e.into_inner()).stop();
}

/// Toca `segment` como canal principal em `at_ms`. A câmera que era a principal passa a ser
/// um canal de comparação (a comparação não se perde ao clicar em outra faixa), e a câmera
/// clicada deixa de ser seguidora.
fn play(app: &mut App, camera: String, segment: SegmentInfo, at_ms: i64) {
    let opened = open_player(&app.recordings_dir, camera.clone(), segment, at_ms);
    let player = match opened {
        Ok(p) => p,
        Err(e) => {
            super::update::toast(app, e);
            return;
        }
    };
    let Some(v) = app.recordings.as_mut() else {
        stop_player(player);
        return;
    };
    // Parar o que tocava antes de abrir o próximo (um arquivo por vez neste canal).
    let old_master = v.player.take();
    if let Some(old) = &old_master
        && old.camera != camera
        && !v.followers.iter().any(|f| f.camera == old.camera)
        && v.followers.len() + 1 < MAX_CHANNELS
    {
        v.followers.push(Follower {
            camera: old.camera.clone(),
            player: None,
            note: None,
            retry_at: None,
        });
    }
    if let Some(old) = old_master {
        stop_player(old);
    }
    if let Some(i) = v.followers.iter().position(|f| f.camera == camera) {
        let f = v.followers.remove(i);
        if let Some(p) = f.player {
            stop_player(p);
        }
    }
    v.player = Some(player);
}

/// Liga / desliga uma câmera na comparação.
fn toggle_compare(app: &mut App, camera: String) {
    let Some(v) = app.recordings.as_mut() else {
        return;
    };
    if v.player.as_ref().is_some_and(|p| p.camera == camera) {
        return; // o canal principal já está na tela
    }
    if let Some(i) = v.followers.iter().position(|f| f.camera == camera) {
        let f = v.followers.remove(i);
        if let Some(p) = f.player {
            stop_player(p);
        }
        return;
    }
    let used = v.followers.len() + usize::from(v.player.is_some());
    if used >= MAX_CHANNELS {
        super::update::toast(
            app,
            format!("No máximo {MAX_CHANNELS} câmeras lado a lado: tire uma antes"),
        );
        return;
    }
    v.followers.push(Follower {
        camera,
        player: None,
        note: None,
        retry_at: None,
    });
}

/// Mantém cada canal de comparação no instante do principal: abre o segmento certo, corrige
/// o desvio ou mostra que não há gravação ali.
fn sync_followers(app: &mut App) {
    let now = now_ms();
    let dir = app.recordings_dir.clone();
    let Some(v) = app.recordings.as_mut() else {
        return;
    };
    let Some((_, t)) = v.playhead() else {
        return;
    };
    let (paused, rate) = v
        .player
        .as_ref()
        .map_or((false, 1.0), |p| (p.paused, p.rate));
    // O principal acabou o arquivo e parou: os outros não seguem tocando sozinhos (seriam
    // puxados de volta a cada segundo); ficam parados até o principal voltar a andar.
    let master_ended = v.player.as_ref().is_some_and(|p| {
        p.pending_seek_ms.is_none()
            && p.bridge
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .playback_ended()
    });
    let mut followers = std::mem::take(&mut v.followers);
    for f in &mut followers {
        let spans = v.spans_of(&f.camera);
        let only: Vec<SegmentSpan> = spans.iter().map(|(_, s)| *s).collect();
        let current = f.player.as_ref().and_then(|p| {
            let idx = spans
                .iter()
                .position(|(g, _)| v.segments[*g].id == p.segment.id)?;
            // Right after opening, the file has not reported a position yet: that is "still
            // loading", not "nothing is playing" (which would reopen the file every tick,
            // forever). Pretend it is exactly where it should be until it says otherwise.
            let pos = p
                .bridge
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .playback_position_ms()
                .map_or_else(|| (t - p.segment.ts_start).max(0), |ms| ms as i64);
            Some((idx, pos))
        });
        // Um seek pendente (arquivo ainda sem duração) é concluído aqui.
        if let Some(p) = f.player.as_mut()
            && let Some(ms) = p.pending_seek_ms
        {
            let b = p.bridge.lock().unwrap_or_else(|e| e.into_inner());
            if b.playback_duration_ms().is_some() && b.seek_ms(ms).is_ok() {
                p.pending_seek_ms = None;
            }
        }
        if let Some(p) = f.player.as_mut() {
            if master_ended && !p.held {
                p.held = true;
                let _ = p
                    .bridge
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .set_playback_paused(true);
            } else if !master_ended && p.held {
                p.held = false;
                if !paused {
                    let _ = p
                        .bridge
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .set_playback_paused(false);
                }
            }
        }
        // Com um seek ainda pendente a posição não vale: não corrige o desvio por cima dele.
        let settling = f
            .player
            .as_ref()
            .is_some_and(|p| p.pending_seek_ms.is_some());
        let action = timeline_view::follow_action(&only, current, t, now);
        if !matches!(action, timeline_view::FollowAction::Keep) {
            log::debug!(
                "follower {}: t={t} current={current:?} → {action:?}",
                f.camera
            );
        }
        match action {
            timeline_view::FollowAction::Keep => {}
            timeline_view::FollowAction::Seek { .. } if settling => {}
            timeline_view::FollowAction::Seek { offset_ms } => {
                if let Some(p) = &f.player {
                    let _ = p
                        .bridge
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .seek_ms(offset_ms);
                }
            }
            timeline_view::FollowAction::Gap => {
                if let Some(p) = f.player.take() {
                    stop_player(p);
                }
                f.note = Some("Sem gravação neste instante".into());
                f.retry_at = None;
            }
            timeline_view::FollowAction::Open { index, offset_ms } => {
                if f.retry_at.is_some_and(|at| std::time::Instant::now() < at) {
                    continue;
                }
                if let Some(p) = f.player.take() {
                    stop_player(p);
                }
                let segment = v.segments[spans[index].0].clone();
                let at = segment.ts_start + offset_ms as i64;
                match open_player(&dir, f.camera.clone(), segment, at) {
                    Ok(mut p) => {
                        log::debug!("follower {} opened {}", f.camera, p.segment.file);
                        // Entra com o mesmo estado do principal (pausa, velocidade).
                        p.paused = paused;
                        p.rate = rate;
                        {
                            let b = p.bridge.lock().unwrap_or_else(|e| e.into_inner());
                            if paused {
                                let _ = b.set_playback_paused(true);
                            }
                            if rate != 1.0 {
                                let _ = b.set_playback_rate(rate);
                            }
                        }
                        f.player = Some(p);
                        f.note = None;
                        f.retry_at = None;
                    }
                    Err(e) => {
                        log::debug!("follower {} did not open: {e}", f.camera);
                        f.note = Some(e);
                        f.retry_at =
                            Some(std::time::Instant::now() + std::time::Duration::from_secs(3));
                    }
                }
            }
        }
    }
    v.followers = followers;
}

fn skip(app: &mut App, ms: i64) {
    let Some(p) = app.recordings.as_mut().and_then(|v| v.player.as_mut()) else {
        return;
    };
    let b = p.bridge.lock().unwrap_or_else(|e| e.into_inner());
    let now = b.playback_position_ms().unwrap_or(0) as i64;
    let dur = b.playback_duration_ms().map_or(i64::MAX, |d| d as i64);
    let _ = b.seek_ms((now + ms).clamp(0, dur) as u64);
}

fn mark(app: &mut App, is_in: bool) {
    let Some(v) = app.recordings.as_mut() else {
        return;
    };
    let Some((_, t)) = v.playhead() else {
        super::update::toast(
            app,
            "Toque uma gravação para marcar o início e o fim do clipe",
        );
        return;
    };
    if is_in {
        v.mark_in = Some(t);
        if v.mark_out.is_some_and(|o| o <= t) {
            v.mark_out = None;
        }
    } else {
        v.mark_out = Some(t);
    }
}

fn export(app: &mut App) {
    let Some(v) = app.recordings.as_ref() else {
        return;
    };
    let (Some(from), Some(to)) = (v.mark_in, v.mark_out) else {
        super::update::toast(
            app,
            "Marque o início (I) e o fim (O) do clipe antes de exportar",
        );
        return;
    };
    let Some(camera) = v.player.as_ref().map(|p| p.camera.clone()) else {
        return;
    };
    if to <= from {
        super::update::toast(app, "O fim do clipe precisa ser depois do início");
        return;
    }
    if super::update::send_to_daemon(
        app,
        Request::ExportClip {
            camera,
            from_ms: from,
            to_ms: to,
        },
        PendingRequest::Export,
    ) {
        super::update::toast(app, "Exportando o clipe…");
    }
}

/// A cada [`REFRESH_EVERY`] pergunta ao daemon pelo histórico de novo; se a janela segue o
/// vivo, ela anda junto com o relógio. Sem daemon, não faz nada (e não enche a tela de avisos).
fn refresh_if_due(app: &mut App) {
    let connected = app.daemon.is_connected();
    let now = now_ms();
    let Some(v) = app.recordings.as_mut() else {
        return;
    };
    if !connected || v.refreshed_at.elapsed() < REFRESH_EVERY {
        return;
    }
    if v.follow {
        v.span = v.span.following(now);
    }
    request_history(app);
}

/// A cada tick: conclui um seek pendente e passa ao trecho seguinte quando um acaba.
pub fn tick(app: &mut App) {
    refresh_if_due(app);
    let mut next: Option<(String, SegmentInfo)> = None;
    if let Some(v) = app.recordings.as_mut()
        && let Some(p) = v.player.as_mut()
    {
        let b = p.bridge.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ms) = p.pending_seek_ms
            && b.playback_duration_ms().is_some()
            && b.seek_ms(ms).is_ok()
        {
            p.pending_seek_ms = None;
        }
        let ended = p.pending_seek_ms.is_none() && b.playback_ended();
        drop(b);
        if ended {
            let camera = p.camera.clone();
            let end = p.segment.ts_end.unwrap_or(i64::MAX);
            let spans = v.spans_of(&camera);
            let only: Vec<SegmentSpan> = spans.iter().map(|(_, s)| *s).collect();
            if let Some(i) = timeline_view::next_after(&only, end - 1) {
                next = Some((camera, v.segments[spans[i].0].clone()));
            }
        }
    }
    if let Some((camera, seg)) = next {
        let start = seg.ts_start;
        play(app, camera, seg, start);
    }
    sync_followers(app);
}

// ───────────────────────────── view ─────────────────────────────

struct TimelineProgram<'a> {
    view: &'a RecordingsView,
    colors: ThemeColors,
    now: i64,
    utc_offset_secs: i64,
}

impl canvas::Program<Message> for TimelineProgram<'_> {
    /// `true` while the left button is held on the timeline (dragging the playhead).
    type State = bool;

    fn draw(
        &self,
        _state: &bool,
        renderer: &Renderer,
        _theme: &iced::Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let c = self.colors;
        let col = Theme::color_from_hex;
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let v = self.view;
        let plot_w = (bounds.width - GUTTER).max(1.0);
        let x_of = |f: f32| GUTTER + f * plot_w;

        for (i, name) in v.lanes.iter().enumerate() {
            let y = i as f32 * LANE_H;
            frame.fill_rectangle(
                Point::new(GUTTER, y + 2.0),
                Size::new(plot_w, LANE_H - 4.0),
                col(c.surface),
            );
            frame.fill_text(canvas::Text {
                content: name.clone(),
                position: Point::new(6.0, y + LANE_H / 2.0 - 7.0),
                color: col(c.text),
                size: 12.0.into(),
                ..canvas::Text::default()
            });
            let spans: Vec<SegmentSpan> = v.spans_of(name).iter().map(|(_, s)| *s).collect();
            for b in timeline_view::bars(&spans, v.span, self.now) {
                let color = if b.live {
                    col(c.accent_red)
                } else if b.motion {
                    col(c.accent_amber)
                } else {
                    col(c.accent_blue)
                };
                frame.fill_rectangle(
                    Point::new(x_of(b.start), y + 6.0),
                    Size::new((b.end - b.start) * plot_w, LANE_H - 12.0),
                    color,
                );
                if b.protected {
                    // cadeado: um traço claro no topo (nunca só cor)
                    frame.fill_rectangle(
                        Point::new(x_of(b.start), y + 3.0),
                        Size::new(((b.end - b.start) * plot_w).max(2.0), 2.0),
                        col(c.text),
                    );
                }
            }
            for e in v.events.iter().filter(|e| e.camera == *name) {
                let f = v.span.frac(e.ts);
                if (0.0..=1.0).contains(&f) {
                    frame.fill_rectangle(
                        Point::new(x_of(f) - 1.0, y + 2.0),
                        Size::new(2.0, 5.0),
                        col(c.text),
                    );
                }
            }
        }

        // clipe marcado
        if let (Some(a), Some(b)) = (v.mark_in, v.mark_out) {
            let (fa, fb) = (
                v.span.frac(a).clamp(0.0, 1.0),
                v.span.frac(b).clamp(0.0, 1.0),
            );
            let mut shade = col(c.accent_green);
            shade.a = 0.25;
            frame.fill_rectangle(
                Point::new(x_of(fa), 0.0),
                Size::new((fb - fa).max(0.0) * plot_w, v.lanes.len() as f32 * LANE_H),
                shade,
            );
        }

        // eixo
        let axis_y = v.lanes.len() as f32 * LANE_H;
        for (f, label) in timeline_view::ticks(v.span, self.utc_offset_secs, 8) {
            frame.fill_rectangle(
                Point::new(x_of(f), 0.0),
                Size::new(1.0, axis_y + 4.0),
                col(c.border),
            );
            frame.fill_text(canvas::Text {
                content: label,
                position: Point::new(x_of(f) - 14.0, axis_y + 5.0),
                color: col(c.text_secondary),
                size: 11.0.into(),
                ..canvas::Text::default()
            });
        }

        // cabeça de reprodução
        if let Some((_, t)) = v.playhead() {
            let f = v.span.frac(t);
            if (0.0..=1.0).contains(&f) {
                frame.fill_rectangle(
                    Point::new(x_of(f) - 1.0, 0.0),
                    Size::new(2.0, axis_y),
                    col(c.text),
                );
            }
        }
        vec![frame.into_geometry()]
    }

    fn update(
        &self,
        dragging: &mut bool,
        event: canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> (canvas::event::Status, Option<Message>) {
        use canvas::event::Status::{Captured, Ignored};
        let plot_w = (bounds.width - GUTTER).max(1.0);
        let lanes = self.view.lanes.len();
        // Where the pointer is on the plot, even a little outside it while dragging.
        let at = |p: iced::Point| {
            let frac = ((p.x - GUTTER) / plot_w).clamp(0.0, 1.0);
            let lane = ((p.y / LANE_H).max(0.0) as usize).min(lanes.saturating_sub(1));
            (lane, self.view.span.time_at(frac), frac)
        };
        let clicked = |lane, t_ms| Some(Message::Recordings(RecMsg::Clicked { lane, t_ms }));
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let Some(pos) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                if pos.x < GUTTER || (pos.y / LANE_H) as usize >= lanes {
                    return (Ignored, None);
                }
                *dragging = true;
                let (lane, t_ms, _) = at(pos);
                (Captured, clicked(lane, t_ms))
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { .. }) if *dragging => {
                match cursor.position_from(bounds.position()) {
                    Some(pos) => {
                        let (lane, t_ms, _) = at(pos);
                        (Captured, clicked(lane, t_ms))
                    }
                    None => (Ignored, None),
                }
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if *dragging =>
            {
                *dragging = false;
                (Captured, None)
            }
            canvas::Event::Mouse(mouse::Event::WheelScrolled { delta }) => {
                let Some(pos) = cursor.position_in(bounds) else {
                    return (Ignored, None);
                };
                let y = match delta {
                    mouse::ScrollDelta::Lines { y, .. } | mouse::ScrollDelta::Pixels { y, .. } => y,
                };
                if y == 0.0 {
                    return (Ignored, None);
                }
                let (_, _, frac) = at(pos);
                let factor = if y > 0.0 { 0.8 } else { 1.25 };
                (
                    Captured,
                    Some(Message::Recordings(RecMsg::Zoom {
                        factor,
                        anchor: frac,
                    })),
                )
            }
            _ => (Ignored, None),
        }
    }
}

fn pill_button<'a>(
    app: &App,
    label: impl iced::widget::text::IntoFragment<'a>,
    msg: RecMsg,
    intent: Intent,
) -> iced::widget::Button<'a, Message> {
    button(text(label).size(12))
        .padding(iced::Padding::from([4, 10]))
        .style(style::pill(app.theme, intent))
        .on_press(Message::Recordings(msg))
}

/// The player's picture with the stored detections near the playhead drawn over it.
fn player_with_boxes<'a>(p: &'a Player, v: &'a RecordingsView) -> Element<'a, Message> {
    let picture: Element<'a, Message> = p.video.view().map(|_| Message::FrameUpdate);
    let Some((_, ts)) = v.playhead() else {
        return picture;
    };
    let boxes = boxes_at(&v.events, &p.camera, ts);
    if boxes.is_empty() {
        return picture;
    }
    let (_, w, h, _) = p
        .bridge
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .read_frame();
    iced::widget::stack![
        picture,
        crate::ui::detections_overlay::layer(boxes, w as f32, h as f32)
    ]
    .into()
}

pub fn view(app: &App) -> Element<'_, Message> {
    let Some(v) = app.recordings.as_ref() else {
        return column![].into();
    };
    let colors = app.theme.colors();
    let text_color = Theme::color_from_hex(colors.text);
    let dim = Theme::color_from_hex(colors.text_secondary);

    let span_hours = (v.span.len() / 3_600_000).max(1);
    let header = row![
        text("Gravações").size(15).color(text_color),
        iced::widget::horizontal_space(),
        pill_button(app, "1 h", RecMsg::SetSpan(1), sel(span_hours == 1)),
        pill_button(app, "6 h", RecMsg::SetSpan(6), sel(span_hours == 6)),
        pill_button(app, "24 h", RecMsg::SetSpan(24), sel(span_hours == 24)),
        pill_button(app, "7 d", RecMsg::SetSpan(168), sel(span_hours == 168)),
        pill_button(app, "‹", RecMsg::Pan(-0.5), Intent::Ghost),
        pill_button(app, "›", RecMsg::Pan(0.5), Intent::Ghost),
        pill_button(app, "Fechar  Esc", RecMsg::Close, Intent::Ghost),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center);

    // ── a imagem
    let stage: Element<'_, Message> = match &v.player {
        Some(_) if !v.followers.is_empty() => channels_stage(app, v),
        Some(p) => player_with_boxes(p, v),
        None => container(
            text(if v.loading {
                "Carregando…".to_string()
            } else if let Some(e) = &v.error {
                format!("Não consegui carregar o histórico: {e}")
            } else if v.segments.is_empty() {
                "Nada gravado neste período. Ative a gravação por movimento ou a manual (r)."
                    .to_string()
            } else {
                "Clique na linha do tempo para ver a gravação".to_string()
            })
            .size(13)
            .color(dim),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into(),
    };
    let stage = container(stage)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::BLACK)),
            ..container::Style::default()
        });
    let stage = row![stage, events_column(app, v)]
        .spacing(8)
        .height(Length::Fill);

    // ── os controles
    let controls: Element<'_, Message> = match &v.player {
        Some(p) => {
            let (pos, dur) = {
                let b = p.bridge.lock().unwrap_or_else(|e| e.into_inner());
                (
                    b.playback_position_ms().unwrap_or(0),
                    b.playback_duration_ms().unwrap_or(0),
                )
            };
            let when = format_clock(p.segment.ts_start + pos as i64);
            row![
                pill_button(
                    app,
                    if p.paused { "Tocar" } else { "Pausar" },
                    RecMsg::PlayPause,
                    Intent::Primary
                ),
                pill_button(app, "−10 s", RecMsg::Skip(-SKIP_MS), Intent::Ghost),
                pill_button(app, "+10 s", RecMsg::Skip(SKIP_MS), Intent::Ghost),
                pill_button(app, "Quadro", RecMsg::Step, Intent::Ghost),
                pill_button(app, "0,5×", RecMsg::Rate(0.5), sel(p.rate == 0.5)),
                pill_button(app, "1×", RecMsg::Rate(1.0), sel(p.rate == 1.0)),
                pill_button(app, "2×", RecMsg::Rate(2.0), sel(p.rate == 2.0)),
                pill_button(app, "4×", RecMsg::Rate(4.0), sel(p.rate == 4.0)),
                text(format!(
                    "{}  ·  {}  ·  {} / {}",
                    p.camera,
                    when,
                    format_mmss(pos),
                    format_mmss(dur)
                ))
                .size(12)
                .color(dim),
                iced::widget::horizontal_space(),
                pill_button(app, "Início  I", RecMsg::MarkIn, sel(v.mark_in.is_some())),
                pill_button(app, "Fim  O", RecMsg::MarkOut, sel(v.mark_out.is_some())),
                pill_button(
                    app,
                    if p.segment.protected { "Soltar  P" } else { "Proteger  P" },
                    RecMsg::ToggleProtect,
                    sel(p.segment.protected)
                ),
                pill_button(app, "Exportar  E", RecMsg::Export, Intent::Primary),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .into()
        }
        None => text(
            "Clique para ver · roda do mouse aproxima · ‹ › desloca · Espaço toca/pausa · setas esquerda/direita pulam 10 s",
        )
        .size(11)
        .color(dim)
        .into(),
    };

    let lanes = v.lanes.len().max(1) as f32;
    let timeline = canvas(TimelineProgram {
        view: v,
        colors,
        now: now_ms(),
        utc_offset_secs: chrono::Local::now().offset().local_minus_utc() as i64,
    })
    .width(Length::Fill)
    .height(Length::Fixed(lanes * LANE_H + AXIS_H + 6.0));

    let bg = Theme::color_from_hex(colors.background);
    container(
        column![header, stage, compare_row(app, v), controls, timeline]
            .spacing(8)
            .padding(12),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(move |_: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(bg)),
        ..container::Style::default()
    })
    .into()
}

/// Os canais lado a lado: o principal e os que acompanham, em grade (2 por linha).
fn channels_stage<'a>(app: &'a App, v: &'a RecordingsView) -> Element<'a, Message> {
    let colors = app.theme.colors();
    let dim = Theme::color_from_hex(colors.text_secondary);
    let text_color = Theme::color_from_hex(colors.text);
    let tile = |name: &str, body: Element<'a, Message>, is_main: bool| -> Element<'a, Message> {
        let label = if is_main {
            format!("{name}  (principal)")
        } else {
            name.to_string()
        };
        container(
            column![text(label).size(11).color(text_color), body]
                .spacing(2)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(2)
        .style(|_: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(iced::Color::BLACK)),
            ..container::Style::default()
        })
        .into()
    };
    let mut tiles: Vec<Element<'a, Message>> = Vec::new();
    if let Some(p) = &v.player {
        tiles.push(tile(
            &p.camera,
            p.video.view().map(|_| Message::FrameUpdate),
            true,
        ));
    }
    for f in &v.followers {
        let body: Element<'a, Message> = match &f.player {
            Some(p) => p.video.view().map(|_| Message::FrameUpdate),
            None => container(
                text(f.note.clone().unwrap_or_else(|| "Carregando…".to_string()))
                    .size(12)
                    .color(dim),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into(),
        };
        tiles.push(tile(&f.camera, body, false));
    }
    let mut grid = column![].spacing(4).height(Length::Fill);
    let mut it = tiles.into_iter();
    while let Some(first) = it.next() {
        let second = it
            .next()
            .unwrap_or_else(|| iced::widget::Space::new(Length::Fill, Length::Fill).into());
        grid = grid.push(row![first, second].spacing(4).height(Length::Fill));
    }
    grid.into()
}

/// "Comparar: [Câmera] [Câmera]": liga e desliga canais lado a lado (só com mais de uma câmera).
fn compare_row<'a>(app: &'a App, v: &'a RecordingsView) -> Element<'a, Message> {
    let master = v.player.as_ref().map(|p| p.camera.as_str());
    let others: Vec<&String> = v
        .lanes
        .iter()
        .filter(|l| Some(l.as_str()) != master)
        .collect();
    if v.player.is_none() || others.is_empty() {
        return iced::widget::Space::new(Length::Shrink, Length::Shrink).into();
    }
    let dim = Theme::color_from_hex(app.theme.colors().text_secondary);
    let mut r = row![text("Comparar:").size(12).color(dim)]
        .spacing(6)
        .align_y(iced::Alignment::Center);
    for name in others {
        let on = v.followers.iter().any(|f| f.camera == *name);
        r = r.push(
            button(text(name.as_str()).size(12))
                .padding(iced::Padding::from([4, 10]))
                .style(style::pill(app.theme, sel(on)))
                .on_press(Message::Recordings(RecMsg::ToggleCompare(name.clone()))),
        );
    }
    r.into()
}

/// O nome de um evento do histórico, em português.
pub fn event_label(kind: &str) -> &'static str {
    match kind {
        "motion" => "Movimento",
        "recording_start" => "Gravação iniciada",
        "recording_stop" => "Gravação parou",
        "offline" => "Câmera offline",
        "online" => "Câmera online",
        "snapshot" => "Foto",
        "disk_low" => "Disco quase cheio",
        "detection" => "Objeto",
        _ => "Evento",
    }
}

/// Os eventos que a lista mostra: do mais novo ao mais antigo, com os filtros aplicados
/// (`label`: só as detecções desse objeto).
pub fn visible_events<'a>(
    events: &'a [HistoryEvent],
    motion_only: bool,
    label: Option<&str>,
    limit: usize,
) -> Vec<&'a HistoryEvent> {
    events
        .iter()
        .rev()
        .filter(|e| !motion_only || e.kind == "motion")
        .filter(|e| label.is_none_or(|l| e.kind == "detection" && e.label == l))
        .take(limit)
        .collect()
}

/// How far from the playhead a stored detection is still drawn over the picture.
const BOX_WINDOW_MS: i64 = 2500;

/// As caixas das detecções gravadas de `camera` perto do instante `ts_ms` (Unix ms): a mais próxima de
/// cada objeto, dentro de [`BOX_WINDOW_MS`]. O histórico guarda uma detecção por objeto a cada
/// `[detect] cooldown_secs`, então só aparecem caixas perto desses instantes.
pub fn boxes_at(events: &[HistoryEvent], camera: &str, ts_ms: i64) -> Vec<WireBox> {
    let mut best: Vec<(i64, &HistoryEvent)> = Vec::new();
    for e in events {
        let near = (e.ts - ts_ms).abs();
        if e.kind != "detection" || e.camera != camera || e.bbox.is_none() || near > BOX_WINDOW_MS {
            continue;
        }
        match best.iter_mut().find(|(_, b)| b.label == e.label) {
            Some(slot) if slot.0 <= near => {}
            Some(slot) => *slot = (near, e),
            None => best.push((near, e)),
        }
    }
    best.into_iter()
        .filter_map(|(_, e)| {
            let [x, y, w, h] = e.bbox?;
            Some(WireBox {
                label: e.label.clone(),
                score: e.score.unwrap_or(0.0) as f32,
                x,
                y,
                w,
                h,
            })
        })
        .collect()
}

/// Os objetos que aparecem nas detecções do período, em ordem alfabética.
pub fn detection_labels(events: &[HistoryEvent]) -> Vec<String> {
    let mut v: Vec<String> = events
        .iter()
        .filter(|e| e.kind == "detection" && !e.label.is_empty())
        .map(|e| e.label.clone())
        .collect();
    v.sort();
    v.dedup();
    v
}

/// O próximo filtro do botão de objeto: nenhum → o primeiro → … → o último → nenhum. Um filtro que
/// deixou de existir (o período mudou) volta ao início.
pub fn next_label(current: Option<&str>, labels: &[String]) -> Option<String> {
    match current.and_then(|c| labels.iter().position(|l| l == c)) {
        None if current.is_none() => labels.first().cloned(),
        None => None,
        Some(i) => labels.get(i + 1).cloned(),
    }
}

/// A linha de um evento na lista: detecções mostram o objeto e a confiança ("Garagem · pessoa 86%").
pub fn event_text(e: &HistoryEvent) -> String {
    if e.kind == "detection" && !e.label.is_empty() {
        let what = crate::domain::detect::label_pt(&e.label);
        return match e.score {
            Some(s) => format!("{} · {what} {:.0}%", e.camera, s * 100.0),
            None => format!("{} · {what}", e.camera),
        };
    }
    format!("{} · {}", e.camera, event_label(&e.kind))
}

fn events_column<'a>(app: &'a App, v: &'a RecordingsView) -> Element<'a, Message> {
    let colors = app.theme.colors();
    let dim = Theme::color_from_hex(colors.text_secondary);
    let text_color = Theme::color_from_hex(colors.text);
    let shown = visible_events(&v.events, v.motion_only, v.label_filter.as_deref(), 200);
    let mut list = column![].spacing(2).width(Length::Fill);
    if shown.is_empty() {
        list = list.push(text("Nenhum evento neste período").size(12).color(dim));
    }
    for e in shown {
        let playable = v.lanes.contains(&e.camera);
        let line = row![
            text(format_clock(e.ts)).size(11).color(dim),
            text(event_text(e)).size(12).color(text_color),
        ]
        .spacing(8);
        let b = button(line)
            .width(Length::Fill)
            .padding(iced::Padding::from([3, 6]))
            .style(style::pill(app.theme, Intent::Ghost));
        list = list.push(if playable {
            b.on_press(Message::Recordings(RecMsg::EventClicked {
                camera: e.camera.clone(),
                ts_ms: e.ts,
            }))
        } else {
            b
        });
    }
    let mut header = row![
        text("Eventos").size(13).color(text_color),
        iced::widget::horizontal_space(),
    ]
    .spacing(4)
    .align_y(iced::Alignment::Center);
    // o botão de objeto só existe quando o período tem alguma detecção
    if !detection_labels(&v.events).is_empty() {
        let name = v.label_filter.as_deref().map_or_else(
            || "todos".to_string(),
            |l| crate::domain::detect::label_pt(l).to_string(),
        );
        header = header.push(pill_button(
            app,
            format!("Objeto: {name}"),
            RecMsg::CycleLabel,
            sel(v.label_filter.is_some()),
        ));
    }
    header = header.push(pill_button(
        app,
        "Só movimento",
        RecMsg::ToggleMotionOnly,
        sel(v.motion_only),
    ));
    column![header, iced::widget::scrollable(list).height(Length::Fill),]
        .spacing(6)
        .width(EVENTS_W)
        .into()
}

fn sel(on: bool) -> Intent {
    if on { Intent::Selected } else { Intent::Ghost }
}

fn format_mmss(ms: u64) -> String {
    let s = ms / 1000;
    format!("{:02}:{:02}", s / 60, s % 60)
}

/// `dd/mm HH:MM:SS` local.
fn format_clock(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%d/%m %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "?".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: i64, kind: &str) -> HistoryEvent {
        HistoryEvent {
            id,
            camera: "Garagem".into(),
            ts: id,
            kind: kind.into(),
            label: String::new(),
            segment_id: None,
            score: None,
            bbox: None,
            zone: None,
        }
    }

    #[test]
    fn the_event_list_is_newest_first_and_can_filter_motion() {
        let events = [
            ev(1, "motion"),
            ev(2, "recording_start"),
            ev(3, "motion"),
            ev(4, "offline"),
        ];
        let all: Vec<i64> = visible_events(&events, false, None, 10)
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(all, [4, 3, 2, 1]);
        let motion: Vec<i64> = visible_events(&events, true, None, 10)
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(motion, [3, 1]);
        assert_eq!(
            visible_events(&events, false, None, 2).len(),
            2,
            "respeita o limite"
        );
    }

    #[test]
    fn every_known_event_kind_has_a_portuguese_label() {
        for k in [
            "motion",
            "recording_start",
            "recording_stop",
            "offline",
            "online",
            "snapshot",
            "disk_low",
            "detection",
        ] {
            assert_ne!(event_label(k), "Evento", "{k}");
        }
        assert_eq!(event_label("algo_novo"), "Evento");
    }

    #[test]
    fn the_recordings_folder_is_found_by_a_file_the_daemon_listed() {
        let base = std::env::temp_dir().join(format!("rrv-find-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (wrong, host, other) = (base.join("wrong"), base.join("host"), base.join("other"));
        for d in [&wrong, &host, &other] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(host.join("a.mkv"), b"x").unwrap();
        let cands = [other.clone(), host.clone()];
        // a pasta configurada não tem o arquivo: acha a candidata que tem
        assert_eq!(dir_holding("a.mkv", &wrong, &cands), Some(host.clone()));
        // se a configurada já tem, fica a configurada
        assert_eq!(dir_holding("a.mkv", &host, &cands), Some(host.clone()));
        // nenhuma tem: não inventa
        assert_eq!(dir_holding("b.mkv", &wrong, &cands), None);
        // uma pasta que só existe (sem o arquivo) não conta
        assert_eq!(dir_holding("a.mkv", &wrong, &[other]), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_refusal_explains_the_mode_and_what_to_do() {
        use crate::ui::daemon::Mode;
        let mut d = DaemonState::embedded("/run/user/1000/rrv/rrv.sock".into());
        let embedded = unavailable_reason(&d).unwrap();
        assert!(
            embedded.contains("motor local") && embedded.contains("RRV_SOCKET"),
            "{embedded}"
        );
        assert!(
            embedded.contains("/run/user/1000/rrv/rrv.sock"),
            "diz onde procurou: {embedded}"
        );
        for (mode, word) in [
            (Mode::Connecting, "Conectando"),
            (Mode::Lost, "Sem contato"),
            (Mode::Incompatible, "versões diferentes"),
            (Mode::NoPermission, "permissão"),
        ] {
            d.mode = mode;
            let why = unavailable_reason(&d).unwrap_or_default();
            assert!(why.contains(word), "{mode:?}: {why}");
        }
        d.mode = Mode::Connected;
        assert_eq!(unavailable_reason(&d), None, "conectado: abre");
    }

    #[test]
    fn mmss_formats() {
        assert_eq!(format_mmss(0), "00:00");
        assert_eq!(format_mmss(65_400), "01:05");
    }

    fn det(id: i64, label: &str, score: f64) -> HistoryEvent {
        HistoryEvent {
            label: label.into(),
            score: Some(score),
            ..ev(id, "detection")
        }
    }

    #[test]
    fn the_object_filter_cycles_through_the_labels_present_and_back_to_none() {
        let events = [
            det(1, "person", 0.9),
            ev(2, "motion"),
            det(3, "car", 0.7),
            det(4, "person", 0.8),
        ];
        let labels = detection_labels(&events);
        assert_eq!(labels, ["car", "person"]);
        let mut cur: Option<String> = None;
        let mut seen = Vec::new();
        for _ in 0..4 {
            cur = next_label(cur.as_deref(), &labels);
            seen.push(cur.clone());
        }
        assert_eq!(
            seen,
            [
                Some("car".into()),
                Some("person".into()),
                None,
                Some("car".into())
            ]
        );
        // um filtro que sumiu do período (carro) volta ao início em vez de travar
        assert_eq!(next_label(Some("dog"), &labels), None);
        assert!(detection_labels(&[ev(1, "motion")]).is_empty());
        assert_eq!(next_label(None, &[]), None);
    }

    #[test]
    fn filtering_by_object_keeps_only_those_detections_newest_first() {
        let events = [
            det(1, "person", 0.9),
            ev(2, "motion"),
            det(3, "car", 0.7),
            det(4, "person", 0.8),
        ];
        let ids: Vec<i64> = visible_events(&events, false, Some("person"), 10)
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, [4, 1]);
        assert!(visible_events(&events, true, Some("person"), 10).is_empty());
    }

    #[test]
    fn a_detection_line_names_the_object_and_the_confidence() {
        assert_eq!(event_text(&det(1, "person", 0.867)), "Garagem · pessoa 87%");
        assert_eq!(event_text(&ev(2, "motion")), "Garagem · Movimento");
    }

    #[test]
    fn stored_boxes_near_the_playhead_are_drawn_the_nearest_per_object() {
        let boxed = |id: i64, ts: i64, label: &str, x: f32| HistoryEvent {
            ts,
            bbox: Some([x, 0.2, 0.1, 0.3]),
            ..det(id, label, 0.9)
        };
        let events = [
            boxed(1, 10_000, "person", 0.1),
            boxed(2, 11_000, "person", 0.5), // mais perto de 11 s
            boxed(3, 11_500, "car", 0.7),
            boxed(4, 30_000, "person", 0.9), // longe demais
            HistoryEvent {
                camera: "Outra".into(),
                ..boxed(5, 11_000, "person", 0.3)
            },
            det(6, "person", 0.9), // sem caixa (evento antigo)
        ];
        let got = boxes_at(&events, "Garagem", 11_100);
        let mut labels: Vec<_> = got.iter().map(|b| (b.label.as_str(), b.x)).collect();
        labels.sort_by(|a, b| a.0.cmp(b.0));
        assert_eq!(labels, [("car", 0.7), ("person", 0.5)]);
        assert!(boxes_at(&events, "Garagem", 20_000).is_empty());
    }
}
