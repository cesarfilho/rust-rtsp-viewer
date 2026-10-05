//! Exportar um clipe (plano 3.5): um intervalo de tempo vira um `.mp4` **sem reencode**.
//!
//! Cada segmento é lido como quadros já codificados (`matroskademux ! parsebin ! appsink`),
//! o intervalo é escolhido **a partir do keyframe anterior** ao pedido (a imagem nunca
//! começa suja) e os quadros vão, com o tempo refeito a partir de zero, para um único
//! `appsrc ! mp4mux ! filesink`. Isso junta vários segmentos sem passo intermediário.
//!
//! Só vídeo H.264/H.265 (o que o daemon grava hoje). O clipe pode começar até um GOP antes
//! do pedido; nunca depois.

use std::path::{Path, PathBuf};

use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use crate::infrastructure::launch::quote_launch_value;

/// Um segmento gravado que pode entrar no clipe.
#[derive(Debug, Clone)]
pub struct ClipSource {
    pub path: PathBuf,
    /// Quando o segmento começou (Unix ms).
    pub start_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipInfo {
    pub bytes: u64,
    pub pieces: usize,
}

const CAPS: &str = "video/x-h264,stream-format=avc,alignment=au;\
                    video/x-h265,stream-format=hvc1,alignment=au";

/// Exporta `[from_ms, to_ms]` (Unix ms) dos `sources` (do mais antigo ao mais novo) para
/// `dest` (`.mp4`).
pub fn export_clip(
    sources: &[ClipSource],
    from_ms: i64,
    to_ms: i64,
    dest: &Path,
) -> Result<ClipInfo, String> {
    gst::init().map_err(|e| format!("GStreamer: {e}"))?;
    if from_ms >= to_ms {
        return Err("intervalo vazio ou invertido".into());
    }
    if sources.is_empty() {
        return Err("não há gravação nesse intervalo".into());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("pasta de destino: {e}"))?;
    }
    let result = remux(sources, from_ms, to_ms, dest);
    if result.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    result
}

/// A saída: um pipeline `appsrc ! mp4mux ! filesink` criado quando chega o primeiro quadro
/// (é dele que se conhecem os caps).
struct Out<'a> {
    dest: &'a Path,
    pipeline: Option<(gst::Pipeline, gst_app::AppSrc)>,
    /// Instante (Unix ms) do primeiro quadro escrito: vira o tempo zero do clipe.
    origin_ms: Option<i64>,
}

impl Out<'_> {
    fn push(&mut self, sample: &gst::Sample, abs_ms: i64) -> Result<(), String> {
        if self.pipeline.is_none() {
            let caps = sample.caps().ok_or("quadro sem caps")?;
            let appsrc = gst::ElementFactory::make("appsrc")
                .property("caps", caps.to_owned())
                .property("format", gst::Format::Time)
                .property("block", true)
                .build()
                .map_err(|e| e.to_string())?
                .downcast::<gst_app::AppSrc>()
                .map_err(|_| "appsrc".to_string())?;
            let mux = gst::ElementFactory::make("mp4mux")
                .build()
                .map_err(|e| format!("mp4mux: {e}"))?;
            let sink = gst::ElementFactory::make("filesink")
                .property("location", self.dest.to_string_lossy().to_string())
                .build()
                .map_err(|e| e.to_string())?;
            let pipeline = gst::Pipeline::new();
            pipeline
                .add_many([appsrc.upcast_ref(), &mux, &sink])
                .map_err(|e| e.to_string())?;
            mux.link(&sink).map_err(|e| format!("ligar a saída: {e}"))?;
            // O pad de vídeo se pede pelo nome: `link` escolheria `audio_%u`, porque o
            // appsrc ainda não tem caps que o distingam.
            let video_pad = mux
                .request_pad_simple("video_%u")
                .ok_or("o mp4mux recusou um pad de vídeo")?;
            appsrc
                .static_pad("src")
                .ok_or("appsrc sem pad")?
                .link(&video_pad)
                .map_err(|e| format!("ligar a saída: {e:?}"))?;
            pipeline
                .set_state(gst::State::Playing)
                .map_err(|e| format!("saída: {e:?}"))?;
            self.pipeline = Some((pipeline, appsrc));
        }
        let origin = *self.origin_ms.get_or_insert(abs_ms);
        let shift = gst::ClockTime::from_mseconds((abs_ms - origin).max(0) as u64);
        let src_buf = sample.buffer().ok_or("quadro sem dados")?;
        let mut buf = src_buf.copy();
        {
            let b = buf.get_mut().ok_or("buffer compartilhado")?;
            let file_pts = src_buf.pts().unwrap_or(gst::ClockTime::ZERO);
            b.set_pts(shift);
            if let Some(dts) = src_buf.dts() {
                // mantém a distância pts→dts original
                let lead = file_pts.saturating_sub(dts);
                b.set_dts(shift.saturating_sub(lead));
            }
        }
        let (pipeline, appsrc) = self.pipeline.as_ref().ok_or("sem saída")?;
        // A saída que falhou deixa de consumir e o `push_buffer` bloquearia para sempre.
        if let Some(m) = pipeline
            .bus()
            .and_then(|b| b.pop_filtered(&[gst::MessageType::Error]))
            && let gst::MessageView::Error(e) = m.view()
        {
            return Err(format!("mp4: {} ({:?})", e.error(), e.debug()));
        }
        appsrc
            .push_buffer(buf)
            .map(|_| ())
            .map_err(|e| format!("escrever: {e:?}"))
    }

    fn finish(self) -> Result<(), String> {
        let Some((pipeline, appsrc)) = self.pipeline else {
            return Err("o intervalo não tem vídeo".into());
        };
        let _ = appsrc.end_of_stream();
        let r = match pipeline.bus().and_then(|b| {
            b.timed_pop_filtered(
                gst::ClockTime::from_seconds(60),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            )
        }) {
            Some(m) => match m.view() {
                gst::MessageView::Error(e) => Err(format!("mp4: {}", e.error())),
                _ => Ok(()),
            },
            None => Err("mp4: tempo esgotado".into()),
        };
        let _ = pipeline.set_state(gst::State::Null);
        r
    }
}

fn remux(
    sources: &[ClipSource],
    from_ms: i64,
    to_ms: i64,
    dest: &Path,
) -> Result<ClipInfo, String> {
    let mut out = Out {
        dest,
        pipeline: None,
        origin_ms: None,
    };
    let mut pieces = 0;
    for s in sources {
        if s.start_ms > to_ms {
            continue;
        }
        if read_segment(s, from_ms, to_ms, &mut out)? {
            pieces += 1;
        }
    }
    out.finish()?;
    let bytes = std::fs::metadata(dest).map_err(|e| e.to_string())?.len();
    Ok(ClipInfo { bytes, pieces })
}

/// Lê um segmento e entrega à saída os quadros do intervalo, do keyframe anterior em
/// diante. Devolve se algum quadro entrou no clipe.
fn read_segment(
    s: &ClipSource,
    from_ms: i64,
    to_ms: i64,
    out: &mut Out<'_>,
) -> Result<bool, String> {
    let desc = format!(
        "filesrc location={} ! matroskademux ! parsebin ! appsink name=sink sync=false",
        quote_launch_value(&s.path.to_string_lossy())
    );
    let pipeline = gst::parse::launch(&desc)
        .map_err(|e| format!("abrir {}: {e}", s.path.display()))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| "não é um pipeline".to_string())?;
    let sink = pipeline
        .by_name("sink")
        .ok_or("sem appsink")?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| "appsink".to_string())?;
    sink.set_caps(Some(&CAPS.parse::<gst::Caps>().map_err(|e| e.to_string())?));
    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| format!("ler {}: {e:?}", s.path.display()))?;

    let mut gop: Vec<(gst::Sample, i64)> = Vec::new();
    let mut emitting = false;
    let mut any = false;
    let result = (|| -> Result<(), String> {
        loop {
            let sample = match sink.try_pull_sample(gst::ClockTime::from_seconds(20)) {
                Some(smp) => smp,
                None => {
                    if sink.is_eos() {
                        return Ok(());
                    }
                    // sem quadro e sem EOS: erro no bus ou arquivo parado
                    if let Some(m) = pipeline
                        .bus()
                        .and_then(|b| b.pop_filtered(&[gst::MessageType::Error]))
                        && let gst::MessageView::Error(e) = m.view()
                    {
                        return Err(format!("ler {}: {}", s.path.display(), e.error()));
                    }
                    return Err(format!("ler {}: tempo esgotado", s.path.display()));
                }
            };
            let Some(buf) = sample.buffer() else { continue };
            let pts = buf.pts().unwrap_or(gst::ClockTime::ZERO).mseconds() as i64;
            let abs = s.start_ms + pts;
            let key = !buf.flags().contains(gst::BufferFlags::DELTA_UNIT);
            if emitting {
                if abs > to_ms {
                    return Ok(());
                }
                out.push(&sample, abs)?;
                any = true;
                continue;
            }
            if key {
                gop.clear();
            }
            gop.push((sample, abs));
            if abs >= from_ms {
                // chegou ao pedido: solta o GOP guardado (do keyframe até aqui)
                emitting = true;
                for (smp, a) in std::mem::take(&mut gop) {
                    if a > to_ms {
                        return Ok(());
                    }
                    out.push(&smp, a)?;
                    any = true;
                }
            }
        }
    })();
    let _ = pipeline.set_state(gst::State::Null);
    result.map(|()| any)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(name: &str, dir: &Path) -> PathBuf {
        let _ = gst::init();
        let path = dir.join(name);
        let desc = format!(
            "videotestsrc num-buffers=90 ! video/x-raw,format=I420,width=320,height=240,framerate=30/1 \
             ! x264enc tune=zerolatency key-int-max=15 ! h264parse ! matroskamux ! filesink location={}",
            quote_launch_value(&path.to_string_lossy())
        );
        let p = gst::parse::launch(&desc).unwrap();
        p.set_state(gst::State::Playing).unwrap();
        p.bus()
            .unwrap()
            .timed_pop_filtered(
                gst::ClockTime::from_seconds(20),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            )
            .expect("clipe de teste");
        p.set_state(gst::State::Null).unwrap();
        path
    }

    /// Duração em ms e se toca até o EOS.
    fn probe(path: &Path) -> (u64, bool) {
        let desc = format!(
            "filesrc location={} ! decodebin ! fakesink sync=false",
            quote_launch_value(&path.to_string_lossy())
        );
        let p = gst::parse::launch(&desc)
            .unwrap()
            .downcast::<gst::Pipeline>()
            .unwrap();
        p.set_state(gst::State::Paused).unwrap();
        let _ = p.state(gst::ClockTime::from_seconds(5));
        let dur = p
            .query_duration::<gst::ClockTime>()
            .map_or(0, |d| d.mseconds());
        p.set_state(gst::State::Playing).unwrap();
        let ok = p
            .bus()
            .unwrap()
            .timed_pop_filtered(
                gst::ClockTime::from_seconds(20),
                &[gst::MessageType::Eos, gst::MessageType::Error],
            )
            .is_some_and(|m| matches!(m.view(), gst::MessageView::Eos(_)));
        p.set_state(gst::State::Null).unwrap();
        (dur, ok)
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rrv-clip-t-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_clip_inside_one_segment_is_trimmed_without_reencoding() {
        let d = tmp("one");
        let a = make("a.mkv", &d); // 3 s
        let out = d.join("clip.mp4");
        let info = export_clip(
            &[ClipSource {
                path: a,
                start_ms: 10_000,
            }],
            11_000, // 1 s depois do início
            12_500,
            &out,
        )
        .unwrap();
        assert_eq!(info.pieces, 1);
        let (dur, ok) = probe(&out);
        assert!(ok, "o clipe não toca até o fim");
        // 1,5 s pedidos; o keyframe anterior (GOP de 0,5 s) pode somar até 0,5 s
        assert!((1400..=2200).contains(&dur), "duração {dur} ms");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_clip_across_two_segments_is_joined() {
        let d = tmp("two");
        let a = make("a.mkv", &d);
        let b = make("b.mkv", &d);
        let out = d.join("clip.mp4");
        let info = export_clip(
            &[
                ClipSource {
                    path: a,
                    start_ms: 0,
                },
                ClipSource {
                    path: b,
                    start_ms: 3_000,
                },
            ],
            1_000,
            5_000,
            &out,
        )
        .unwrap();
        assert_eq!(info.pieces, 2);
        let (dur, ok) = probe(&out);
        assert!(ok);
        assert!((3800..=4800).contains(&dur), "duração {dur} ms");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn bad_requests_are_clear_errors_and_leave_no_file() {
        let d = tmp("bad");
        let out = d.join("x.mp4");
        assert!(
            export_clip(&[], 0, 10, &out)
                .unwrap_err()
                .contains("não há")
        );
        let a = make("a.mkv", &d);
        let src = [ClipSource {
            path: a,
            start_ms: 0,
        }];
        assert!(export_clip(&src, 10, 10, &out).is_err());
        assert!(
            export_clip(
                &[ClipSource {
                    path: d.join("sumiu.mkv"),
                    start_ms: 0
                }],
                0,
                10,
                &out
            )
            .is_err()
        );
        assert!(!out.exists());
        let _ = std::fs::remove_dir_all(&d);
    }
}
