#!/usr/bin/env bash
# Opens a small Linux desktop in a window (Xephyr, shown on Windows by WSLg) with the Linux build of
# Don't Forget running inside, so you can try it with your own mouse. Close the window to stop.
# From Windows:  wsl -d Ubuntu -u root -- bash /mnt/c/Users/BeForce1/personal/dont-forget/tools/linux-try.sh
set -u
here="$(cd "$(dirname "$0")" && pwd)"
bin=/root/dont-forget/target/release/DontForget
[ -x "$bin" ] || bash "$here/linux-build.sh"
D=:2

old=$(cat /tmp/.X2-lock 2>/dev/null | tr -d ' ')
[ -n "$old" ] && kill "$old" 2>/dev/null; sleep 0.3
rm -f /tmp/.X2-lock
Xephyr $D -screen 1280x800 -title "Don't Forget - Linux test desktop" -no-host-grab >/dev/null 2>&1 &
xephyr=$!
# Xephyr listens on an abstract socket here, so ask the server itself whether it is up
for _ in $(seq 1 50); do DISPLAY=$D xdpyinfo >/dev/null 2>&1 && break; sleep 0.2; done
export DISPLAY=$D
openbox >/dev/null 2>&1 &
sleep 0.5
xsetroot -solid '#2b2d42'

# things to try it on
convert -size 520x200 xc:'#fdf6e3' -fill '#073642' -pointsize 22 \
  -annotate +20+45 'Problem 4. Let A = {1,2,3,4,5,6}' \
  -annotate +20+90 'aRb iff 3 divides a - b.' \
  -annotate +20+135 '(c) How many relations contain R?' /tmp/problem.png
display -geometry +40+40 -title "Snip me (hold the wheel and drag)" /tmp/problem.png >/dev/null 2>&1 &
xterm -geometry 70x16+640+40 -T "Help" -e bash -c '
cat <<EOF
 Don'"'"'t Forget, Linux test desktop

 Hold the mouse wheel and drag      snip (left to right = green)
 Drag right to left                 add a blue note to the stack
 Quick wheel click                  drop the following notes
 Wheel click on a note              delete it
 Hold Shift                         freeze following notes
 Right click a note                 follow / pin
 Scroll on a note                   resize

 xeyes (bottom right) is a skipped app,
 "FakeGame" (bottom left) counts as a game.
 Close this whole window to stop.
EOF
exec bash' >/dev/null 2>&1 &
xeyes -geometry 260x180+980+560 >/dev/null 2>&1 &
mkdir -p /tmp/steamapps/common/FakeGame && cp "$(command -v xeyes)" /tmp/steamapps/common/FakeGame/fakegame
/tmp/steamapps/common/FakeGame/fakegame -geometry 260x180+40+560 -fg red >/dev/null 2>&1 &

mkdir -p /root/.config/dont-forget
[ -f /root/.config/dont-forget/settings.ini ] || printf 'ignored_apps=xeyes\nblock_games=1\n' > /root/.config/dont-forget/settings.ini
grep -q '^ignored_apps=.*xeyes' /root/.config/dont-forget/settings.ini || sed -i 's/^ignored_apps=.*/ignored_apps=xeyes/' /root/.config/dont-forget/settings.ini
"$bin" &

echo "Linux test desktop is open. Close its window to stop."
wait $xephyr
"$bin" --quit >/dev/null 2>&1
pkill -P $$ 2>/dev/null
exit 0
