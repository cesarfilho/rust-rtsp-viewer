//! O estado da conexão da janela com o daemon e o que a interface mostra dele
//! (spec `docs/specs/ux-daemon.md`).
//!
//! Puro: recebe os [`LinkEvent`]s da thread de conexão e decide modo, textos e
//! avisos, sem desenhar nada. A janela só traduz isto em widgets.

use crate::i18n::{plural, t, tf};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::ipc::link::LinkEvent;
use crate::ipc::protocol::{CameraInfo, Response, WireEvent};

/// Em que modo a janela está (tabela de modos da spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Não há daemon: a própria janela grava e detecta (uso de antes do ADR 0010).
    Embedded,
    /// Há socket; o `Hello` está em andamento.
    Connecting,
    /// O daemon grava e detecta; a janela mostra e comanda.
    Connected,
    /// Estava conectado e perdeu o canal. O daemon pode ainda estar gravando.
    Lost,
    /// O daemon fala outra versão do protocolo.
    Incompatible,
    /// O socket existe mas é de outro usuário.
    NoPermission,
}

/// Cor semântica do chip e do banner (o tema decide o tom exato).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Good,
    Warning,
    Error,
}

/// Aviso fino sob a barra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Banner {
    pub tone: Tone,
    pub text: String,
}

/// O que o chamador precisa fazer depois de [`DaemonState::apply`].
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    None,
    /// Aviso breve ao usuário.
    Toast(String),
    /// Um evento do daemon para a timeline.
    Event(WireEvent),
    /// A resposta a um pedido da janela.
    Reply {
        token: u64,
        result: Result<Response, String>,
    },
}

/// Como a janela abre: com `--embedded` ou sem socket, usa o motor local (quem
/// nunca instalou o daemon não vê diferença); com socket, conecta. Nunca sobe um
/// daemon sozinha.
pub fn initial_mode(force_embedded: bool, socket_exists: bool) -> Mode {
    if force_embedded || !socket_exists {
        Mode::Embedded
    } else {
        Mode::Connecting
    }
}

/// Uma confirmação que a pessoa precisa dar antes de uma ação que não se desfaz
/// sem custo (spec `ux-daemon.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    /// Trocar para o motor local: pode gravar em dobro se o daemon ainda grava.
    UseLocalEngine,
    /// Sair com gravações locais em curso: sair as interrompe.
    Quit { recordings: usize },
}

impl Modal {
    /// A confirmação ao sair, se for o caso: só quando fechar a janela cortaria
    /// gravações que **ela mesma** está fazendo (motor local). Com um daemon, ou
    /// sem nada gravando, fecha direto.
    pub fn for_quit(local_recordings: usize) -> Option<Modal> {
        (local_recordings > 0).then_some(Modal::Quit {
            recordings: local_recordings,
        })
    }

    pub fn title(&self) -> &'static str {
        match self {
            Modal::UseLocalEngine => t("Usar o motor local?"),
            Modal::Quit { .. } => t("Sair e interromper as gravações?"),
        }
    }

    pub fn body(&self) -> String {
        match self {
            Modal::UseLocalEngine => t("Isto começa a gravar daqui. Se o daemon ainda estiver gravando, haverá gravações em dobro. Continuar?").into(),
            Modal::Quit { recordings: 1 } => t("Há 1 gravação em curso. Sair a interrompe. Para gravar com a janela fechada, use o daemon.").into(),
            Modal::Quit { recordings } => tf("Há {} gravações em curso. Sair as interrompe. Para gravar com a janela fechada, use o daemon.", &[recordings]),
        }
    }

    pub fn confirm_label(&self) -> &'static str {
        match self {
            Modal::UseLocalEngine => t("Usar motor local"),
            Modal::Quit { .. } => t("Sair mesmo assim"),
        }
    }
}

/// Um pedido à espera de resposta do daemon, para a janela saber o que fazer com
/// ela (e mostrar "aguardando…" até lá).
#[derive(Debug, Clone)]
pub enum PendingRequest {
    /// Gravar/parar a câmera de nome `camera`.
    ToggleRecording { camera: String },
    /// Ligar/desligar (a janela já aplicou localmente).
    SetEnabled { camera: String },
    /// Salvar as zonas da câmera de índice local `camera`. `zones` é a lista que
    /// passa a valer **se** o daemon confirmar; até lá nada muda na janela e o
    /// editor segue aberto com o desenho intacto.
    SetZones {
        camera: usize,
        zones: Vec<crate::domain::zones::MotionZone>,
    },
    /// O histórico da vista de gravações.
    History,
    /// Exportar um clipe.
    Export,
    /// Proteger / soltar um segmento (a janela só aplica quando o daemon confirma).
    Protect { segment_id: i64, protected: bool },
}

#[derive(Debug, Clone)]
pub struct DaemonState {
    pub mode: Mode,
    pub socket: PathBuf,
    /// `rrv-daemon 0.8.0` (vazio até conectar).
    pub server: String,
    /// O estado das câmeras segundo o daemon.
    pub cameras: Vec<CameraInfo>,
    /// O motivo da última falha, para o menu.
    pub detail: Option<String>,
    /// Quando a próxima tentativa acontece (só em `Lost`).
    pub retry_in: Option<Duration>,
    pub lost_since: Option<Instant>,
    /// O menu do chip está aberto.
    pub menu_open: bool,
}

impl DaemonState {
    pub fn embedded(socket: PathBuf) -> Self {
        Self::new(Mode::Embedded, socket)
    }

    pub fn connecting(socket: PathBuf) -> Self {
        Self::new(Mode::Connecting, socket)
    }

    fn new(mode: Mode, socket: PathBuf) -> Self {
        Self {
            mode,
            socket,
            server: String::new(),
            cameras: Vec::new(),
            detail: None,
            retry_in: None,
            lost_since: None,
            menu_open: false,
        }
    }

    /// A janela decodifica só para exibir? (Todo modo menos o motor local.)
    pub fn is_daemon_mode(&self) -> bool {
        self.mode != Mode::Embedded
    }

    /// O daemon está respondendo agora.
    pub fn is_connected(&self) -> bool {
        self.mode == Mode::Connected
    }

    /// Aplica um evento da thread de conexão.
    pub fn apply(&mut self, event: LinkEvent) -> Effect {
        match event {
            // Uma nova tentativa não apaga o aviso de "perdido": evita piscar.
            LinkEvent::Connecting => {
                if self.mode != Mode::Lost {
                    self.mode = Mode::Connecting;
                }
                Effect::None
            }
            LinkEvent::Connected { server, cameras } => {
                let was_lost = self.mode == Mode::Lost;
                self.mode = Mode::Connected;
                self.server = server;
                self.cameras = cameras;
                self.detail = None;
                self.retry_in = None;
                self.lost_since = None;
                if was_lost {
                    Effect::Toast(t("Daemon reconectado").into())
                } else {
                    Effect::None
                }
            }
            LinkEvent::Status(cameras) => {
                if self.mode == Mode::Connected {
                    self.cameras = cameras;
                }
                Effect::None
            }
            LinkEvent::Event(ev) => Effect::Event(ev),
            LinkEvent::Lost { reason, retry_in } => {
                if self.mode != Mode::Lost {
                    self.lost_since = Some(Instant::now());
                }
                self.mode = Mode::Lost;
                self.detail = Some(reason);
                self.retry_in = Some(retry_in);
                Effect::None
            }
            LinkEvent::Incompatible { message } => {
                self.mode = Mode::Incompatible;
                self.detail = Some(message);
                self.retry_in = None;
                Effect::None
            }
            LinkEvent::NoPermission { message } => {
                self.mode = Mode::NoPermission;
                self.detail = Some(message);
                self.retry_in = None;
                Effect::None
            }
            LinkEvent::Reply { token, result } => Effect::Reply { token, result },
        }
    }

    /// Texto do chip na barra.
    pub fn chip_label(&self) -> &'static str {
        match self.mode {
            Mode::Embedded => t("Motor local"),
            Mode::Connecting => t("Daemon · conectando…"),
            Mode::Connected => t("Daemon · conectado"),
            Mode::Lost => t("Daemon · sem resposta"),
            Mode::Incompatible => t("Daemon · versão incompatível"),
            Mode::NoPermission => t("Daemon · sem permissão"),
        }
    }

    /// A forma da bolinha do chip: cada modo tem a sua, para o estado nunca
    /// depender só de cor (acessibilidade). Todos existem na fonte embutida.
    pub fn glyph(&self) -> &'static str {
        match self.mode {
            Mode::Embedded => "\u{25CB}",     // ○
            Mode::Connecting => "\u{25CC}",   // ◌
            Mode::Connected => "\u{25CF}",    // ●
            Mode::Lost => "\u{25D0}",         // ◐
            Mode::Incompatible => "\u{25B2}", // ▲
            Mode::NoPermission => "\u{25A0}", // ■
        }
    }

    pub fn tone(&self) -> Tone {
        match self.mode {
            Mode::Embedded => Tone::Neutral,
            Mode::Connected => Tone::Good,
            Mode::Connecting | Mode::Lost => Tone::Warning,
            Mode::Incompatible | Mode::NoPermission => Tone::Error,
        }
    }

    /// Quantas câmeras o daemon diz que estão gravando.
    pub fn recording_count(&self) -> usize {
        self.cameras.iter().filter(|c| c.recording).count()
    }

    /// O que o daemon sabe da câmera `name` (a ligação é pelo **nome**, porque a
    /// janela e o daemon podem ter arquivos de configuração diferentes).
    pub fn info_for(&self, name: &str) -> Option<&CameraInfo> {
        self.cameras.iter().find(|c| c.name == name)
    }

    /// A linha de estado do menu do chip.
    pub fn summary(&self) -> String {
        match self.mode {
            Mode::Embedded => {
                t("Esta janela grava e detecta sozinha. Fechá-la interrompe as gravações.").into()
            }
            Mode::Connecting => t("Conectando ao daemon…").into(),
            Mode::Connected => {
                let n = self.cameras.len();
                let rec = self.recording_count();
                let recording = if rec == 0 {
                    t("nenhuma gravando").to_string()
                } else {
                    tf("gravando {}", &[&rec])
                };
                tf(
                    "Conectado ao {} · {} {} · {}",
                    &[
                        &self.server,
                        &n,
                        &plural(n, "câmera", "câmeras"),
                        &recording,
                    ],
                )
            }
            Mode::Lost => {
                let why = self.reason_suffix();
                match self.retry_in {
                    Some(d) => tf(
                        "Sem resposta do daemon{}. Nova tentativa em {} s.",
                        &[&why, &d.as_secs().max(1)],
                    ),
                    None => tf("Sem resposta do daemon{}.", &[&why]),
                }
            }
            Mode::Incompatible => self
                .detail
                .clone()
                .unwrap_or_else(|| t("A versão do daemon não combina com a da janela.").into()),
            // pela rede o problema é o token, e o link diz qual dos dois (faltou ou foi recusado)
            Mode::NoPermission if self.over_network() => self.detail.clone().unwrap_or_else(|| {
                t("O daemon pela rede recusou o acesso: confira o token (RRV_TOKEN ou o segredo rrv_token).").into()
            }),
            Mode::NoPermission => {
                t("Sem permissão para o socket do daemon (é de outro usuário?).").into()
            }
        }
    }

    /// A conexão é com um daemon em outra máquina (`tcp://host:porta`)?
    fn over_network(&self) -> bool {
        crate::ipc::client::tcp_address(&self.socket).is_some()
    }

    /// ` (motivo)` quando o link disse por que perdeu o contato (ex.: o caminho do
    /// socket passa de 107 bytes), senão nada. Sem o motivo a pessoa só veria "sem
    /// resposta" e não teria como saber que o problema é de configuração.
    fn reason_suffix(&self) -> String {
        match self.detail.as_deref().map(str::trim) {
            Some(r) if !r.is_empty() => format!(" ({r})"),
            _ => String::new(),
        }
    }

    /// O aviso fino sob a barra, quando há algo a dizer.
    pub fn banner(&self) -> Option<Banner> {
        match self.mode {
            Mode::Lost => Some(Banner {
                tone: Tone::Warning,
                text: tf(
                    "Sem resposta do daemon{}. As câmeras podem não estar gravando. Tentando reconectar…",
                    &[&self.reason_suffix()],
                ),
            }),
            Mode::Incompatible => Some(Banner {
                tone: Tone::Error,
                text: t("A janela e o daemon falam versões diferentes do protocolo. Atualize um dos dois.").into(),
            }),
            Mode::NoPermission if self.over_network() => Some(Banner {
                tone: Tone::Error,
                text: self.summary(),
            }),
            Mode::NoPermission => Some(Banner {
                tone: Tone::Error,
                text: tf(
                    "Sem permissão para o socket do daemon ({}).",
                    &[&self.socket.display()],
                ),
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cam(index: usize, name: &str, recording: bool) -> CameraInfo {
        CameraInfo {
            index,
            name: name.into(),
            status: if recording { "recording" } else { "live" }.into(),
            enabled: true,
            recording,
            motion: false,
            stream: "main".into(),
            decoder: None,
            decoder_hw: false,
            ..Default::default()
        }
    }

    fn connected() -> DaemonState {
        let mut s = DaemonState::connecting("/run/rrv/rrv.sock".into());
        s.apply(LinkEvent::Connected {
            server: "rrv-daemon 0.8.0".into(),
            cameras: vec![cam(0, "Portão", true), cam(1, "Garagem", false)],
        });
        s
    }

    #[test]
    fn a_lost_daemon_says_why_so_a_config_problem_is_not_just_silence() {
        let mut s = DaemonState::connecting("/x".into());
        s.apply(LinkEvent::Lost {
            reason: "o caminho do socket tem 137 bytes e o limite do Unix é 107".into(),
            retry_in: Duration::from_secs(4),
        });
        let banner = s.banner().unwrap().text;
        assert!(banner.contains("137 bytes"), "{banner}");
        assert!(s.summary().contains("137 bytes"));
        // sem motivo, o texto continua o de sempre (sem parênteses vazios)
        s.detail = None;
        assert!(!s.banner().unwrap().text.contains("()"));
    }

    #[test]
    fn the_initial_mode_never_starts_a_daemon_and_defaults_to_local() {
        assert_eq!(
            initial_mode(false, false),
            Mode::Embedded,
            "sem socket: motor local"
        );
        assert_eq!(initial_mode(true, true), Mode::Embedded, "--embedded vence");
        assert_eq!(initial_mode(false, true), Mode::Connecting);
    }

    #[test]
    fn embedded_is_the_default_without_a_daemon() {
        let s = DaemonState::embedded("/x".into());
        assert_eq!(s.mode, Mode::Embedded);
        assert!(!s.is_daemon_mode() && !s.is_connected());
        assert_eq!(s.chip_label(), "Motor local");
        assert_eq!(s.tone(), Tone::Neutral);
        assert!(s.banner().is_none());
        assert!(s.summary().contains("interrompe"));
    }

    #[test]
    fn connecting_then_connected_shows_the_daemons_cameras() {
        let s = connected();
        assert_eq!(s.mode, Mode::Connected);
        assert!(s.is_connected() && s.is_daemon_mode());
        assert_eq!(s.chip_label(), "Daemon · conectado");
        assert_eq!(s.tone(), Tone::Good);
        assert_eq!(s.recording_count(), 1);
        assert!(s.summary().contains("rrv-daemon 0.8.0"));
        assert!(s.summary().contains("2 câmeras") && s.summary().contains("gravando 1"));
        assert!(s.banner().is_none());
    }

    #[test]
    fn cameras_are_matched_by_name_not_by_index() {
        let s = connected();
        // a janela pode ter outra ordem (ou outro arquivo de config)
        assert!(s.info_for("Garagem").is_some_and(|c| !c.recording));
        assert!(s.info_for("Portão").is_some_and(|c| c.recording));
        assert!(s.info_for("Quintal").is_none());
    }

    #[test]
    fn status_updates_only_while_connected() {
        let mut s = connected();
        s.apply(LinkEvent::Status(vec![cam(0, "Portão", false)]));
        assert_eq!(s.recording_count(), 0);
        let mut lost = connected();
        lost.apply(LinkEvent::Lost {
            reason: "x".into(),
            retry_in: Duration::from_secs(2),
        });
        lost.apply(LinkEvent::Status(vec![]));
        assert_eq!(
            lost.cameras.len(),
            2,
            "perdido: mantém o último estado conhecido"
        );
    }

    #[test]
    fn losing_the_daemon_warns_and_keeps_the_last_state() {
        let mut s = connected();
        s.apply(LinkEvent::Lost {
            reason: "o daemon fechou a conexão".into(),
            retry_in: Duration::from_secs(4),
        });
        assert_eq!(s.mode, Mode::Lost);
        assert_eq!(s.chip_label(), "Daemon · sem resposta");
        assert_eq!(s.tone(), Tone::Warning);
        assert!(s.lost_since.is_some());
        assert!(s.summary().contains("4 s"), "{}", s.summary());
        let b = s.banner().unwrap();
        assert_eq!(b.tone, Tone::Warning);
        assert!(b.text.contains("podem não estar gravando"));
        assert_eq!(s.cameras.len(), 2, "nada é apagado");
        assert!(
            s.is_daemon_mode(),
            "perder o daemon NÃO volta ao motor local"
        );
    }

    #[test]
    fn a_retry_does_not_make_the_lost_warning_flicker() {
        let mut s = connected();
        s.apply(LinkEvent::Lost {
            reason: "x".into(),
            retry_in: Duration::from_secs(1),
        });
        let since = s.lost_since;
        s.apply(LinkEvent::Connecting);
        assert_eq!(s.mode, Mode::Lost, "continua perdido enquanto tenta");
        s.apply(LinkEvent::Lost {
            reason: "y".into(),
            retry_in: Duration::from_secs(2),
        });
        assert_eq!(s.lost_since, since, "o instante da perda não é refeito");
    }

    #[test]
    fn coming_back_clears_the_warning_and_says_so_once() {
        let mut s = connected();
        s.apply(LinkEvent::Lost {
            reason: "x".into(),
            retry_in: Duration::from_secs(1),
        });
        let fx = s.apply(LinkEvent::Connected {
            server: "rrv-daemon 0.8.0".into(),
            cameras: vec![cam(0, "Portão", true)],
        });
        assert_eq!(fx, Effect::Toast("Daemon reconectado".into()));
        assert_eq!(s.mode, Mode::Connected);
        assert!(s.banner().is_none() && s.lost_since.is_none() && s.retry_in.is_none());
        // a primeira conexão não anuncia "reconectado"
        let mut first = DaemonState::connecting("/x".into());
        assert_eq!(
            first.apply(LinkEvent::Connected {
                server: "s".into(),
                cameras: vec![]
            }),
            Effect::None
        );
    }

    #[test]
    fn incompatible_and_no_permission_are_errors_with_their_own_text() {
        let mut s = DaemonState::connecting("/run/rrv/rrv.sock".into());
        s.apply(LinkEvent::Incompatible {
            message: "versão do protocolo 1 não suportada (o daemon fala 2)".into(),
        });
        assert_eq!(s.mode, Mode::Incompatible);
        assert_eq!(s.tone(), Tone::Error);
        assert!(s.summary().contains("não suportada"));
        assert!(s.banner().unwrap().text.contains("Atualize"));

        s.apply(LinkEvent::NoPermission {
            message: "x".into(),
        });
        assert_eq!(s.mode, Mode::NoPermission);
        assert!(s.banner().unwrap().text.contains("/run/rrv/rrv.sock"));
        assert!(s.is_daemon_mode(), "nenhuma troca automática de modo");
    }

    #[test]
    fn events_and_replies_are_handed_to_the_caller() {
        let mut s = connected();
        let ev = WireEvent {
            camera: 0,
            name: "Portão".into(),
            kind: crate::domain::timeline::EventType::Motion,
            detail: None,
            notification: None,
            unix_secs: 1,
        };
        assert_eq!(s.apply(LinkEvent::Event(ev.clone())), Effect::Event(ev));
        assert_eq!(
            s.apply(LinkEvent::Reply {
                token: 3,
                result: Ok(Response::Ok)
            }),
            Effect::Reply {
                token: 3,
                result: Ok(Response::Ok)
            }
        );
    }

    #[test]
    fn every_mode_has_its_own_glyph_as_well_as_its_own_label() {
        let glyphs: std::collections::HashSet<_> = [
            Mode::Embedded,
            Mode::Connecting,
            Mode::Connected,
            Mode::Lost,
            Mode::Incompatible,
            Mode::NoPermission,
        ]
        .into_iter()
        .map(|m| {
            let mut s = DaemonState::embedded("/x".into());
            s.mode = m;
            s.glyph()
        })
        .collect();
        assert_eq!(glyphs.len(), 6, "dois modos com a mesma forma");
    }

    #[test]
    fn quitting_asks_only_when_it_would_cut_a_local_recording() {
        assert_eq!(Modal::for_quit(0), None, "nada gravando: fecha direto");
        assert_eq!(Modal::for_quit(1), Some(Modal::Quit { recordings: 1 }));
        assert_eq!(Modal::for_quit(4), Some(Modal::Quit { recordings: 4 }));
    }

    #[test]
    fn the_confirmations_say_what_is_at_stake() {
        let local = Modal::UseLocalEngine;
        assert!(local.body().contains("em dobro"));
        assert_eq!(local.confirm_label(), "Usar motor local");
        let one = Modal::Quit { recordings: 1 }.body();
        assert!(
            one.contains("1 gravação em curso") && one.contains("interrompe"),
            "{one}"
        );
        assert!(one.contains("use o daemon"));
        let many = Modal::Quit { recordings: 3 }.body();
        assert!(many.contains("3 gravações em curso"), "{many}");
    }

    #[test]
    fn the_chip_never_relies_on_colour_alone() {
        // todo modo tem um rótulo de texto distinto (acessibilidade)
        let labels: std::collections::HashSet<_> = [
            Mode::Embedded,
            Mode::Connecting,
            Mode::Connected,
            Mode::Lost,
            Mode::Incompatible,
            Mode::NoPermission,
        ]
        .into_iter()
        .map(|m| {
            let mut s = DaemonState::embedded("/x".into());
            s.mode = m;
            s.chip_label()
        })
        .collect();
        assert_eq!(labels.len(), 6);
    }

    #[test]
    fn a_refused_token_over_the_network_names_the_token_not_the_socket() {
        let mut s = DaemonState::connecting(PathBuf::from("tcp://192.168.1.10:7878"));
        s.apply(LinkEvent::NoPermission {
            message: "o daemon em 192.168.1.10:7878 recusou o token".into(),
        });
        assert_eq!(s.mode, Mode::NoPermission);
        let banner = s.banner().unwrap().text;
        assert!(banner.contains("recusou o token"), "{banner}");
        assert!(!banner.contains("socket"), "{banner}");
        // num socket local continua falando do socket
        let mut local = DaemonState::connecting(PathBuf::from("/run/user/1000/rrv/rrv.sock"));
        local.apply(LinkEvent::NoPermission {
            message: "x".into(),
        });
        assert!(local.banner().unwrap().text.contains("socket"));
    }
}
