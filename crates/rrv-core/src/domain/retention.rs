//! Retenção das gravações (plano 3.3): o que apagar, em função da idade e do espaço.
//! Puro: recebe os candidatos e o uso do disco e devolve os ids; quem apaga é o
//! `infrastructure::store`.
//!
//! Padrões (spec `ux-historico.md`, D-H2): gravações por movimento ficam 7 dias; as
//! manuais/contínuas não expiram por idade; e o disco nunca passa de 80%.

use serde::Deserialize;

const DAY_MS: i64 = 86_400_000;

/// Espelho TOML de `[retention]`.
#[derive(Debug, Deserialize, Clone, Default)]
pub struct RetentionFile {
    /// Dias que as gravações por movimento ficam (0 = não expiram por idade). Padrão: 7.
    pub motion_days: Option<u32>,
    /// Dias das gravações manuais e contínuas (0 = não expiram por idade). Padrão: 0.
    pub manual_days: Option<u32>,
    /// Uso máximo do disco, em % (50–95). Padrão: 80.
    pub max_disk_percent: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetentionConfig {
    pub motion_days: u32,
    pub manual_days: u32,
    pub max_disk_percent: u8,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            motion_days: 7,
            manual_days: 0,
            max_disk_percent: 80,
        }
    }
}

impl RetentionFile {
    pub fn into_config(self) -> Result<RetentionConfig, String> {
        let d = RetentionConfig::default();
        let cfg = RetentionConfig {
            motion_days: self.motion_days.unwrap_or(d.motion_days),
            manual_days: self.manual_days.unwrap_or(d.manual_days),
            max_disk_percent: self.max_disk_percent.unwrap_or(d.max_disk_percent),
        };
        if !(50..=95).contains(&cfg.max_disk_percent) {
            return Err(format!(
                "max_disk_percent = {} fora de 50–95",
                cfg.max_disk_percent
            ));
        }
        Ok(cfg)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskUsage {
    pub total: u64,
    pub used: u64,
}

/// Abaixo disto de espaço livre o disco está "quase cheio" (spec `ux-historico.md`).
pub const DISK_LOW_FREE_PERCENT: u64 = 10;
/// E só volta ao normal quando passa disto (histerese: sem avisar a cada oscilação).
pub const DISK_RECOVERED_FREE_PERCENT: u64 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskTransition {
    /// Passou a "quase cheio": avisar uma vez.
    BecameLow { free_percent: u64 },
    /// Voltou ao normal.
    Recovered { free_percent: u64 },
}

/// Observa o disco e diz **quando muda** de estado, para o aviso sair uma vez só.
#[derive(Debug, Default)]
pub struct DiskWatch {
    low: bool,
}

impl DiskWatch {
    pub fn update(&mut self, usage: DiskUsage) -> Option<DiskTransition> {
        if usage.total == 0 {
            return None;
        }
        let free = usage.total.saturating_sub(usage.used) * 100 / usage.total;
        if !self.low && free < DISK_LOW_FREE_PERCENT {
            self.low = true;
            Some(DiskTransition::BecameLow { free_percent: free })
        } else if self.low && free >= DISK_RECOVERED_FREE_PERCENT {
            self.low = false;
            Some(DiskTransition::Recovered { free_percent: free })
        } else {
            None
        }
    }
}

/// Um segmento que a retenção pode apagar (fechado e não protegido).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: i64,
    pub ts_start: i64,
    pub bytes: u64,
    /// `"motion"` ou `"manual"`.
    pub mode: String,
}

/// Os ids a apagar. `candidates` deve vir **do mais antigo ao mais novo**.
/// 1. Idade: passou dos dias do seu modo.
/// 2. Espaço: se o disco ainda passa do limite, os mais antigos que restam, até caber.
pub fn plan(
    candidates: &[Candidate],
    now_ms: i64,
    cfg: &RetentionConfig,
    disk: Option<DiskUsage>,
) -> Vec<i64> {
    let mut picked = vec![false; candidates.len()];
    let mut freed: u64 = 0;
    for (i, c) in candidates.iter().enumerate() {
        let days = if c.mode == "motion" {
            cfg.motion_days
        } else {
            cfg.manual_days
        };
        if days > 0 && c.ts_start < now_ms - days as i64 * DAY_MS {
            picked[i] = true;
            freed += c.bytes;
        }
    }
    if let Some(d) = disk {
        let target = d.total / 100 * cfg.max_disk_percent as u64;
        let mut over = d.used.saturating_sub(freed).saturating_sub(target);
        for (i, c) in candidates.iter().enumerate() {
            if over == 0 {
                break;
            }
            if !picked[i] {
                picked[i] = true;
                over = over.saturating_sub(c.bytes);
            }
        }
    }
    candidates
        .iter()
        .zip(picked)
        .filter_map(|(c, p)| p.then_some(c.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 100 * DAY_MS;

    fn seg(id: i64, days_ago: i64, bytes: u64, mode: &str) -> Candidate {
        Candidate {
            id,
            ts_start: NOW - days_ago * DAY_MS,
            bytes,
            mode: mode.into(),
        }
    }

    #[test]
    fn motion_expires_by_age_and_manual_does_not_by_default() {
        let c = [
            seg(1, 10, 1, "motion"),
            seg(2, 10, 1, "manual"),
            seg(3, 3, 1, "motion"),
        ];
        assert_eq!(plan(&c, NOW, &RetentionConfig::default(), None), [1]);
    }

    #[test]
    fn manual_can_have_its_own_days() {
        let cfg = RetentionConfig {
            manual_days: 30,
            ..RetentionConfig::default()
        };
        let c = [seg(1, 40, 1, "manual"), seg(2, 20, 1, "manual")];
        assert_eq!(plan(&c, NOW, &cfg, None), [1]);
    }

    #[test]
    fn zero_days_means_never_by_age() {
        let cfg = RetentionConfig {
            motion_days: 0,
            ..RetentionConfig::default()
        };
        assert!(plan(&[seg(1, 999, 1, "motion")], NOW, &cfg, None).is_empty());
    }

    #[test]
    fn a_full_disk_deletes_the_oldest_until_it_fits() {
        // 1000 total, limite 80% = 800; usado 900 → precisa liberar 100.
        let disk = Some(DiskUsage {
            total: 1000,
            used: 900,
        });
        let c = [
            seg(1, 5, 60, "manual"),
            seg(2, 4, 60, "manual"),
            seg(3, 3, 60, "manual"),
        ];
        assert_eq!(plan(&c, NOW, &RetentionConfig::default(), disk), [1, 2]);
    }

    #[test]
    fn what_age_already_frees_counts_toward_the_space_limit() {
        let disk = Some(DiskUsage {
            total: 1000,
            used: 900,
        });
        // o de 10 dias (150 bytes) sai pela idade e já resolve; o resto fica.
        let c = [seg(1, 10, 150, "motion"), seg(2, 1, 60, "manual")];
        assert_eq!(plan(&c, NOW, &RetentionConfig::default(), disk), [1]);
    }

    #[test]
    fn under_the_limit_nothing_goes() {
        let disk = Some(DiskUsage {
            total: 1000,
            used: 500,
        });
        assert!(
            plan(
                &[seg(1, 1, 100, "manual")],
                NOW,
                &RetentionConfig::default(),
                disk
            )
            .is_empty()
        );
    }

    fn usage(free_percent: u64) -> DiskUsage {
        DiskUsage {
            total: 1000,
            used: 1000 - free_percent * 10,
        }
    }

    #[test]
    fn the_disk_warning_fires_once_and_has_hysteresis() {
        let mut w = DiskWatch::default();
        assert_eq!(w.update(usage(50)), None);
        assert_eq!(w.update(usage(12)), None, "ainda acima de 10%");
        assert_eq!(
            w.update(usage(9)),
            Some(DiskTransition::BecameLow { free_percent: 9 })
        );
        assert_eq!(w.update(usage(8)), None, "já avisou: não repete");
        assert_eq!(w.update(usage(12)), None, "entre 10% e 15% continua baixo");
        assert_eq!(
            w.update(usage(16)),
            Some(DiskTransition::Recovered { free_percent: 16 })
        );
        // pode avisar de novo numa próxima queda
        assert!(matches!(
            w.update(usage(5)),
            Some(DiskTransition::BecameLow { .. })
        ));
    }

    #[test]
    fn an_unreadable_disk_never_warns() {
        let mut w = DiskWatch::default();
        assert_eq!(w.update(DiskUsage { total: 0, used: 0 }), None);
    }

    #[test]
    fn the_percentage_is_validated() {
        let bad = RetentionFile {
            max_disk_percent: Some(99),
            ..Default::default()
        };
        assert!(bad.into_config().is_err());
        assert_eq!(
            RetentionFile::default().into_config().unwrap(),
            RetentionConfig::default()
        );
    }
}
