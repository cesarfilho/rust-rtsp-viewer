//! Histórico persistente (plano 3.2, ADR 0006): segmentos gravados e eventos num
//! SQLite em modo WAL. Os horários são Unix em **milissegundos**.
//!
//! Regras que o resto do código pode contar com:
//! - o arquivo vem **antes** da linha: a retenção apaga o arquivo e só então a linha
//!   ([`Store::delete_segment`]); um arquivo sem linha (queda entre criar e gravar) é
//!   reconhecido na partida por [`Store::reconcile`];
//! - um segmento aberto tem `closed = 0`; se o processo morreu, `reconcile` o fecha com
//!   o tamanho e a data que o arquivo tem em disco, ou remove a linha se o arquivo sumiu;
//! - a escrita no caminho quente passa por [`StoreHandle`] (thread própria, fila): o motor
//!   nunca espera o disco.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use rusqlite::{Connection, OptionalExtension, params};

/// Versão do esquema (`PRAGMA user_version`).
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub id: i64,
    pub camera: String,
    pub path: String,
    pub ts_start: i64,
    /// `None` enquanto aberto.
    pub ts_end: Option<i64>,
    pub bytes: i64,
    pub has_motion: bool,
    pub protected: bool,
    pub closed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StoredEvent {
    pub id: i64,
    pub camera: String,
    pub ts: i64,
    pub kind: String,
    pub label: String,
    pub score: Option<f64>,
    pub segment_id: Option<i64>,
}

/// O que a reconciliação encontrou.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Reconciled {
    /// Segmentos abertos que foram fechados com o que há em disco.
    pub closed: usize,
    /// Linhas removidas porque o arquivo não existe mais.
    pub missing: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("banco de dados: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("o banco é de uma versão mais nova (v{0}) que este programa (v{SCHEMA_VERSION})")]
    TooNew(i64),
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Abre (criando e migrando) o banco em `path`.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::migrate(conn)
    }

    pub fn open_in_memory() -> Result<Self, StoreError> {
        Self::migrate(Connection::open_in_memory()?)
    }

    fn migrate(conn: Connection) -> Result<Self, StoreError> {
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(StoreError::TooNew(version));
        }
        if version < 1 {
            conn.execute_batch(
                "CREATE TABLE segments (
                     id         INTEGER PRIMARY KEY,
                     camera     TEXT NOT NULL,
                     path       TEXT NOT NULL UNIQUE,
                     ts_start   INTEGER NOT NULL,
                     ts_end     INTEGER,
                     bytes      INTEGER NOT NULL DEFAULT 0,
                     has_motion INTEGER NOT NULL DEFAULT 0,
                     protected  INTEGER NOT NULL DEFAULT 0,
                     closed     INTEGER NOT NULL DEFAULT 0
                 );
                 CREATE INDEX segments_by_camera_time ON segments (camera, ts_start);
                 CREATE TABLE events (
                     id         INTEGER PRIMARY KEY,
                     camera     TEXT NOT NULL,
                     ts         INTEGER NOT NULL,
                     kind       TEXT NOT NULL,
                     label      TEXT NOT NULL DEFAULT '',
                     score      REAL,
                     segment_id INTEGER REFERENCES segments (id) ON DELETE SET NULL
                 );
                 CREATE INDEX events_by_camera_time ON events (camera, ts);
                 PRAGMA user_version = 1;",
            )?;
        }
        Ok(Self { conn })
    }

    /// Registra um segmento que acabou de abrir. Repetir o mesmo caminho é inofensivo.
    pub fn segment_opened(&self, camera: &str, path: &str, ts: i64) -> Result<i64, StoreError> {
        self.conn.execute(
            "INSERT INTO segments (camera, path, ts_start) VALUES (?1, ?2, ?3)
             ON CONFLICT (path) DO NOTHING",
            params![camera, path, ts],
        )?;
        Ok(self
            .conn
            .query_row("SELECT id FROM segments WHERE path = ?1", [path], |r| {
                r.get(0)
            })?)
    }

    /// Fecha o segmento `path` com o fim e o tamanho finais.
    pub fn segment_closed(&self, path: &str, ts_end: i64, bytes: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE segments SET ts_end = ?2, bytes = ?3, closed = 1 WHERE path = ?1",
            params![path, ts_end, bytes],
        )?;
        Ok(())
    }

    /// Insere um evento e o liga ao segmento da câmera que cobre `ts`, se houver.
    pub fn insert_event(
        &self,
        camera: &str,
        ts: i64,
        kind: &str,
        label: &str,
        score: Option<f64>,
    ) -> Result<i64, StoreError> {
        let segment: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM segments
                 WHERE camera = ?1 AND ts_start <= ?2 AND (ts_end IS NULL OR ts_end >= ?2)
                 ORDER BY ts_start DESC LIMIT 1",
                params![camera, ts],
                |r| r.get(0),
            )
            .optional()?;
        self.conn.execute(
            "INSERT INTO events (camera, ts, kind, label, score, segment_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![camera, ts, kind, label, score, segment],
        )?;
        if let (Some(id), "motion") = (segment, kind) {
            self.conn
                .execute("UPDATE segments SET has_motion = 1 WHERE id = ?1", [id])?;
        }
        Ok(self.conn.last_insert_rowid())
    }

    fn segment_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Segment> {
        Ok(Segment {
            id: r.get(0)?,
            camera: r.get(1)?,
            path: r.get(2)?,
            ts_start: r.get(3)?,
            ts_end: r.get(4)?,
            bytes: r.get(5)?,
            has_motion: r.get::<_, i64>(6)? != 0,
            protected: r.get::<_, i64>(7)? != 0,
            closed: r.get::<_, i64>(8)? != 0,
        })
    }

    const SEGMENT_COLUMNS: &'static str =
        "id, camera, path, ts_start, ts_end, bytes, has_motion, protected, closed";

    /// Segmentos de `camera` que tocam o intervalo `[from, to]`, do mais antigo ao mais novo.
    pub fn segments_between(
        &self,
        camera: &str,
        from: i64,
        to: i64,
    ) -> Result<Vec<Segment>, StoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM segments
             WHERE camera = ?1 AND ts_start <= ?3 AND (ts_end IS NULL OR ts_end >= ?2)
             ORDER BY ts_start",
            Self::SEGMENT_COLUMNS
        ))?;
        let rows = stmt.query_map(params![camera, from, to], Self::segment_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn events_between(
        &self,
        camera: Option<&str>,
        from: i64,
        to: i64,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, camera, ts, kind, label, score, segment_id FROM events
             WHERE (?1 IS NULL OR camera = ?1) AND ts BETWEEN ?2 AND ?3
             ORDER BY ts",
        )?;
        let rows = stmt.query_map(params![camera, from, to], |r| {
            Ok(StoredEvent {
                id: r.get(0)?,
                camera: r.get(1)?,
                ts: r.get(2)?,
                kind: r.get(3)?,
                label: r.get(4)?,
                score: r.get(5)?,
                segment_id: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn set_protected(&self, segment_id: i64, protected: bool) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE segments SET protected = ?2 WHERE id = ?1",
            params![segment_id, protected as i64],
        )?;
        Ok(())
    }

    /// Soma dos tamanhos de todos os segmentos.
    pub fn total_bytes(&self) -> Result<i64, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT COALESCE(SUM(bytes), 0) FROM segments", [], |r| {
                r.get(0)
            })?)
    }

    /// Os mais antigos **fechados e não protegidos**: candidatos à retenção. `motion_only`
    /// restringe aos que têm movimento (ou aos que não têm, se `false`) quando `Some`.
    pub fn oldest_deletable(
        &self,
        limit: usize,
        before: Option<i64>,
    ) -> Result<Vec<Segment>, StoreError> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {} FROM segments
             WHERE closed = 1 AND protected = 0 AND (?2 IS NULL OR ts_start < ?2)
             ORDER BY ts_start LIMIT ?1",
            Self::SEGMENT_COLUMNS
        ))?;
        let rows = stmt.query_map(params![limit as i64, before], Self::segment_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Apaga o **arquivo** e só então a linha. Se o arquivo não existe, só a linha vai.
    /// Se o arquivo não pode ser apagado, a linha fica (o próximo ciclo tenta de novo).
    pub fn delete_segment(&self, segment: &Segment) -> Result<(), StoreError> {
        match std::fs::remove_file(&segment.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        self.conn
            .execute("DELETE FROM segments WHERE id = ?1", [segment.id])?;
        Ok(())
    }

    /// Na partida: fecha o que ficou aberto por uma queda e tira o que não tem arquivo.
    pub fn reconcile(&self) -> Result<Reconciled, StoreError> {
        let mut out = Reconciled::default();
        let rows: Vec<Segment> = {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT {} FROM segments WHERE closed = 0",
                Self::SEGMENT_COLUMNS
            ))?;
            stmt.query_map([], Self::segment_from_row)?
                .collect::<Result<_, _>>()?
        };
        for seg in rows {
            match std::fs::metadata(&seg.path) {
                Ok(meta) if meta.len() > 0 => {
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_millis() as i64)
                        .unwrap_or(seg.ts_start);
                    self.segment_closed(&seg.path, mtime.max(seg.ts_start), meta.len() as i64)?;
                    out.closed += 1;
                }
                // Sumiu ou ficou vazio (morreu antes do primeiro quadro): sem o que mostrar.
                _ => {
                    let _ = std::fs::remove_file(&seg.path);
                    self.conn
                        .execute("DELETE FROM segments WHERE id = ?1", [seg.id])?;
                    out.missing += 1;
                }
            }
        }
        // Linhas fechadas cujo arquivo foi apagado por fora.
        let closed: Vec<Segment> = {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT {} FROM segments WHERE closed = 1",
                Self::SEGMENT_COLUMNS
            ))?;
            stmt.query_map([], Self::segment_from_row)?
                .collect::<Result<_, _>>()?
        };
        for seg in closed {
            if !Path::new(&seg.path).exists() {
                self.conn
                    .execute("DELETE FROM segments WHERE id = ?1", [seg.id])?;
                out.missing += 1;
            }
        }
        Ok(out)
    }
}

/// O que o caminho quente manda para a thread do banco.
#[derive(Debug)]
pub enum StoreCmd {
    SegmentOpened {
        camera: String,
        path: String,
        ts: i64,
    },
    SegmentClosed {
        path: String,
        ts_end: i64,
    },
    Event {
        camera: String,
        ts: i64,
        kind: String,
        label: String,
        score: Option<f64>,
    },
}

/// Escrita assíncrona: uma thread dona de uma conexão, alimentada por um canal.
/// Clonar o handle é barato; a thread termina quando o último handle cai.
#[derive(Clone)]
pub struct StoreHandle {
    tx: Sender<StoreCmd>,
    _join: std::sync::Arc<JoinOnDrop>,
}

struct JoinOnDrop(std::sync::Mutex<Option<JoinHandle<()>>>);

impl Drop for JoinOnDrop {
    fn drop(&mut self) {
        if let Some(h) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = h.join();
        }
    }
}

impl StoreHandle {
    /// Abre o banco em `path`, reconcilia e começa a atender a fila.
    pub fn spawn(path: PathBuf) -> Result<Self, StoreError> {
        let store = Store::open(&path)?;
        let r = store.reconcile()?;
        if r.closed + r.missing > 0 {
            log::info!(
                "histórico reconciliado: {} segmento(s) fechado(s), {} sem arquivo",
                r.closed,
                r.missing
            );
        }
        let (tx, rx) = mpsc::channel::<StoreCmd>();
        let join = std::thread::Builder::new()
            .name("rrv-store".into())
            .spawn(move || {
                for cmd in rx {
                    if let Err(e) = apply(&store, cmd) {
                        log::warn!("histórico: {e}");
                    }
                }
            })?;
        Ok(Self {
            tx,
            _join: std::sync::Arc::new(JoinOnDrop(std::sync::Mutex::new(Some(join)))),
        })
    }

    pub fn send(&self, cmd: StoreCmd) {
        // Se a thread morreu não há o que fazer: gravar vídeo é mais importante.
        let _ = self.tx.send(cmd);
    }
}

fn apply(store: &Store, cmd: StoreCmd) -> Result<(), StoreError> {
    match cmd {
        StoreCmd::SegmentOpened { camera, path, ts } => {
            store.segment_opened(&camera, &path, ts)?;
        }
        StoreCmd::SegmentClosed { path, ts_end } => {
            let bytes = std::fs::metadata(&path)
                .map(|m| m.len() as i64)
                .unwrap_or(0);
            store.segment_closed(&path, ts_end, bytes)?;
        }
        StoreCmd::Event {
            camera,
            ts,
            kind,
            label,
            score,
        } => {
            store.insert_event(&camera, ts, &kind, &label, score)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rrv-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_segment_goes_from_open_to_closed() {
        let s = Store::open_in_memory().unwrap();
        s.segment_opened("garagem", "/r/a.mkv", 1_000).unwrap();
        let open = s.segments_between("garagem", 0, 10_000).unwrap();
        assert_eq!(open.len(), 1);
        assert!(!open[0].closed && open[0].ts_end.is_none());
        s.segment_closed("/r/a.mkv", 5_000, 777).unwrap();
        let done = &s.segments_between("garagem", 0, 10_000).unwrap()[0];
        assert!(done.closed);
        assert_eq!((done.ts_end, done.bytes), (Some(5_000), 777));
        assert_eq!(s.total_bytes().unwrap(), 777);
    }

    #[test]
    fn opening_the_same_path_twice_keeps_one_row() {
        let s = Store::open_in_memory().unwrap();
        let a = s.segment_opened("c", "/r/a.mkv", 1).unwrap();
        let b = s.segment_opened("c", "/r/a.mkv", 2).unwrap();
        assert_eq!(a, b);
        assert_eq!(s.segments_between("c", 0, 10).unwrap().len(), 1);
    }

    #[test]
    fn segments_between_returns_only_those_touching_the_interval() {
        let s = Store::open_in_memory().unwrap();
        for (i, (a, b)) in [(0, 99), (100, 199), (200, 299)].into_iter().enumerate() {
            let p = format!("/r/{i}.mkv");
            s.segment_opened("c", &p, a).unwrap();
            s.segment_closed(&p, b, 1).unwrap();
        }
        s.segment_opened("outra", "/r/x.mkv", 150).unwrap();
        let got = s.segments_between("c", 150, 250).unwrap();
        assert_eq!(
            got.iter().map(|g| g.path.as_str()).collect::<Vec<_>>(),
            ["/r/1.mkv", "/r/2.mkv"]
        );
    }

    #[test]
    fn a_motion_event_links_to_its_segment_and_marks_it() {
        let s = Store::open_in_memory().unwrap();
        let id = s.segment_opened("c", "/r/a.mkv", 100).unwrap();
        s.insert_event("c", 150, "motion", "", Some(0.4)).unwrap();
        s.segment_closed("/r/a.mkv", 200, 1).unwrap();
        s.insert_event("c", 5_000, "offline", "", None).unwrap(); // fora de qualquer segmento
        let ev = s.events_between(Some("c"), 0, 10_000).unwrap();
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].segment_id, Some(id));
        assert_eq!(ev[0].score, Some(0.4));
        assert_eq!(ev[1].segment_id, None);
        assert!(s.segments_between("c", 0, 10_000).unwrap()[0].has_motion);
        assert_eq!(s.events_between(None, 0, 10_000).unwrap().len(), 2);
        assert!(
            s.events_between(Some("outra"), 0, 10_000)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn retention_never_offers_open_or_protected_segments() {
        let s = Store::open_in_memory().unwrap();
        for (i, ts) in [10, 20, 30].into_iter().enumerate() {
            let p = format!("/r/{i}.mkv");
            let id = s.segment_opened("c", &p, ts).unwrap();
            if i < 2 {
                s.segment_closed(&p, ts + 5, 100).unwrap();
            }
            if i == 0 {
                s.set_protected(id, true).unwrap();
            }
        }
        // 0 protegido, 2 ainda aberto: só o 1.
        let c = s.oldest_deletable(10, None).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].path, "/r/1.mkv");
        assert!(s.oldest_deletable(10, Some(15)).unwrap().is_empty());
    }

    #[test]
    fn delete_removes_the_file_then_the_row() {
        let d = tmp("delete");
        let f = d.join("a.mkv");
        std::fs::write(&f, b"x").unwrap();
        let s = Store::open_in_memory().unwrap();
        let p = f.to_string_lossy().to_string();
        s.segment_opened("c", &p, 1).unwrap();
        s.segment_closed(&p, 2, 1).unwrap();
        let seg = s.oldest_deletable(1, None).unwrap().remove(0);
        s.delete_segment(&seg).unwrap();
        assert!(!f.exists());
        assert!(s.segments_between("c", 0, 10).unwrap().is_empty());
        // arquivo já ausente: só a linha, sem erro
        s.segment_opened("c", &p, 1).unwrap();
        s.segment_closed(&p, 2, 1).unwrap();
        let seg = s.oldest_deletable(1, None).unwrap().remove(0);
        s.delete_segment(&seg).unwrap();
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Critério de aceite 1 da spec: uma queda no meio de um segmento não deixa órfão.
    #[test]
    fn reconcile_closes_crashed_segments_and_drops_rows_without_a_file() {
        let d = tmp("reconcile");
        let alive = d.join("vivo.mkv");
        std::fs::write(&alive, vec![0u8; 321]).unwrap();
        let empty = d.join("vazio.mkv");
        std::fs::write(&empty, b"").unwrap();
        let gone = d.join("sumiu.mkv").to_string_lossy().to_string();
        let db = d.join("h.db");

        {
            let s = Store::open(&db).unwrap();
            s.segment_opened("c", &alive.to_string_lossy(), 1_000)
                .unwrap();
            s.segment_opened("c", &empty.to_string_lossy(), 2_000)
                .unwrap();
            s.segment_opened("c", &gone, 3_000).unwrap();
            // "queda": nada foi fechado.
        }
        let s = Store::open(&db).unwrap();
        let r = s.reconcile().unwrap();
        assert_eq!(
            r,
            Reconciled {
                closed: 1,
                missing: 2
            }
        );
        let rows = s.segments_between("c", 0, i64::MAX).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].closed && rows[0].bytes == 321);
        assert!(!empty.exists(), "o arquivo vazio é limpo junto");

        // um arquivo fechado apagado por fora também some
        std::fs::remove_file(&alive).unwrap();
        assert_eq!(s.reconcile().unwrap().missing, 1);
        assert!(s.segments_between("c", 0, i64::MAX).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_database_from_the_future_is_refused() {
        let d = tmp("future");
        let db = d.join("h.db");
        {
            let c = Connection::open(&db).unwrap();
            c.pragma_update(None, "user_version", 99).unwrap();
        }
        assert!(matches!(Store::open(&db), Err(StoreError::TooNew(99))));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_handle_writes_in_the_background_and_flushes_on_drop() {
        let d = tmp("handle");
        let db = d.join("h.db");
        let f = d.join("a.mkv");
        std::fs::write(&f, vec![0u8; 50]).unwrap();
        let p = f.to_string_lossy().to_string();
        let h = StoreHandle::spawn(db.clone()).unwrap();
        h.send(StoreCmd::SegmentOpened {
            camera: "c".into(),
            path: p.clone(),
            ts: 100,
        });
        h.send(StoreCmd::Event {
            camera: "c".into(),
            ts: 120,
            kind: "motion".into(),
            label: String::new(),
            score: None,
        });
        h.send(StoreCmd::SegmentClosed {
            path: p,
            ts_end: 200,
        });
        drop(h); // junta a thread: tudo foi gravado
        let s = Store::open(&db).unwrap();
        let seg = &s.segments_between("c", 0, 1_000).unwrap()[0];
        assert!(seg.closed && seg.bytes == 50 && seg.has_motion);
        assert_eq!(s.events_between(Some("c"), 0, 1_000).unwrap().len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }
}
