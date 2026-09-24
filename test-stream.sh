#!/bin/bash

# Script de teste de estabilidade do stream RTSP
# Testa o stream por um período e reporta problemas

echo "🎥 RTSP Stream Stability Test"
echo "═══════════════════════════════════════════════════════"
echo ""

# Verificar argumentos
if [ -z "$1" ]; then
    echo "❌ Uso: ./test-stream.sh <RTSP_URL> [DURATION_SECONDS]"
    echo ""
    echo "Exemplos:"
    echo "  ./test-stream.sh rtsp://admin:senha@192.168.15.88:554/stream"
    echo "  ./test-stream.sh rtsp://admin:senha@192.168.15.88:554/stream 30"
    echo ""
    exit 1
fi

RTSP_URL="$1"
DURATION=${2:-20}  # Default: 20 segundos

echo "📋 Configuração:"
echo "   URL: $RTSP_URL"
echo "   Duração: $DURATION segundos"
echo ""

# Verificar se URL é válida
if [[ ! "$RTSP_URL" =~ ^rtsp:// ]]; then
    echo "❌ Erro: URL deve começar com rtsp://"
    exit 1
fi

echo "🔍 Iniciando teste..."
echo "═══════════════════════════════════════════════════════"
echo ""

# Criar arquivo de log temporário
LOG_FILE=$(mktemp /tmp/rtsp-test-XXXXXX.log)

# Executar o viewer com logging por tempo limitado
echo "⏱️  Executando stream por $DURATION segundos..."
echo "   (output salvo em $LOG_FILE)"
echo ""

timeout "$DURATION" RUST_LOG=debug cargo run --release -- "$RTSP_URL" 2>&1 | tee "$LOG_FILE"

EXIT_CODE=$?

echo ""
echo "═══════════════════════════════════════════════════════"
echo "📊 Análise do Teste"
echo "═══════════════════════════════════════════════════════"
echo ""

# Verificar se timeout ocorreu (significa que rodou até o fim)
if [ $EXIT_CODE -eq 124 ]; then
    echo "✅ Stream rodou por $DURATION segundos sem travar"
    echo ""
elif [ $EXIT_CODE -eq 0 ]; then
    echo "✅ Stream encerrou normalmente"
    echo ""
else
    echo "⚠️  Stream encerrou com código: $EXIT_CODE"
    echo ""
fi

# Contar erros
ERROR_COUNT=$(grep -c "\[ERROR\]\|Error:" "$LOG_FILE" 2>/dev/null || echo "0")
WARNING_COUNT=$(grep -c "\[WARN\]\|Warning:" "$LOG_FILE" 2>/dev/null || echo "0")
RECONNECT_COUNT=$(grep -c "reconnect\|Reconnecting" "$LOG_FILE" 2>/dev/null || echo "0")
DROPPED_COUNT=$(grep -c "dropped\|drop" "$LOG_FILE" 2>/dev/null || echo "0")

echo "📈 Estatísticas:"
echo "   Erros:       $ERROR_COUNT"
echo "   Avisos:      $WARNING_COUNT"
echo "   Reconexões:  $RECONNECT_COUNT"
echo "   Frames drop: $DROPPED_COUNT"
echo ""

# Verificar problemas comuns
if [ "$ERROR_COUNT" -gt 0 ]; then
    echo "❌ Erros encontrados:"
    grep "\[ERROR\]\|Error:" "$LOG_FILE" | head -5
    echo ""
fi

if [ "$RECONNECT_COUNT" -gt 0 ]; then
    echo "⚠️  Reconexões detectadas (rede instável?)"
    echo ""
fi

if [ "$DROPPED_COUNT" -gt 10 ]; then
    echo "⚠️  Muitos frames sendo descartados"
    echo "   Sugestão: Aumentar cache para 5 segundos"
    echo ""
fi

# Mostrar pipeline usado
echo "🔧 Pipeline usado:"
grep "Pipeline:" "$LOG_FILE" | head -1
echo ""

# Mostrar configurações
echo "⚙️  Configurações detectadas:"
grep "Cache duration" "$LOG_FILE" | head -1
grep "Latency set" "$LOG_FILE" | head -1
echo ""

# Verificar se stream chegou a começar
if grep -q "Pipeline started, streaming" "$LOG_FILE"; then
    echo "✅ Stream iniciou com sucesso"
    
    # Calcular tempo até começar
    START_TIME=$(grep -a "Pipeline started" "$LOG_FILE" | head -1)
    echo "   $START_TIME"
    echo ""
else
    echo "❌ Stream não conseguiu iniciar"
    echo ""
fi

# Recomendações
echo "═══════════════════════════════════════════════════════"
echo "💡 Recomendações:"
echo "═══════════════════════════════════════════════════════"
echo ""

if [ "$ERROR_COUNT" -gt 0 ] || [ "$RECONNECT_COUNT" -gt 0 ]; then
    echo "1. Aumentar cache para 5 segundos:"
    echo "   make run URL=$RTSP_URL CACHE=5"
    echo ""
    echo "2. Aumentar latência para 300ms:"
    echo "   make run URL=$RTSP_URL CACHE=5 LATENCY=300"
    echo ""
    echo "3. Testar stream secundário (menor qualidade):"
    echo "   Substituir subtype=0 por subtype=1 na URL"
    echo ""
fi

if [ "$ERROR_COUNT" -eq 0 ] && [ "$RECONNECT_COUNT" -eq 0 ] && [ "$DROPPED_COUNT" -lt 5 ]; then
    echo "✅ Stream estável! Configuração atual está boa."
    echo ""
fi

echo "📁 Log completo salvo em: $LOG_FILE"
echo ""
echo "Para mais ajuda, veja: TROUBLESHOOTING.md"
echo ""

# Limpar log se tudo OK
if [ "$ERROR_COUNT" -eq 0 ] && [ "$RECONNECT_COUNT" -eq 0 ]; then
    read -p "Manter arquivo de log? (s/N): " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Ss]$ ]]; then
        rm "$LOG_FILE"
        echo "🗑️  Log removido"
    fi
fi
