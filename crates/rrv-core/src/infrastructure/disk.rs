//! Uso do disco onde ficam as gravações (para a retenção, plano 3.3).

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use crate::domain::retention::DiskUsage;

/// Total e usado do sistema de arquivos de `path`. `None` se não der para ler.
/// "Usado" é `total - disponível para não-root`, como o `df` mostra.
pub fn usage(path: &Path) -> Option<DiskUsage> {
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c` é uma string C válida e `st` é preenchida pela chamada quando devolve 0.
    let st = unsafe {
        if libc::statvfs(c.as_ptr(), st.as_mut_ptr()) != 0 {
            return None;
        }
        st.assume_init()
    };
    let frag = st.f_frsize as u64;
    let total = st.f_blocks as u64 * frag;
    let avail = st.f_bavail as u64 * frag;
    Some(DiskUsage {
        total,
        used: total.saturating_sub(avail),
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_temp_dir_has_a_sane_usage() {
        let u = super::usage(&std::env::temp_dir()).expect("statvfs");
        assert!(u.total > 0 && u.used <= u.total);
    }

    #[test]
    fn a_missing_path_is_none() {
        assert!(super::usage(std::path::Path::new("/não/existe/mesmo")).is_none());
    }
}
