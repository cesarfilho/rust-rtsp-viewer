use super::sidebar;
use super::theme::Theme;

pub use crate::domain::view::GridMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Grid,
    Flex,
}

impl LayoutMode {
    pub fn next(self) -> Self {
        match self {
            LayoutMode::Grid => LayoutMode::Flex,
            LayoutMode::Flex => LayoutMode::Grid,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    FrameUpdate,
    ThemeChanged(Theme),
    LanguageChanged(crate::i18n::Lang),
    Sidebar(sidebar::Message),
    LayoutModeChanged(LayoutMode),
    FlexMainSelected(usize),
    /// Grid density preset picked from the toolbar / cycled with `g`.
    GridModeChanged(GridMode),
    /// Advance / go back one grid page (pagination + `[` `]` / PageUp/PageDown).
    NextPage,
    PrevPage,
    /// Start/stop the page carousel (`c`).
    ToggleRotate,
    /// Nudge the carousel dwell time by one step; `true` = longer.
    RotateIntervalStep(bool),
    WindowResized(iced::Size),
    Snapshot,
    ToggleRecording,
    ToggleAudio,
    VolumeUp,
    VolumeDown,
    ToggleSelection,
    CycleLayout,
    ToggleSidebar,
    CycleSidebarTab,
    ToggleFullscreen,
    ExitFullscreen,
    /// Toggle immersive mode — hide all chrome, grid edge-to-edge (`h`).
    ToggleImmersive,
    /// Put one camera full-bleed (double-click a cell, or `f` / `Enter`).
    EnterSpotlight(usize),
    /// Step the spotlighted camera to the previous/next one (`true` = next).
    SpotlightStep(bool),
    /// Leave spotlight / immersive (one level), driven by `Esc` cascade.
    ExitFocus,
    /// Pointer touched the top edge (or a key was pressed) — show the reveal rail.
    RevealChrome,
    /// Toggle the toolbar overflow (`⋯`) menu.
    ToggleOverflowMenu,
    /// Pointer entered / left a grid cell (drives the on-cell action row).
    CellHoverEnter(usize),
    CellHoverExit,
    /// Pointer position over the video area (window coords), for anchoring the
    /// right-click command menu where the cursor is.
    PointerMoved(iced::Point),
    /// Dispatched by every command-menu row: closes the menu(s), then runs the
    /// wrapped message. Keeps the menu from lingering after a pick.
    RunMenuCommand(Box<Message>),
    SelectCamera(usize),
    Quit,
    /// `Ctrl+Q` and the window's close button: asks first when closing would cut
    /// off a recording this window is making itself.
    QuitRequested,
    /// Open / close the daemon chip's menu.
    ToggleDaemonMenu,
    /// "Reconectar agora".
    DaemonReconnect,
    /// "Usar o motor local…": asks for confirmation first.
    AskUseLocalEngine,
    CopyDaemonStartCommand,
    CopyDaemonSocket,
    /// The open confirmation was declined / accepted.
    ModalCancel,
    ModalConfirm,
    ShowHelp,
    /// Right-click on a grid cell — open the per-camera command menu.
    ShowContextMenu(usize),
    DismissContextMenu,
    /// Move keyboard focus into the sidebar search box (the `/` key).
    FocusSearch,
    /// Leave the search box, re-enabling single-key shortcuts.
    BlurSearch,
    /// A raw key press, resolved against the current focus state in `update`.
    KeyPressed(iced::keyboard::Key, iced::keyboard::Modifiers),
    /// Open a folder in the file manager (clickable toasts).
    OpenDir(std::path::PathBuf),
    /// A timeline row was clicked — jump to that camera.
    EventClicked(usize),
    /// Open the motion-zone editor on a camera (spotlights it).
    EditZones(usize),
    /// A vertex clicked on the editor canvas, normalized to the video frame.
    ZoneVertex(f64, f64),
    /// The recordings view (timeline, player, export).
    Recordings(super::recordings::RecMsg),
    /// Swallows a click so it does not fall through to the layer below.
    Noop,
    /// Close the polygon being drawn and store it as a zone (`Enter`).
    ZoneFinish,
    /// Drop the last vertex, or the last zone when no polygon is open (`Backspace`).
    ZoneUndo,
    /// Remove every zone of the camera being edited.
    ZoneClear,
    /// Leave the editor, discarding any unfinished polygon (`Esc`).
    ZoneCancel,
    /// A snapshot PNG finished encoding+writing on a background task.
    /// `sequence` is 1 for a lone snapshot / the first burst frame.
    SnapshotSaved {
        camera_idx: usize,
        sequence: u32,
        result: Result<std::path::PathBuf, String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_mode_toggle() {
        assert_eq!(LayoutMode::Grid.next(), LayoutMode::Flex);
        assert_eq!(LayoutMode::Flex.next(), LayoutMode::Grid);
    }

    #[test]
    fn message_clone() {
        let m = Message::SelectCamera(5);
        let _m2 = m.clone();
        assert!(format!("{:?}", m).contains("SelectCamera"));
    }
}
