#!/usr/bin/env bash
# Test RTSP connectivity using GStreamer
# Usage: ./test_cam.sh <rtsp_url>
# Example: ./test_cam.sh "rtsp://admin:senha@192.168.1.10:554/cam/realmonitor?channel=1&subtype=0"

if [[ -z "$1" ]]; then
  echo "Usage: $0 <rtsp_url>"
  exit 1
fi

RTSP_URL="$1"

gst-launch-1.0 -v rtspsrc location="$RTSP_URL" latency=100 ! fakesink sync=false -e