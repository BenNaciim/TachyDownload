; Tauri NSIS hooks. Same behaviour as the Electron installer's build/installer.nsh:
; at the end of the install, fetch yt-dlp and ffmpeg (SHA-256 verified). A failure (no network...) is ignored,
; Settings > Videos still has the install buttons. ASCII only: the file is read with the ANSI code page.
!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Video tools (yt-dlp, ffmpeg)..."
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --setup-tools' $0
  DetailPrint "Video tools: done (code $0)"
!macroend

; The template's "delete the application data" box only targets the folders named after the app identifier. Our
; downloads list, settings, logins and tools live in %APPDATA%\Tachy Download: when the box is ticked (and this is
; not an update), they go too. Without the box they stay, as with the Electron version.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$APPDATA\Tachy Download"
  ${EndIf}
!macroend
