; Tauri NSIS hooks. ASCII only: the file is read with the ANSI code page.

; --- Making room for the install, the update and the uninstall ------------------------------------------------------
; The template checks that the program is not running with the Restart Manager, which also reports an antivirus or a
; firewall service that keeps the file open after scanning it (McAfee does, and cannot be asked to let go): the
; install, the silent update and the uninstall then stop with "closing Tachy Download failed". So the program is
; stopped here first, then the old executable is renamed out of the way (a rename works on a file that is open for
; scanning), and the template's check finds nothing. The renamed file is deleted at the end of the install or of
; the uninstall (at the next restart when something still has it open).
!macro TachyMakeRoom
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM "${MAINBINARYNAME}.exe"'
  Pop $0
  Sleep 500
  Delete "$INSTDIR\${MAINBINARYNAME}.exe.old"
  IfFileExists "$INSTDIR\${MAINBINARYNAME}.exe" 0 +2
    Rename "$INSTDIR\${MAINBINARYNAME}.exe" "$INSTDIR\${MAINBINARYNAME}.exe.old"
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro TachyMakeRoom
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro TachyMakeRoom
!macroend

; --- After the install ----------------------------------------------------------------------------------------------
; Same behaviour as the Electron installer's build/installer.nsh: at the end of the install, fetch yt-dlp and
; ffmpeg (SHA-256 verified). A failure (no network...) is ignored, Settings > Videos still has the install buttons.
!macro NSIS_HOOK_POSTINSTALL
  Delete /REBOOTOK "$INSTDIR\${MAINBINARYNAME}.exe.old"
  ; Remember the language the installer ran in (chosen from the system's, no selector is shown). The template only
  ; saves it when the selector is shown; without it the uninstaller asks "Installer Language" every time, and a silent
  ; uninstall fails on that question.
  WriteRegStr HKCU "${MANUPRODUCTKEY}" "Installer Language" $LANGUAGE
  DetailPrint "Video tools (yt-dlp, ffmpeg)..."
  ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --setup-tools' $0
  DetailPrint "Video tools: done (code $0)"
!macroend

; --- After the uninstall --------------------------------------------------------------------------------------------
; The template's "delete the application data" box only targets the folders named after the app identifier. Our
; downloads list, settings, logins and tools live in %APPDATA%\Tachy Download: when the box is ticked (and this is
; not an update), they go too. Without the box they stay, as with the Electron version.
!macro NSIS_HOOK_POSTUNINSTALL
  Delete /REBOOTOK "$INSTDIR\${MAINBINARYNAME}.exe.old"
  RMDir /REBOOTOK "$INSTDIR"
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    RmDir /r "$APPDATA\Tachy Download"
  ${EndIf}
!macroend
