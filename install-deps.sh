#!/bin/bash

# Script de instalação das dependências do RTSP Viewer
# Este script instala todas as dependências necessárias do sistema

echo "Instalando dependências do GStreamer..."

# Detectar o sistema operacional
if [ -f /etc/debian_version ]; then
    # Debian/Ubuntu
    echo "Sistema Debian/Ubuntu detectado"
    sudo apt update
    sudo apt install -y \
        pkg-config \
        libgstreamer1.0-dev \
        libgstreamer-plugins-base1.0-dev \
        gstreamer1.0-tools \
        gstreamer1.0-plugins-base \
        gstreamer1.0-plugins-good \
        gstreamer1.0-plugins-bad \
        gstreamer1.0-rtsp \
        libssl-dev
    
elif [ -f /etc/arch-release ]; then
    # Arch Linux
    echo "Sistema Arch Linux detectado"
    sudo pacman -Sy --noconfirm \
        pkg-config \
        gstreamer \
        gst-plugins-base \
        gst-plugins-good \
        gst-plugins-bad \
        gst-rtsp \
        openssl
    
elif [ "$(uname)" == "Darwin" ]; then
    # macOS
    echo "macOS detectado"
    if ! command -v brew &> /dev/null; then
        echo "Homebrew não encontrado. Instale primeiro: https://brew.sh/"
        exit 1
    fi
    brew install \
        pkg-config \
        gstreamer \
        gst-plugins-base \
        gst-plugins-good \
        gst-plugins-bad \
        gst-rtsp \
        openssl
else
    echo "Sistema não reconhecido. Por favor, instale manualmente as dependências."
    echo "Veja o README.md para instruções."
    exit 1
fi

echo ""
echo "Instalação concluída!"
echo "Agora você pode compilar o projeto com: cargo build"
