#!/bin/sh
xrandr --output eDP-1 --off --output DP-1 --off \
  --output DP-1-2 --primary --mode 1920x1080 --rate 119.88 --pos 1920x0 --rotate normal \
  --output DP-3 --off \
  --output HDMI-1-0 --mode 1920x1080 --pos 0x0 --rotate normal \
  --output DP-4 --off \
  --output eDP-1 --off
