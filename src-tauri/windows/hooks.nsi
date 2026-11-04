; Tauri NSIS hooks. Same behaviour as the Electron installer's build/installer.nsh:
; at the end of the install, fetch yt-dlp and ffmpeg (SHA-256 verified). A failure (no network...) is ignored,
; Settings > Videos still has the install buttons. ASCII only: the file is read with the ANSI code page.
!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Video tools (yt-dlp, ffmpeg)..."
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --setup-tools' $0
  DetailPrint "Video tools: done (code $0)"
!macroend
