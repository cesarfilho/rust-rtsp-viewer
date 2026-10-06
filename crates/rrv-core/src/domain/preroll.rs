//! O ring buffer do pré-roll (plano 3.6, ADR 0007): guarda os últimos instantes do vídeo
//! **já codificado**, em GOPs inteiros, para a gravação por movimento poder começar antes
//! do movimento. Puro e genérico no item (na prática, um `gst::Sample`).
//!
//! Um clipe só começa num keyframe, então o ring guarda GOPs inteiros: a história devolvida
//! tem pelo menos `keep_ms` e, no máximo, `keep_ms` mais um GOP (o que a câmera usar).

use std::collections::VecDeque;

/// Um item do ring: o instante (ms, só a ordem e as diferenças importam) e se é keyframe.
struct Entry<T> {
    pts_ms: i64,
    key: bool,
    item: T,
}

/// Teto do que um ring guarda de um GOP só: câmeras com "Smart Codec" (H.264+) esticam o
/// GOP em cena parada (a Intelbras de teste mandava um keyframe a cada ~20 s), e sem teto a
/// memória do ring seguiria o GOP. Passou disso, o ring recomeça no próximo keyframe.
pub const MAX_GOP_SPAN_MS: i64 = 30_000;

pub struct GopRing<T> {
    keep_ms: i64,
    max_span_ms: i64,
    entries: VecDeque<Entry<T>>,
}

impl<T> GopRing<T> {
    pub fn new(keep_ms: i64) -> Self {
        Self {
            keep_ms: keep_ms.max(0),
            max_span_ms: MAX_GOP_SPAN_MS,
            entries: VecDeque::new(),
        }
    }

    /// Com outro teto de GOP (testes).
    pub fn with_max_span(mut self, max_span_ms: i64) -> Self {
        self.max_span_ms = max_span_ms;
        self
    }

    pub fn push(&mut self, pts_ms: i64, key: bool, item: T) {
        // Sem keyframe ainda não há por onde começar um clipe: o que vem antes do
        // primeiro keyframe é descartado.
        if self.entries.is_empty() && !key {
            return;
        }
        self.entries.push_back(Entry { pts_ms, key, item });
        self.trim();
        // Um GOP só, e já passou do teto: não dá para guardar mais sem crescer sem limite.
        // Descarta tudo e espera o próximo keyframe (que reabre o ring).
        if let (Some(first), Some(last)) = (self.entries.front(), self.entries.back())
            && last.pts_ms - first.pts_ms > self.max_span_ms
            && !self.entries.iter().skip(1).any(|e| e.key)
        {
            self.entries.clear();
        }
    }

    /// Descarta o GOP mais antigo enquanto o que sobra, sem ele, ainda cobre `keep_ms`.
    fn trim(&mut self) {
        loop {
            let Some(newest) = self.entries.back().map(|e| e.pts_ms) else {
                return;
            };
            // início do segundo GOP
            let Some(second) = self
                .entries
                .iter()
                .skip(1)
                .position(|e| e.key)
                .map(|i| i + 1)
            else {
                return; // um só GOP: não há o que descartar
            };
            if newest - self.entries[second].pts_ms >= self.keep_ms {
                self.entries.drain(..second);
            } else {
                return;
            }
        }
    }

    /// Quanto vídeo o ring tem agora, em ms.
    pub fn span_ms(&self) -> i64 {
        match (self.entries.front(), self.entries.back()) {
            (Some(a), Some(b)) => b.pts_ms - a.pts_ms,
            _ => 0,
        }
    }

    /// O instante (ms) do início do que o ring guarda (o keyframe mais antigo).
    pub fn oldest_pts_ms(&self) -> Option<i64> {
        self.entries.front().map(|e| e.pts_ms)
    }

    /// O instante do item mais novo (ms).
    pub fn newest_pts_ms(&self) -> Option<i64> {
        self.entries.back().map(|e| e.pts_ms)
    }

    /// O item mais novo.
    pub fn newest(&self) -> Option<&T> {
        self.entries.back().map(|e| &e.item)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// O ring do áudio: amostras pequenas e independentes (sem GOP), guardadas pelo tempo.
/// Acompanha o do vídeo: a história do áudio começa no mesmo instante que a do vídeo.
pub struct AudioRing<T> {
    keep_ms: i64,
    entries: VecDeque<(i64, T)>,
}

impl<T> AudioRing<T> {
    pub fn new(keep_ms: i64) -> Self {
        Self {
            keep_ms: keep_ms.max(0),
            entries: VecDeque::new(),
        }
    }

    pub fn push(&mut self, pts_ms: i64, item: T) {
        self.entries.push_back((pts_ms, item));
        while self
            .entries
            .front()
            .is_some_and(|(t, _)| pts_ms - t > self.keep_ms)
        {
            self.entries.pop_front();
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// O instante mais novo guardado (ms).
    pub fn newest_pts_ms(&self) -> Option<i64> {
        self.entries.back().map(|(t, _)| *t)
    }

    /// O primeiro e o último instante guardados (ms).
    pub fn bounds(&self) -> Option<(i64, i64)> {
        Some((self.entries.front()?.0, self.entries.back()?.0))
    }

    pub fn newest(&self) -> Option<&T> {
        self.entries.back().map(|(_, i)| i)
    }
}

impl<T: Clone> AudioRing<T> {
    /// As amostras de `from_ms` em diante (inclusive), em ordem.
    pub fn since(&self, from_ms: i64) -> Vec<T> {
        self.entries
            .iter()
            .filter(|(t, _)| *t >= from_ms)
            .map(|(_, i)| i.clone())
            .collect()
    }
}

impl<T: Clone> GopRing<T> {
    /// A história, do keyframe mais antigo ao mais novo. O ring continua como está.
    pub fn history(&self) -> Vec<T> {
        self.entries.iter().map(|e| e.item.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 30 fps, keyframe a cada `gop` quadros; devolve (pts_ms, key).
    fn frames(n: usize, gop: usize) -> impl Iterator<Item = (i64, bool)> {
        (0..n).map(move |i| (i as i64 * 33, i % gop == 0))
    }

    #[test]
    fn it_keeps_at_least_keep_ms_in_whole_gops() {
        let mut r = GopRing::new(5_000);
        for (i, (t, k)) in frames(600, 60).enumerate() {
            r.push(t, k, i); // 20 s de vídeo, GOP de 2 s
        }
        let h = r.history();
        // começa num keyframe (índice múltiplo de 60) e cobre pelo menos 5 s
        assert_eq!(h[0] % 60, 0, "a história começa num keyframe");
        assert!(r.span_ms() >= 5_000, "span {}", r.span_ms());
        assert!(
            r.span_ms() < 5_000 + 2_100,
            "no máximo keep + um GOP: {}",
            r.span_ms()
        );
        assert_eq!(*h.last().unwrap(), 599, "vai até o quadro mais novo");
    }

    #[test]
    fn what_comes_before_the_first_keyframe_is_dropped() {
        let mut r = GopRing::new(5_000);
        r.push(0, false, 0);
        r.push(33, false, 1);
        assert!(r.is_empty());
        r.push(66, true, 2);
        r.push(99, false, 3);
        assert_eq!(r.history(), [2, 3]);
    }

    #[test]
    fn a_gop_longer_than_the_cap_resets_the_ring_until_the_next_keyframe() {
        // 1 keyframe e depois só quadros P por 40 s (GOP "esticado" de câmera Smart Codec)
        let mut r = GopRing::new(5_000).with_max_span(30_000);
        r.push(0, true, 0);
        for i in 1..1_300 {
            r.push(i * 33, false, i);
        }
        assert!(r.is_empty(), "o GOP passou de 30 s: o ring foi esvaziado");
        // os P que chegam sem keyframe não reabrem o ring
        r.push(1_300 * 33, false, 1_300);
        assert!(r.is_empty());
        // o próximo keyframe reabre
        r.push(1_301 * 33, true, 1_301);
        r.push(1_302 * 33, false, 1_302);
        assert_eq!(r.history(), [1_301, 1_302]);
        // e dentro do teto o GOP longo continua inteiro
        let mut ok = GopRing::new(1_000).with_max_span(30_000);
        for (i, (t, k)) in frames(600, 600).enumerate() {
            ok.push(t, k, i); // ~20 s
        }
        assert_eq!(ok.len(), 600);
    }

    #[test]
    fn a_single_long_gop_is_never_cut() {
        // GOP maior que keep: guarda o GOP inteiro, não corta no meio
        let mut r = GopRing::new(1_000);
        for (i, (t, k)) in frames(300, 300).enumerate() {
            r.push(t, k, i);
        }
        assert_eq!(r.len(), 300);
        assert_eq!(r.history()[0], 0);
    }

    #[test]
    fn keep_zero_holds_just_the_current_gop() {
        let mut r = GopRing::new(0);
        for (i, (t, k)) in frames(200, 30).enumerate() {
            r.push(t, k, i);
        }
        let h = r.history();
        assert_eq!(h[0] % 30, 0);
        assert!(h.len() <= 30 * 2);
    }

    #[test]
    fn history_does_not_consume_and_clear_empties() {
        let mut r = GopRing::new(5_000);
        r.push(0, true, 1);
        r.push(33, false, 2);
        assert_eq!(r.history(), r.history());
        r.clear();
        assert!(r.is_empty() && r.span_ms() == 0);
    }

    #[test]
    fn the_oldest_pts_is_the_first_keyframe_kept() {
        let mut r = GopRing::new(1_000);
        assert_eq!(r.oldest_pts_ms(), None);
        for (i, (t, k)) in frames(300, 30).enumerate() {
            r.push(t, k, i);
        }
        let oldest = r.oldest_pts_ms().unwrap();
        assert_eq!(oldest % (30 * 33), 0, "cai num keyframe: {oldest}");
        assert!(r.span_ms() >= 1_000);
    }

    #[test]
    fn the_audio_ring_keeps_a_time_window_and_serves_from_an_instant() {
        let mut a = AudioRing::new(3_000);
        for i in 0..400 {
            a.push(i * 64, i); // 64 ms por amostra (AAC a 16 kHz, 1024 amostras)
        }
        // 400 × 64 ms = 25,6 s: só ~3 s ficam
        assert!(a.len() <= 3_000 / 64 + 2, "{}", a.len());
        let newest = 399 * 64;
        let since = a.since(newest - 1_000);
        assert!(since.len() >= 15 && since.len() <= 17, "{}", since.len());
        assert_eq!(*since.last().unwrap(), 399);
        assert!(a.since(newest + 1).is_empty());
        assert_eq!(a.newest(), Some(&399));
    }

    #[test]
    fn memory_stays_bounded_over_a_long_run() {
        let mut r = GopRing::new(5_000);
        for (i, (t, k)) in frames(100_000, 30).enumerate() {
            r.push(t, k, i);
        }
        // 5 s + 1 GOP a 30 fps ≈ até ~ 180 quadros, nunca cresce com o tempo
        assert!(r.len() < 200, "{}", r.len());
    }
}
