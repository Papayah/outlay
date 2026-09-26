#!/bin/sh
xrandr --output eDP-2 --mode 1920x1080 --rate 165.01 --pos 1920x1440 --rotate normal \
       --output DP-1-2 --primary --mode 2560x1440 --rate 143.91 --pos 1920x0 --rotate normal \
       --output HDMI-1-0 --mode 1920x1080 --pos 0x853 --rotate normal
