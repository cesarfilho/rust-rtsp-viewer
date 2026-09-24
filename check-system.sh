#!/bin/bash

# Script de verificação do sistema
# Verifica se todas as dependências necessárias estão instaladas

echo "🔍 Verificando dependências do RTSP Viewer..."
echo ""

# Verificar Rust e Cargo
echo "📦 Rust e Cargo:"
if command -v rustc &> /dev/null; then
    echo "   ✅ rustc: $(rustc --version)"
else
    echo "   ❌ rustc não encontrado"
    echo "      Instale em: https://rustup.rs/"
fi

if command -v cargo &> /dev/null; then
    echo "   ✅ cargo: $(cargo --version)"
else
    echo "   ❌ cargo não encontrado"
fi

echo ""

# Verificar pkg-config
echo "🔧 pkg-config:"
if command -v pkg-config &> /dev/null; then
    echo "   ✅ pkg-config: $(pkg-config --version)"
else
    echo "   ❌ pkg-config não encontrado"
    echo "      Ubuntu/Debian: sudo apt install pkg-config"
    echo "      Arch: sudo pacman -S pkg-config"
    echo "      macOS: brew install pkg-config"
fi

echo ""

# Verificar bibliotecas GStreamer
echo "🎥 GStreamer:"
REQUIRED_LIBS=(
    "gstreamer-1.0"
    "gstreamer-base-1.0"
    "gstreamer-video-1.0"
    "gstreamer-rtsp-1.0"
)

all_found=true
for lib in "${REQUIRED_LIBS[@]}"; do
    if pkg-config --exists "$lib" 2>/dev/null; then
        version=$(pkg-config --modversion "$lib")
        echo "   ✅ $lib: $version"
    else
        echo "   ❌ $lib não encontrado"
        all_found=false
    fi
done

echo ""

# Verificar plugins GStreamer
echo "🔌 Plugins GStreamer:"
if gst-inspect-1.0 --version &> /dev/null; then
    echo "   ✅ gst-inspect-1.0 disponível"
    
    # Verificar plugins específicos
    plugins=("rtspsrc" "decodebin" "autovideosink")
    for plugin in "${plugins[@]}"; do
        if gst-inspect-1.0 "$plugin" &> /dev/null; then
            echo "   ✅ $plugin"
        else
            echo "   ❌ $plugin não encontrado"
            all_found=false
        fi
    done
else
    echo "   ❌ gst-inspect-1.0 não encontrado"
    echo "      Ubuntu/Debian: sudo apt install gstreamer1.0-tools"
    echo "      Arch: sudo pacman -S gstreamer"
    echo "      macOS: brew install gstreamer"
    all_found=false
fi

echo ""
echo "═══════════════════════════════════════════════════════"
if [ "$all_found" = true ]; then
    echo "✅ Todas as dependências estão instaladas!"
    echo "   Você pode compilar o projeto com: cargo build"
else
    echo "❌ Algumas dependências estão faltando."
    echo "   Execute o script de instalação: ./install-deps.sh"
    echo "   Ou veja o README.md para instruções detalhadas."
fi
echo "═══════════════════════════════════════════════════════"
