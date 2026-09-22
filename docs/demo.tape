# Renders docs/demo.gif for the README, which is hosted on issue #6 to keep it
# out of git history: drag a new one into that issue and update the README
# link. Re-record from the repo root with:
# vhs docs/demo.tape (it builds and runs target/release/tuiman). Needs brew,
# neither lazygit nor yazi installed yet, and the default theme saved (the
# picker walks down three from it to catppuccin). The uninstall takes the last
# row of the installed list, so no installed TUI may have fewer stars than
# yazi.

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

# open the theme picker and walk down to catppuccin, previewing each theme
Type "t"
Sleep 1s
Down
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
Wait+Screen@3m /install lazygit.*finished/
Sleep 1.5s

# search and install yazi
Escape
Type "/yazi"
Sleep 0.7s
Enter
Sleep 1s
Enter
Sleep 1.5s
Enter
Wait+Screen@3m /install yazi.*finished/
Sleep 1.5s

# back to the top of the full list, then only installed TUIs and uninstall the last one, yazi
Escape
Sleep 0.7s
Type "gg"
Sleep 1s
Type "i"
Sleep 1.5s
Type "G"
Sleep 1s
Enter
Sleep 1.5s
Enter
Wait+Screen@3m /uninstall yazi.*finished/
Sleep 2s

Hide
Type "q"
