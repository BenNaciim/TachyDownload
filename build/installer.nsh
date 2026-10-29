; Runs at the end of the install: fetches yt-dlp and ffmpeg (SHA-256 verified).
; Failures (no network...) are ignored: Settings > Videos still has the install buttons.
!macro customInstall
  DetailPrint "Téléchargement des outils vidéo (yt-dlp, ffmpeg)…"
  ExecWait '"$INSTDIR\${APP_EXECUTABLE_FILENAME}" --setup-tools' $0
  DetailPrint "Outils vidéo : terminé (code $0)"
!macroend
