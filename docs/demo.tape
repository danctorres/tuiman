# Renders docs/demo.gif for the README, which is hosted on issue #6 to keep it
# out of git history: drag a new one into that issue and update the README
# link. Re-record from the repo root with:
# vhs docs/demo.tape (it builds and runs target/release/tuiman). Needs brew,
# lazygit not installed yet, btop installed, and a theme other
# than dracula saved.

Output docs/demo.gif

Set Shell bash
Set FontSize 16
Set Width 1200
Set Height 700
Set Padding 10
Set TypingSpeed 200ms

Hide
Type "cargo build --release -q && clear && target/release/tuiman" Enter
Wait+Screen@5m /NAME/
Sleep 1s
Show

Sleep 1.5s

# open the theme picker, search for dracula (the match previews), close the
# search and pick it
Type "t"
Sleep 1s
Type "/dra"
Sleep 1s
Enter
Sleep 0.7s
Enter
Sleep 1.5s

# search and install lazygit: Enter shows the exact command first, then runs it
Type "/lazy"
Sleep 0.7s
Enter
Sleep 1s
Enter
Sleep 1.5s
Enter
Wait+Screen@3m / install lazygit.*finished/
Sleep 1.5s

# only installed TUIs, then search for btop, uninstall it and clear all filters
Escape
Sleep 0.7s
Type "i"
Sleep 1.5s
Type "/btop"
Sleep 0.7s
Enter
Sleep 1s
Enter
Sleep 1.5s
Enter
Wait+Screen@3m /uninstall btop.*finished/
Sleep 1.5s
Type "c"
Sleep 2s

Hide
Type "q"
