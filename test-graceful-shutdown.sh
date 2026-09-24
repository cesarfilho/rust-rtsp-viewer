#!/bin/bash
# Test graceful shutdown with Ctrl+C (SIGINT)

echo "🧪 Testing graceful shutdown..."
echo ""

# Start the application in background
cargo run &
APP_PID=$!

echo "Application started with PID: $APP_PID"
echo "Waiting 3 seconds for stream to start..."
sleep 3

echo ""
echo "📤 Sending SIGINT (Ctrl+C) signal..."
kill -SIGINT $APP_PID

# Wait for graceful shutdown (max 5 seconds)
echo "Waiting for graceful shutdown..."
for i in {1..5}; do
    if ! kill -0 $APP_PID 2>/dev/null; then
        echo ""
        echo "✅ SUCCESS: Application closed gracefully after ${i} second(s)"
        exit 0
    fi
    sleep 1
done

echo ""
echo "❌ FAILED: Application did not close within 5 seconds"
echo "Force killing..."
kill -9 $APP_PID 2>/dev/null
exit 1
