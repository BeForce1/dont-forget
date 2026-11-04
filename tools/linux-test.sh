#!/usr/bin/env bash
# End-to-end test of the Linux version on a virtual X screen (Xvfb + openbox), driven by xdotool.
# Needs Xvfb, openbox, xdotool, xev, xeyes, xclip and ImageMagick.
# Usage: linux-test.sh <binary> <output-dir for screenshots>
set -u
bin=${1:-/root/dont-forget/target/release/DontForget}
out=${2:-/tmp/df-test}
mkdir -p "$out"
export DISPLAY=:99
cfg=/root/.config/dont-forget
mkdir -p "$cfg"
printf 'after_capture=follow\nhold_ms=250\nignored_apps=xeyes\nignored_sites=\nblock_games=1\nallowed_games=\nshown_intro=1\n' > "$cfg/settings.ini"
rm -rf "$cfg/notes"

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
# random noise compresses badly, so its PNG is big enough for a chunked (INCR) clipboard transfer
convert -size 420x300 xc: +noise Random /tmp/noise.png
display -geometry +60+420 -title noise /tmp/noise.png &
# a "game": any program running from a Steam library folder
mkdir -p /tmp/steamapps/common/FakeGame && cp "$(command -v xeyes)" /tmp/steamapps/common/FakeGame/fakegame
/tmp/steamapps/common/FakeGame/fakegame -geometry 200x120+1040+660 &
sleep 1.5

"$bin" > /tmp/df.log 2>&1 &
sleep 1
pass=0; fail=0
check() { if eval "$2"; then echo "PASS  $1"; pass=$((pass+1)); else echo "FAIL  $1"; fail=$((fail+1)); fi; }
notes() { xwininfo -root -tree | grep -c "Don't Forget note"; }
shown() { xdotool search --onlyvisible --name "Don't Forget note" 2>/dev/null | wc -l; }
all_geoms() { xwininfo -root -tree | grep "Don't Forget note" | grep -oE '[0-9]+x[0-9]+\+-?[0-9]+\+-?[0-9]+' | sort | tr '\n' ' '; }
note_xy() { geom=$(note_geom); x=$(echo "$geom" | sed -E 's/.*\+(-?[0-9]+)\+(-?[0-9]+)/\1/'); y=$(echo "$geom" | sed -E 's/.*\+(-?[0-9]+)\+(-?[0-9]+)/\2/'); }
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

# 8. games: a wheel-drag in a game goes to the game, unless that game is allowed
drag_game() { xdotool mousemove 1080 690 mousedown 2; for i in $(seq 1 10); do xdotool mousemove $((1080 + i*8)) $((690 + i*4)); sleep 0.02; done; sleep 0.4; xdotool mouseup 2; sleep 0.4; }
drag_game
check "game gets the drag, no note" "[ \$(notes) -eq 0 ]"
sed -i 's/^allowed_games=.*/allowed_games=fakegame/' "$cfg/settings.ini"
"$bin" --reload >/dev/null
drag_game
check "allowed game: the drag snips" "[ \$(notes) -eq 1 ]"
shot 6-allowed-game
"$bin" --clear >/dev/null; sleep 0.3

# 9. undo: --restore brings back what --clear removed; a wheel-click delete comes back too
"$bin" --restore >/dev/null; sleep 0.4
check "--restore brings back cleared notes" "[ \$(notes) -eq 1 ]"
note_xy; xdotool mousemove $((x + 20)) $((y + 20)); sleep 0.2; xdotool click 2; sleep 0.4
check "wheel click deletes it again" "[ \$(notes) -eq 0 ]"
xdotool key ctrl+alt+z; sleep 0.4
check "Ctrl+Alt+Z restores it" "[ \$(notes) -eq 1 ]"
"$bin" --clear >/dev/null; sleep 0.3

# 10. keyboard snip: Ctrl+Alt+S, then a left-button drag
xdotool mousemove 80 80; xdotool key ctrl+alt+s; sleep 0.4
xdotool mousedown 1; for i in $(seq 1 15); do xdotool mousemove $((80 + i*20)) $((80 + i*8)); sleep 0.02; done
sleep 0.2; shot 7-key-selecting; xdotool mouseup 1; sleep 0.5
check "Ctrl+Alt+S + left drag snips" "[ \$(notes) -eq 1 ]"
xdotool click 2; sleep 0.3
xdotool key ctrl+alt+s; sleep 0.3; xdotool key Escape; sleep 0.3
xdotool mousedown 1; xdotool mousemove 500 500; xdotool mouseup 1; sleep 0.3
check "Esc cancels a keyboard snip" "[ \$(notes) -eq 1 ]"

# 11. clipboard: the snip is there as image/png
check "clipboard offers image/png" "xclip -selection clipboard -t TARGETS -o | grep -q image/png"
xclip -selection clipboard -t image/png -o > /tmp/clip.png
check "clipboard PNG is the snip ($(identify -format %wx%h /tmp/clip.png 2>/dev/null))" "[ \"\$(identify -format %wx%h /tmp/clip.png)\" = 300x120 ]"
# a big, noisy snip goes out in pieces (INCR)
xdotool mousemove 70 430 mousedown 2; for i in $(seq 1 20); do xdotool mousemove $((70 + i*20)) $((430 + i*14)); sleep 0.02; done
sleep 0.2; xdotool mouseup 2; sleep 0.5; xdotool click 2; sleep 0.3
xclip -selection clipboard -t image/png -o > /tmp/clip2.png
size=$(stat -c %s /tmp/clip2.png)
check "big snip copied in pieces ($size bytes, $(identify -format %wx%h /tmp/clip2.png 2>/dev/null))" "[ $size -gt 262144 ] && [ \"\$(identify -format %wx%h /tmp/clip2.png)\" = 400x280 ]"

# 12. hide and show
xdotool key ctrl+alt+h; sleep 0.3
check "Ctrl+Alt+H hides all notes" "[ \$(shown) -eq 0 ] && [ \$(notes) -eq 2 ]"
"$bin" --hide >/dev/null; sleep 0.3
check "--hide shows them again" "[ \$(shown) -eq 2 ]"

# 13. look: dashed, thick frames apply on --reload
printf 'outline=dashed\noutline_width=4\nsnip_color=#FF4FA3\n' >> "$cfg/settings.ini"
"$bin" --reload >/dev/null; sleep 0.4
xdotool mousemove 1100 100; sleep 0.2
shot 8-dashed

# 14. kept notes: quit, start again, the notes and the undo list come back
before=$(all_geoms)
"$bin" --quit >/dev/null; sleep 0.5
check "--quit stops it" "! pgrep -x DontForget >/dev/null"
check "notes saved as PNGs" "[ \$(ls $cfg/notes/*.png | wc -l) -ge 2 ] && grep -q '^note=' $cfg/notes/notes.ini"
"$bin" > /tmp/df2.log 2>&1 &
sleep 1
after=$(all_geoms)
check "notes come back where they were ($after)" "[ '$before' = '$after' ]"
check "undo list kept" "'$bin' --status | grep -qE '[1-9][0-9]* deleted'"
shot 9-restored

# 15. a note saved off-screen moves back on
"$bin" --quit >/dev/null; sleep 0.5
sed -i -E 's/^note=([0-9]+),-?[0-9]+,-?[0-9]+,/note=\1,5000,4000,/' $cfg/notes/notes.ini
"$bin" > /tmp/df3.log 2>&1 &
sleep 1
check "off-screen notes move back on ($(all_geoms))" "! all_geoms | grep -qE '\+[0-9]{4}\+[0-9]{4}'"

# 16. keep_notes=0: the notes folder is emptied on quit
sed -i '/^keep_notes=/d' "$cfg/settings.ini"; echo keep_notes=0 >> "$cfg/settings.ini"
"$bin" --reload >/dev/null; "$bin" --quit >/dev/null; sleep 0.5
check "keep_notes=0 wipes saved notes" "[ ! -d $cfg/notes ]"

echo "--- $pass passed, $fail failed"
echo "--- app log:"; cat /tmp/df.log
pkill -f "Xvfb :99"
