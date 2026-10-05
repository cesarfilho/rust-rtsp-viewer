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

pub struct GopRing<T> {
    keep_ms: i64,
    entries: VecDeque<Entry<T>>,
}

impl<T> GopRing<T> {
    pub fn new(keep_ms: i64) -> Self {
        Self {
            keep_ms: keep_ms.max(0),
            entries: VecDeque::new(),
        }
    }

    pub fn push(&mut self, pts_ms: i64, key: bool, item: T) {
        // Sem keyframe ainda não há por onde começar um clipe: o que vem antes do
        // primeiro keyframe é descartado.
        if self.entries.is_empty() && !key {
            return;
        }
        self.entries.push_back(Entry { pts_ms, key, item });
        self.trim();
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
    fn memory_stays_bounded_over_a_long_run() {
        let mut r = GopRing::new(5_000);
        for (i, (t, k)) in frames(100_000, 30).enumerate() {
            r.push(t, k, i);
        }
        // 5 s + 1 GOP a 30 fps ≈ até ~ 180 quadros, nunca cresce com o tempo
        assert!(r.len() < 200, "{}", r.len());
    }
}
