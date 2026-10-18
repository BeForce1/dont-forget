#!/usr/bin/env bash
# End-to-end test of the Linux version on a virtual X screen (Xvfb + openbox), driven by xdotool.
# Usage: linux-test.sh <binary> <output-dir for screenshots>
set -u
bin=${1:-/root/dont-forget/target/release/DontForget}
out=${2:-/tmp/df-test}
mkdir -p "$out"
export DISPLAY=:99
cfg=/root/.config/dont-forget
mkdir -p "$cfg"
printf 'after_capture=follow\nhold_ms=250\nignored_apps=xeyes\nignored_sites=\nshown_intro=1\n' > "$cfg/settings.ini"

pkill -f "Xvfb :99" 2>/dev/null; sleep 0.3
Xvfb :99 -screen 0 1280x800x24 -nolisten tcp >/dev/null 2>&1 &
sleep 1
openbox >/dev/null 2>&1 &
sleep 0.5
: # (no xsetroot needed)

# something to snip: a "problem" image
convert -size 520x200 xc:'#fdf6e3' -fill '#073642' -pointsize 22 \
  -annotate +20+45 'Problem 4. Let A = {1,2,3,4,5,6}' \
  -annotate +20+90 'aRb iff 3 divides a - b.' \
  -annotate +20+135 '(c) How many relations contain R?' \
  -fill '#dc322f' -draw 'rectangle 20,160 500,180' /tmp/problem.png
display -geometry +60+60 -title problem /tmp/problem.png &
# a window that logs the mouse buttons it receives, to check quick clicks are passed on
xev -geometry 360x220+760+80 -name xev-target -event button > /tmp/xev.log 2>&1 &
# an "ignored app" (process name xeyes) where wheel-drags must pass straight through
xeyes -geometry 300x200+760+420 &
sleep 1.5

"$bin" > /tmp/df.log 2>&1 &
sleep 1
pass=0; fail=0
check() { if eval "$2"; then echo "PASS  $1"; pass=$((pass+1)); else echo "FAIL  $1"; fail=$((fail+1)); fi; }
notes() { xwininfo -root -tree | grep -c "Don't Forget note"; }
note_geom() { xwininfo -root -tree | grep "Don't Forget note" | head -1 | grep -oE '[0-9]+x[0-9]+\+-?[0-9]+\+-?[0-9]+' | head -1; }
shot() { import -window root "$out/$1.png"; }

check "app is running" "pgrep -x DontForget >/dev/null"
check "--status answers" "'$bin' --status | grep -q running"

# 1. wheel-drag left->right over the problem: a note appears and follows the mouse
xdotool mousemove 80 80 mousedown 2
for i in $(seq 1 20); do xdotool mousemove $((80 + i*24)) $((80 + i*9)); sleep 0.02; done
sleep 0.2; shot 1-selecting
xdotool mouseup 2; sleep 0.5
check "snip creates a note" "[ \$(notes) -eq 1 ]"
g1=$(note_geom)
xdotool mousemove 300 500; sleep 0.6; shot 2-following
g2=$(note_geom)
check "note follows the mouse ($g1 -> $g2)" "[ '$g1' != '$g2' ]"

# 2. quick wheel click anywhere drops (pins) the following note
xdotool click 2; sleep 0.3
g3=$(note_geom); xdotool mousemove 600 650; sleep 0.5; g4=$(note_geom)
check "wheel click pins it ($g3 stays $g4)" "[ '$g3' = '$g4' ]"
shot 3-pinned

# 3. a quick wheel click with nothing following reaches the app under the mouse
xdotool mousemove 900 180; sleep 0.2; : > /tmp/xev.log
xdotool click 2; sleep 0.5
check "quick wheel click reaches the app" "grep -q 'button 2' /tmp/xev.log"

# 4. ignored app: wheel-drag over xeyes makes no note
before=$(notes)
xdotool mousemove 850 500 mousedown 2; for i in $(seq 1 10); do xdotool mousemove $((850 + i*10)) $((500 + i*5)); sleep 0.02; done
sleep 0.4; xdotool mouseup 2; sleep 0.4
check "ignored app gets the drag, no note" "[ \$(notes) -eq $before ]"

# 5. right-to-left snip makes a blue note that joins the followers
xdotool mousemove 560 240 mousedown 2
for i in $(seq 1 12); do xdotool mousemove $((560 - i*20)) $((240 - i*8)); sleep 0.02; done
sleep 0.2; xdotool mouseup 2; sleep 0.4
xdotool mousemove 400 450; sleep 0.6; shot 4-blue-following
check "second note created" "[ \$(notes) -eq 2 ]"

# 6. wheel click drops it; wheel click on a note deletes it
xdotool click 2; sleep 0.3
geom=$(note_geom); x=$(echo "$geom" | sed -E 's/.*\+(-?[0-9]+)\+(-?[0-9]+)/\1/'); y=$(echo "$geom" | sed -E 's/.*\+(-?[0-9]+)\+(-?[0-9]+)/\2/')
xdotool mousemove $((x + 30)) $((y + 30)); sleep 0.3; shot 5-hover
xdotool click 2; sleep 0.4
check "wheel click on a note deletes it" "[ \$(notes) -eq 1 ]"

# 7. commands
"$bin" --clear >/dev/null; sleep 0.3
check "--clear removes all notes" "[ \$(notes) -eq 0 ]"
"$bin" --quit >/dev/null; sleep 0.5
check "--quit stops it" "! pgrep -x DontForget >/dev/null"

echo "--- $pass passed, $fail failed"
echo "--- app log:"; cat /tmp/df.log
pkill -f "Xvfb :99"
