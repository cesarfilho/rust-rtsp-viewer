//! O catálogo português → inglês. A chave é o texto exatamente como está no código, dentro de `t("…")`.
//! Mantenha em ordem de arquivo/área; o teste `tests/i18n_scan.rs` exige que toda chave usada exista aqui.

pub const EN: &[(&str, &str)] = &[
    // menu (view/menu.rs)
    ("Grade", "Grid"),
    ("Exibição", "Display"),
    ("Sair do modo imersivo", "Exit immersive mode"),
    ("Modo imersivo", "Immersive mode"),
    ("Tela cheia", "Full screen"),
    ("Câmera · {}", "Camera · {}"),
    ("Spotlight", "Spotlight"),
    ("Snapshot", "Snapshot"),
    ("Parar gravação", "Stop recording"),
    ("Gravar", "Record"),
    ("Áudio", "Audio"),
    ("Zonas de movimento", "Motion zones"),
    ("Desativar", "Disable"),
    ("Ativar", "Enable"),
    ("Aparência", "Appearance"),
    ("Gravações", "Recordings"),
    ("Ajuda", "Help"),
    ("Sair", "Quit"),
    // plurais usados nos testes e em avisos
    ("câmera", "camera"),
    ("câmeras", "cameras"),
    // ícones das células (view/cell_overlay.rs)
    ("Capturar imagem  (s)", "Capture image  (s)"),
    ("Gravar  (r)", "Record  (r)"),
    ("Parar gravação  (r)", "Stop recording  (r)"),
    ("Ouvir áudio  (m)", "Listen to audio  (m)"),
    ("Silenciar  (m)", "Mute  (m)"),
    ("Ampliar câmera  (f)", "Enlarge camera  (f)"),
    ("Snapshot  (s)", "Snapshot  (s)"),
    ("Zonas de movimento ({})", "Motion zones ({})"),
    ("Conectando\u{2026}", "Connecting\u{2026}"),
    ("Reconectando", "Reconnecting"),
    ("Pausada", "Paused"),
    ("Desativada", "Disabled"),
    ("Offline", "Offline"),
    // editor de zonas e grade (view/flex_layout.rs)
    (
        "Clique no vídeo para marcar os cantos da zona",
        "Click the video to mark the zone's corners",
    ),
    (
        "{} ponto(s) · Enter ou clique no 1º ponto conclui",
        "{} point(s) · Enter or click the 1st point to finish",
    ),
    ("Concluir", "Finish"),
    ("Desfazer", "Undo"),
    ("Limpar", "Clear"),
    ("Nenhuma câmera configurada", "No camera configured"),
    ("DESATIVADA", "DISABLED"),
    // ajuda de atalhos (view/overlays.rs)
    ("Atalhos de teclado", "Keyboard shortcuts"),
    (
        "Modo imersivo (esconde toda a interface)",
        "Immersive mode (hides all the interface)",
    ),
    (
        "Spotlight da câmera selecionada",
        "Spotlight the selected camera",
    ),
    (
        "Câmera anterior / próxima (no spotlight)",
        "Previous / next camera (in spotlight)",
    ),
    ("Tela cheia do SO + imersivo", "OS full screen + immersive"),
    (
        "Alternar layout (grade / flex)",
        "Switch layout (grid / flex)",
    ),
    (
        "Densidade da grade (Auto/2x2/3x3/4x4)",
        "Grid density (Auto/2x2/3x3/4x4)",
    ),
    ("Página anterior / seguinte", "Previous / next page"),
    (
        "Ligar / desligar o carrossel de páginas",
        "Turn the page carousel on / off",
    ),
    ("Selecionar câmera", "Select camera"),
    ("Alternar seleção", "Toggle selection"),
    ("Iniciar / parar gravação", "Start / stop recording"),
    ("Mudo  ·  volume", "Mute  ·  volume"),
    ("Mostrar / ocultar a sidebar", "Show / hide the sidebar"),
    ("Alternar aba da sidebar", "Switch sidebar tab"),
    ("Filtrar câmeras", "Filter cameras"),
    (
        "Gravações: linha do tempo e player",
        "Recordings: timeline and player",
    ),
    (
        "Zonas de movimento: concluir / desfazer",
        "Motion zones: finish / undo",
    ),
    (
        "Voltar: ajuda \u{2192} spotlight \u{2192} imersivo \u{2192} menu \u{2192} busca",
        "Back: help \u{2192} spotlight \u{2192} immersive \u{2192} menu \u{2192} search",
    ),
    ("Mostrar / ocultar esta ajuda", "Show / hide this help"),
    (
        "Clique para selecionar \u{00B7} duplo-clique para spotlight \u{00B7} clique direito para ações",
        "Click to select \u{00B7} double-click for spotlight \u{00B7} right-click for actions",
    ),
    // barra lateral (sidebar/*)
    ("Filtrar câmeras  (/)", "Filter cameras  (/)"),
    ("Todas {}", "All {}"),
    ("Câmera não encontrada", "Camera not found"),
    ("Selecione uma câmera", "Select a camera"),
    ("Sem dados ainda…", "No data yet…"),
    ("no ar há {}", "up for {}"),
    ("Codec", "Codec"),
    ("Decoder", "Decoder"),
    ("Resolução", "Resolution"),
    ("Fluxo", "Stream"),
    ("Bitrate", "Bitrate"),
    ("STREAM", "STREAM"),
    ("REDE", "NETWORK"),
    ("Latência", "Latency"),
    ("Jitter", "Jitter"),
    ("Perda", "Loss"),
    ("Reconexões", "Reconnections"),
    ("GRAVAÇÃO", "RECORDING"),
    ("Ativa há", "Recording for"),
    ("Avançado", "Advanced"),
    ("CONTADORES", "COUNTERS"),
    ("Decode", "Decode"),
    ("Frames perdidos", "Dropped frames"),
    ("Erros de decode", "Decode errors"),
    ("Frames", "Frames"),
    ("Luma", "Luma"),
    ("Parado há", "Still for"),
    ("Último erro", "Last error"),
    ("Nenhum evento na última hora", "No events in the last hour"),
    (
        "{} eventos (última hora) · clique para abrir a câmera",
        "{} events (last hour) · click to open the camera",
    ),
    ("Câmeras", "Cameras"),
    ("Inspetor", "Inspector"),
    ("Diagnóstico", "Diagnostics"),
    ("Eventos", "Events"),
    // barra de ferramentas e aviso do daemon (view/toolbar.rs, view/daemon.rs)
    ("sem câmeras", "no cameras"),
    ("Reconectar agora", "Reconnect now"),
    ("Usar o motor local…", "Use the local engine…"),
    (
        "Copiar comando para iniciar o daemon",
        "Copy the command to start the daemon",
    ),
    ("Copiar caminho do socket", "Copy the socket path"),
    ("Usar motor local", "Use local engine"),
    ("Cancelar", "Cancel"),
    // estado do daemon (daemon.rs)
    ("Usar o motor local?", "Use the local engine?"),
    (
        "Sair e interromper as gravações?",
        "Quit and stop the recordings?",
    ),
    (
        "Isto começa a gravar daqui. Se o daemon ainda estiver gravando, haverá gravações em dobro. Continuar?",
        "This starts recording from here. If the daemon is still recording, you will get duplicate recordings. Continue?",
    ),
    (
        "Há 1 gravação em curso. Sair a interrompe. Para gravar com a janela fechada, use o daemon.",
        "There is 1 recording in progress. Quitting stops it. To record with the window closed, use the daemon.",
    ),
    (
        "Há {} gravações em curso. Sair as interrompe. Para gravar com a janela fechada, use o daemon.",
        "There are {} recordings in progress. Quitting stops them. To record with the window closed, use the daemon.",
    ),
    ("Motor local", "Local engine"),
    ("Daemon · conectando…", "Daemon · connecting…"),
    ("Daemon · conectado", "Daemon · connected"),
    ("Daemon · sem resposta", "Daemon · not responding"),
    (
        "Daemon · versão incompatível",
        "Daemon · incompatible version",
    ),
    ("Daemon · sem permissão", "Daemon · no permission"),
    (
        "Esta janela grava e detecta sozinha. Fechá-la interrompe as gravações.",
        "This window records and detects on its own. Closing it stops the recordings.",
    ),
    ("Conectando ao daemon…", "Connecting to the daemon…"),
    ("nenhuma gravando", "none recording"),
    ("gravando {}", "{} recording"),
    (
        "Conectado ao {} · {} {} · {}",
        "Connected to {} · {} {} · {}",
    ),
    (
        "Sem resposta do daemon{}. Nova tentativa em {} s.",
        "No response from the daemon{}. Retrying in {} s.",
    ),
    (
        "Sem resposta do daemon{}.",
        "No response from the daemon{}.",
    ),
    (
        "A versão do daemon não combina com a da janela.",
        "The daemon's version does not match the window's.",
    ),
    (
        "Sem permissão para o socket do daemon (é de outro usuário?).",
        "No permission for the daemon's socket (is it another user's?).",
    ),
    (
        "Sem resposta do daemon{}. As câmeras podem não estar gravando. Tentando reconectar…",
        "No response from the daemon{}. The cameras may not be recording. Trying to reconnect…",
    ),
    (
        "A janela e o daemon falam versões diferentes do protocolo. Atualize um dos dois.",
        "The window and the daemon speak different protocol versions. Update one of them.",
    ),
    (
        "Sem permissão para o socket do daemon ({}).",
        "No permission for the daemon's socket ({}).",
    ),
    ("Sair mesmo assim", "Quit anyway"),
    ("Daemon reconectado", "Daemon reconnected"),
    // vista de gravações (recordings.rs)
    (
        "A vista Gravações precisa do daemon, e esta janela está no motor local (o chip da barra diz \"Motor local\"). Abra a janela ligada ao daemon: RRV_SOCKET={} ou --daemon <socket>",
        "The Recordings view needs the daemon, and this window is on the local engine (the bar chip says \"Local engine\"). Open the window connected to the daemon: RRV_SOCKET={} or --daemon <socket>",
    ),
    (
        "Conectando ao daemon… tente de novo em instantes",
        "Connecting to the daemon… try again in a moment",
    ),
    (
        "Sem contato com o daemon agora (veja o aviso sob a barra); a vista Gravações volta quando ele responder",
        "No contact with the daemon right now (see the notice under the bar); the Recordings view returns when it answers",
    ),
    (
        "A janela e o daemon falam versões diferentes do protocolo; atualize um dos dois",
        "The window and the daemon speak different protocol versions; update one of them",
    ),
    (
        "Sem permissão para o socket do daemon (é de outro usuário?); a vista Gravações precisa dele",
        "No permission for the daemon's socket (is it another user's?); the Recordings view needs it",
    ),
    (
        "Gravações do daemon encontradas em {}",
        "Daemon recordings found in {}",
    ),
    (
        "Toque uma gravação para protegê-la",
        "Play a recording to protect it",
    ),
    (
        "Trecho protegido: a retenção não o apaga",
        "Segment protected: retention will not delete it",
    ),
    (
        "Trecho solto: volta a valer a retenção",
        "Segment released: retention applies again",
    ),
    (
        "Não consegui mudar a proteção: {}",
        "Could not change the protection: {}",
    ),
    ("Clipe salvo: {} ({} KiB)", "Clip saved: {} ({} KiB)"),
    ("Não consegui exportar: {}", "Could not export: {}"),
    ("Sem gravação neste instante", "No recording at this moment"),
    (
        "Sem gravação aqui; indo para o próximo trecho",
        "No recording here; jumping to the next segment",
    ),
    (
        "Arquivo não encontrado: {}. Ajuste [recording] dir da janela para a pasta de gravações do daemon",
        "File not found: {}. Set the window's [recording] dir to the daemon's recordings folder",
    ),
    (
        "Não consegui abrir o player: {}",
        "Could not open the player: {}",
    ),
    (
        "Não consegui abrir a gravação: {}",
        "Could not open the recording: {}",
    ),
    (
        "No máximo {} câmeras lado a lado: tire uma antes",
        "At most {} cameras side by side: remove one first",
    ),
    (
        "Toque uma gravação para marcar o início e o fim do clipe",
        "Play a recording to mark the clip's start and end",
    ),
    (
        "Marque o início (I) e o fim (O) do clipe antes de exportar",
        "Mark the clip's start (I) and end (O) before exporting",
    ),
    (
        "O fim do clipe precisa ser depois do início",
        "The clip's end must be after its start",
    ),
    ("Exportando o clipe…", "Exporting the clip…"),
    ("Fechar  Esc", "Close  Esc"),
    ("Carregando…", "Loading…"),
    (
        "Não consegui carregar o histórico: {}",
        "Could not load the history: {}",
    ),
    (
        "Nada gravado neste período. Ative a gravação por movimento ou a manual (r).",
        "Nothing recorded in this period. Turn on motion recording or manual recording (r).",
    ),
    (
        "Clique na linha do tempo para ver a gravação",
        "Click the timeline to see the recording",
    ),
    ("Tocar", "Play"),
    ("Pausar", "Pause"),
    ("Quadro", "Frame"),
    ("Início  I", "Start  I"),
    ("Fim  O", "End  O"),
    ("Soltar  P", "Release  P"),
    ("Proteger  P", "Protect  P"),
    ("Exportar  E", "Export  E"),
    (
        "Clique para ver · roda do mouse aproxima · ‹ › desloca · Espaço toca/pausa · setas esquerda/direita pulam 10 s",
        "Click to view · mouse wheel zooms · ‹ › pan · Space plays/pauses · left/right arrows skip 10 s",
    ),
    ("Comparar:", "Compare:"),
    ("Movimento", "Motion"),
    ("Gravação iniciada", "Recording started"),
    ("Gravação parou", "Recording stopped"),
    ("Câmera offline", "Camera offline"),
    ("Câmera online", "Camera online"),
    ("Foto", "Photo"),
    ("Disco quase cheio", "Disk almost full"),
    ("Objeto", "Object"),
    ("Evento", "Event"),
    ("Nenhum evento neste período", "No events in this period"),
    ("todos", "all"),
    ("Objeto: {}", "Object: {}"),
    ("Só movimento", "Motion only"),
    // avisos da janela (update.rs)
    (
        "Zonas: clique para marcar os pontos · Enter conclui · Esc sai",
        "Zones: click to mark the points · Enter finishes · Esc exits",
    ),
    (
        "Zonas removidas: o quadro inteiro conta",
        "Zones removed: the whole frame counts",
    ),
    ("Carrossel ligado ({}s)", "Carousel on ({}s)"),
    ("Carrossel desligado", "Carousel off"),
    ("Carrossel: {}s", "Carousel: {}s"),
    (
        "Snapshot: {} · clique para abrir a pasta",
        "Snapshot: {} · click to open the folder",
    ),
    ("Falha no snapshot: {}", "Snapshot failed: {}"),
    ("Reconectando ao daemon…", "Reconnecting to the daemon…"),
    (
        "Comando copiado: docker compose up -d",
        "Command copied: docker compose up -d",
    ),
    ("Caminho do socket copiado", "Socket path copied"),
    (
        "Uma zona precisa de pelo menos 3 pontos",
        "A zone needs at least 3 points",
    ),
    (
        "Zona sem área: os pontos estão alinhados ou repetidos",
        "Zone has no area: the points are collinear or repeated",
    ),
    ("Zona {}", "Zone {}"),
    ("Zona salva", "Zone saved"),
    (
        "Motor local ativado: esta janela agora grava e detecta",
        "Local engine on: this window now records and detects",
    ),
    (
        "Disco das gravações quase cheio: {}",
        "Recordings disk almost full: {}",
    ),
    (
        "Sem conexão com o daemon: tente de novo quando ele voltar",
        "No connection to the daemon: try again when it is back",
    ),
    (
        "O daemon não conhece a câmera '{}'",
        "The daemon does not know the camera '{}'",
    ),
    (
        "Gravação iniciada no daemon",
        "Recording started on the daemon",
    ),
    (
        "Gravação parada no daemon",
        "Recording stopped on the daemon",
    ),
    ("Zona salva no daemon", "Zone saved on the daemon"),
    (
        "Não foi possível salvar a zona: {}",
        "Could not save the zone: {}",
    ),
    ("Falha na gravação: {}", "Recording failed: {}"),
    (
        "O daemon não aplicou '{}': {}",
        "The daemon did not apply '{}': {}",
    ),
    ("Selecione uma câmera primeiro", "Select a camera first"),
    ("Rajada 1/{}", "Burst 1/{}"),
    ("Rajada concluída: {} quadros", "Burst finished: {} frames"),
    ("Gravação parada", "Recording stopped"),
    (
        "Áudio desativado no config.toml",
        "Audio disabled in config.toml",
    ),
    // estados, diagnóstico e avisos do núcleo
    ("Ao vivo", "Live"),
    ("Gravando", "Recording"),
    (
        "latência alta — considere reduzir --cache",
        "high latency — consider lowering --cache",
    ),
    (
        "variação de banda — ver CPU/rede",
        "bandwidth variation — check CPU/network",
    ),
    (
        "perda de pacotes — verificar interferência",
        "packet loss — check for interference",
    ),
    (
        "múltiplas reconexões — RTSP ou rede instável",
        "multiple reconnections — RTSP or unstable network",
    ),
    (
        "cena parada > 5min — câmera travada ou cena realmente parada?",
        "static scene > 5min — camera frozen or the scene really is still?",
    ),
    (
        "câmera coberta (luma ≤ 2/255) há ~30s — verificar obstrução",
        "camera covered (luma ≤ 2/255) for ~30s — check for an obstruction",
    ),
    (
        "câmera coberta (luma ≤ 2/255) há ~1min — verificar obstrução",
        "camera covered (luma ≤ 2/255) for ~1min — check for an obstruction",
    ),
    (
        "câmera coberta (luma ≤ 2/255) há ~2min — verificar obstrução",
        "camera covered (luma ≤ 2/255) for ~2min — check for an obstruction",
    ),
    (
        "câmera coberta (luma ≤ 2/255) há ~3min — verificar obstrução",
        "camera covered (luma ≤ 2/255) for ~3min — check for an obstruction",
    ),
    (
        "câmera coberta (luma ≤ 2/255) há 5min+ — vandalismo?",
        "camera covered (luma ≤ 2/255) for 5min+ — vandalism?",
    ),
    (
        "cena muito escura (luma ≤ 5/255) há ~30s — verificar cobertura",
        "scene very dark (luma ≤ 5/255) for ~30s — check for a cover",
    ),
    (
        "cena muito escura (luma ≤ 5/255) há ~1min — verificar cobertura",
        "scene very dark (luma ≤ 5/255) for ~1min — check for a cover",
    ),
    (
        "cena muito escura (luma ≤ 5/255) há ~2min — verificar cobertura",
        "scene very dark (luma ≤ 5/255) for ~2min — check for a cover",
    ),
    (
        "cena muito escura (luma ≤ 5/255) há ~3min — verificar cobertura",
        "scene very dark (luma ≤ 5/255) for ~3min — check for a cover",
    ),
    (
        "cena muito escura (luma ≤ 5/255) há 5min+ — vandalismo?",
        "scene very dark (luma ≤ 5/255) for 5min+ — vandalism?",
    ),
    (
        "luma muito baixa há muito tempo — verificar câmera",
        "luma very low for a long time — check the camera",
    ),
    (
        "câmera ofuscada (luma ≥ 250/255) há ~30s — luz direta ou laser?",
        "camera blinded (luma ≥ 250/255) for ~30s — direct light or laser?",
    ),
    (
        "câmera ofuscada (luma ≥ 250/255) há ~1min — luz direta ou laser?",
        "camera blinded (luma ≥ 250/255) for ~1min — direct light or laser?",
    ),
    (
        "câmera ofuscada (luma ≥ 250/255) há ~2min — luz direta ou laser?",
        "camera blinded (luma ≥ 250/255) for ~2min — direct light or laser?",
    ),
    (
        "câmera ofuscada (luma ≥ 250/255) há ~3min — vandalismo?",
        "camera blinded (luma ≥ 250/255) for ~3min — vandalism?",
    ),
    (
        "câmera ofuscada (luma ≥ 250/255) há 5min+ — vandalismo?",
        "camera blinded (luma ≥ 250/255) for 5min+ — vandalism?",
    ),
    ("Movimento detectado", "Motion detected"),
    ("Sem sinal de vídeo", "No video signal"),
    ("Detecção: {}", "Detection: {}"),
    (
        "tentativa {} · reconectando agora",
        "attempt {} · reconnecting now",
    ),
    ("tentativa {} · próxima em {}s", "attempt {} · next in {}s"),
    ("tentativa {}", "attempt {}"),
];
