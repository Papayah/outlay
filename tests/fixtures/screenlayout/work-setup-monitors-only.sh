#!/bin/sh
xrandr --dpi 96 --output eDP-1 --off \
	--output DP-1-2.1 --primary --mode 2560x1440 --rate 59.95 --pos 2560x0 --rotate normal \
	--output DP-1-2.2 --mode 2560x1440 --rate 59.95 --pos 0x0 --rotate normal
