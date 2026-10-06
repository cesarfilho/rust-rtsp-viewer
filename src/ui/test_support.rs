//! Ajuda só dos testes: um bloqueio que os testes que **abrem um dispositivo gráfico** (wgpu) e os que
//! **tocam um arquivo de verdade** (GStreamer) disputam. Rodando juntos, sob carga, os dois grupos
//! travaram (o GStreamer carregando plugins e o wgpu iniciando o Vulkan/EGL no mesmo processo; uma
//! thread de streaming girou a 99% de CPU por 23 minutos). Um de cada vez, não trava.

use std::sync::{Mutex, MutexGuard};

static MEDIA_GPU: Mutex<()> = Mutex::new(());

/// Segure o resultado até o fim do teste.
pub fn media_gpu_lock() -> MutexGuard<'static, ()> {
    MEDIA_GPU.lock().unwrap_or_else(|e| e.into_inner())
}
