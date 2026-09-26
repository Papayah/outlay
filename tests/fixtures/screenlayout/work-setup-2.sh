#!/bin/sh
xrandr --output DP-4 --mode 1920x1080 --pos 5120x360 --rotate normal \
	--output DP-2.2 --primary --mode 2560x1440 --rate 59.95 --pos 2560x0 --rotate normal \
	--output DP-2.1 --mode 2560x1440 --rate 59.95 --pos 0x0 --rotate normal
