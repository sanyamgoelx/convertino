; Convertino's Windows installer look: one dark window, no wizard.
;
;   Welcome   logo, name, [Install], install location (Change)
;   Progress  logo, "Installing Convertino…", thin blue bar
;   Done      logo, "Convertino is ready", [Open Convertino] [Close],
;             desktop shortcut option
;
; Included by installer.nsi (Tauri's NSIS template with the pages replaced).
; Everything is drawn with ordinary Win32 controls: the classic frame
; (header, lines, Back/Next/Cancel) is hidden, the page area fills the
; window, and the title bar is dark (Windows 10 2004+/11; older ones just
; keep their usual title bar). Updates (/P) only show the progress page.
;
; The logo bitmaps (logo-<scale>.bmp) come from make-images.py.

!include nsDialogs.nsh
!include LogicLib.nsh
!include WinMessages.nsh

!define CV_ASSETS "${__FILEDIR__}"

; Colours (RRGGBB for SetCtlColors, BBGGRR where Windows wants a COLORREF)
!define CV_BG          0x14171D
!define CV_BG_REF      0x1D1714
!define CV_TEXT        0xF2F4F8
!define CV_TEXT_REF    0xF8F4F2
!define CV_DIM         0x98A1B0
!define CV_ACCENT      0x0078D4
!define CV_ACCENT_REF  0xD47800
!define CV_ACCENT_TEXT 0x4CA3F0
!define CV_TRACK_REF   0x3A2F2A
!define CV_SECONDARY   0x2A2F3A
!define CV_WHITE       0xFFFFFF

; Window client size at 100 % (pixels)
!define CV_W 440
!define CV_H 440

Var cvDpi
Var cvW
Var cvH
Var cvT1
Var cvT2
Var cvT3
Var cvT4
Var cvTs
Var cvFontTitle
Var cvFontBody
Var cvFontSmall
Var cvFontButton
Var cvPage
Var cvDirLabel
Var cvShortcutBox
Var cvOpenApp
Var cvCompare ; "" new install, 0 same version, 1 upgrade, -1 downgrade

; ${_out} = ${_v} logical pixels at this window's DPI
!macro CvPx _out _v
  IntOp ${_out} ${_v} * $cvDpi
  IntOp ${_out} ${_out} / 96
!macroend

; ---------------------------------------------------------------- frame

Function CvHideChrome
  ; Hide everything in the outer window except the page dialogs (#32770):
  ; header, lines, branding text and the Back/Next/Cancel buttons. NSIS shows
  ; some buttons again on every page change, so each page calls this.
  Push $0
  Push $1
  StrCpy $0 0
  cv_hide_loop:
    FindWindow $0 "" "" $HWNDPARENT $0
    StrCmp $0 0 cv_hide_done
    System::Call 'user32::GetClassNameW(p r0, w .r1, i 64)'
    StrCmp $1 "#32770" cv_hide_loop
    ShowWindow $0 ${SW_HIDE}
    Goto cv_hide_loop
  cv_hide_done:
  Pop $1
  Pop $0
FunctionEnd

Function CvGuiInit
  Push $0
  Push $1
  Push $2
  Push $3
  Push $4
  Push $5
  Push $6
  Push $7

  System::Call 'user32::GetDpiForWindow(p $HWNDPARENT) i .r0'
  ${If} $0 < 96
    StrCpy $0 96
  ${EndIf}
  StrCpy $cvDpi $0
  !insertmacro CvPx $cvW ${CV_W}
  !insertmacro CvPx $cvH ${CV_H}

  ; Dark title bar in the window's own colour (attributes 20, 35, 36, 34)
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 20, *i 1, i 4)'
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 35, *i ${CV_BG_REF}, i 4)'
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 36, *i ${CV_TEXT_REF}, i 4)'
  System::Call 'dwmapi::DwmSetWindowAttribute(p $HWNDPARENT, i 34, *i ${CV_BG_REF}, i 4)'
  SetCtlColors $HWNDPARENT ${CV_TEXT} ${CV_BG}
  Call CvHideChrome

  ; Resize to the chosen client size, centred on the work area
  System::Call '*(i, i, i, i) p .r1'
  System::Call 'user32::GetWindowRect(p $HWNDPARENT, p r1)'
  System::Call '*$1(i .r2, i .r3, i .r4, i .r5)'
  IntOp $6 $4 - $2
  IntOp $7 $5 - $3
  System::Call 'user32::GetClientRect(p $HWNDPARENT, p r1)'
  System::Call '*$1(i, i, i .r4, i .r5)'
  IntOp $6 $6 - $4
  IntOp $7 $7 - $5
  IntOp $6 $6 + $cvW
  IntOp $7 $7 + $cvH
  System::Call 'user32::SystemParametersInfoW(i 0x30, i 0, p r1, i 0)'
  System::Call '*$1(i .r2, i .r3, i .r4, i .r5)'
  IntOp $4 $4 - $2
  IntOp $4 $4 - $6
  IntOp $4 $4 / 2
  IntOp $2 $2 + $4
  IntOp $5 $5 - $3
  IntOp $5 $5 - $7
  IntOp $5 $5 / 2
  IntOp $3 $3 + $5
  System::Free $1
  System::Call 'user32::SetWindowPos(p $HWNDPARENT, p 0, i r2, i r3, i r6, i r7, i 0x14)'

  ; Pages fill the whole window
  GetDlgItem $0 $HWNDPARENT 1018
  System::Call 'user32::SetWindowPos(p r0, p 0, i 0, i 0, i $cvW, i $cvH, i 0x14)'

  CreateFont $cvFontTitle "Segoe UI" 20 600
  CreateFont $cvFontBody "Segoe UI" 10 400
  CreateFont $cvFontSmall "Segoe UI" 9 400
  CreateFont $cvFontButton "Segoe UI" 10 600

  ; Logo for this display scale
  InitPluginsDir
  ${If} $cvDpi <= 108
    File "/oname=$PLUGINSDIR\cv-logo.bmp" "${CV_ASSETS}\logo-100.bmp"
  ${ElseIf} $cvDpi <= 132
    File "/oname=$PLUGINSDIR\cv-logo.bmp" "${CV_ASSETS}\logo-125.bmp"
  ${ElseIf} $cvDpi <= 156
    File "/oname=$PLUGINSDIR\cv-logo.bmp" "${CV_ASSETS}\logo-150.bmp"
  ${ElseIf} $cvDpi <= 180
    File "/oname=$PLUGINSDIR\cv-logo.bmp" "${CV_ASSETS}\logo-175.bmp"
  ${ElseIf} $cvDpi <= 216
    File "/oname=$PLUGINSDIR\cv-logo.bmp" "${CV_ASSETS}\logo-200.bmp"
  ${ElseIf} $cvDpi <= 264
    File "/oname=$PLUGINSDIR\cv-logo.bmp" "${CV_ASSETS}\logo-250.bmp"
  ${Else}
    File "/oname=$PLUGINSDIR\cv-logo.bmp" "${CV_ASSETS}\logo-300.bmp"
  ${EndIf}

  Pop $7
  Pop $6
  Pop $5
  Pop $4
  Pop $3
  Pop $2
  Pop $1
  Pop $0
FunctionEnd

; ---------------------------------------------------------------- controls
; These create plain Win32 controls in a parent window (an nsDialogs page or
; the progress page), so both kinds of page share one look.

; Logo, centred, top at logical y
!macro CvLogo _parent _y
  Push $0
  Push $1
  Push $2
  !insertmacro CvPx $cvT1 88
  !insertmacro CvPx $cvT2 ${_y}
  IntOp $cvT3 $cvW - $cvT1
  IntOp $cvT3 $cvT3 / 2
  ; WS_CHILD|WS_VISIBLE|SS_BITMAP
  !insertmacro CvControl $0 ${_parent} STATIC "" 0x5000000E $cvT3 $cvT2 $cvT1 $cvT1
  System::Call 'user32::LoadImageW(p 0, w "$PLUGINSDIR\cv-logo.bmp", i 0, i 0, i 0, i 0x10) p .r1'
  SendMessage $0 0x172 0 $1 ; STM_SETIMAGE
  Pop $2
  Pop $1
  Pop $0
!macroend

; Text across the whole width, centred. _style adds e.g. 0x200 (one line,
; vertically centred) or 0x8000 (path ellipsis).
!macro CvText _out _parent _text _y _h _font _color _style
  !insertmacro CvPx $cvT2 ${_y}
  !insertmacro CvPx $cvT4 ${_h}
  !insertmacro CvPx $cvT1 32
  IntOp $cvT3 $cvW - $cvT1
  IntOp $cvT3 $cvT3 - $cvT1
  ; WS_CHILD|WS_VISIBLE|SS_CENTER|SS_NOTIFY
  IntOp $cvTs ${_style} | 0x50000101
  !insertmacro CvControl ${_out} ${_parent} STATIC "${_text}" $cvTs $cvT1 $cvT2 $cvT3 $cvT4
  SendMessage ${_out} ${WM_SETFONT} ${_font} 1
  SetCtlColors ${_out} ${_color} ${CV_BG}
!macroend

; A child control. On nsDialogs pages it goes through nsDialogs, so clicks
; reach ${NSD_OnClick}; on the progress page it's created directly.
Var cvOnDialogs
!macro CvControl _out _parent _class _text _style _x _y _w _h
  ${If} $cvOnDialogs = 1
    nsDialogs::CreateControl ${_class} ${_style} 0 ${_x} ${_y} ${_w} ${_h} "${_text}"
  ${Else}
    System::Call 'user32::CreateWindowExW(i 0, w "${_class}", w "${_text}", i ${_style}, i ${_x}, i ${_y}, i ${_w}, i ${_h}, p ${_parent}, p 0, p 0, p 0) p .s'
  ${EndIf}
  Pop ${_out}
!macroend

; A flat, rounded button (a clickable label); _x = -1 centres it
!macro CvButton _out _parent _text _x _y _w _h _bg _fg
  !insertmacro CvPx $cvT1 ${_w}
  !insertmacro CvPx $cvT2 ${_y}
  !insertmacro CvPx $cvT4 ${_h}
  !if ${_x} == -1
    IntOp $cvT3 $cvW - $cvT1
    IntOp $cvT3 $cvT3 / 2
  !else
    !insertmacro CvPx $cvT3 ${_x}
  !endif
  ; WS_CHILD|WS_VISIBLE|WS_TABSTOP|SS_CENTER|SS_CENTERIMAGE|SS_NOTIFY
  !insertmacro CvControl ${_out} ${_parent} STATIC "${_text}" 0x50010301 $cvT3 $cvT2 $cvT1 $cvT4
  SendMessage ${_out} ${WM_SETFONT} $cvFontButton 1
  SetCtlColors ${_out} ${_fg} ${_bg}
  Push $0
  !insertmacro CvPx $0 8
  IntOp $cvT1 $cvT1 + 1
  IntOp $cvT4 $cvT4 + 1
  System::Call 'gdi32::CreateRoundRectRgn(i 0, i 0, i $cvT1, i $cvT4, i r0, i r0) p .r0'
  System::Call 'user32::SetWindowRgn(p ${_out}, p r0, i 1)'
  Pop $0
!macroend

; nsDialogs page with the window's colours
!macro CvPage
  nsDialogs::Create 1018
  Pop $cvPage
  ${If} $cvPage == error
    Abort
  ${EndIf}
  SetCtlColors $cvPage ${CV_TEXT} ${CV_BG}
  StrCpy $cvOnDialogs 1
  Call CvHideChrome
!macroend

; ---------------------------------------------------------------- welcome

Function CvWelcome
  ${If} $PassiveMode = 1
    Abort
  ${EndIf}
  !insertmacro CvPage

  !insertmacro CvLogo $cvPage 64
  !insertmacro CvText $0 $cvPage "${PRODUCTNAME}" 164 48 $cvFontTitle ${CV_TEXT} 0x200
  !insertmacro CvText $0 $cvPage "Convert any file from a radial wheel" 208 24 $cvFontBody ${CV_DIM} 0x200

  ${If} $cvCompare == "0"
    StrCpy $1 "Reinstall"
  ${ElseIf} $cvCompare == "1"
    StrCpy $1 "Update"
  ${Else}
    StrCpy $1 "Install"
  ${EndIf}
  !insertmacro CvButton $0 $cvPage "$1" -1 268 200 44 ${CV_ACCENT} ${CV_WHITE}
  ${NSD_OnClick} $0 CvNext

  !insertmacro CvText $cvDirLabel $cvPage "$INSTDIR" 336 20 $cvFontSmall ${CV_DIM} 0x8200
  !insertmacro CvText $0 $cvPage "Change location" 356 20 $cvFontSmall ${CV_ACCENT_TEXT} 0x200
  ${NSD_OnClick} $0 CvChangeDir
  ; Updates keep the folder they're in
  ${If} $cvCompare != ""
    ShowWindow $0 ${SW_HIDE}
  ${EndIf}

  !insertmacro CvText $0 $cvPage "Version ${VERSION}  ·  free and open source" 404 20 $cvFontSmall ${CV_DIM} 0x200

  nsDialogs::Show
FunctionEnd

Function CvWelcomeLeave
  ; An old MSI copy (Convertino's .msi) is removed first, as Tauri's own
  ; installer does. Registers $R0/$R1/$R6 still hold PageReinstall's findings.
  ${If} $WixMode = 1
    Call PageLeaveReinstall
  ${EndIf}
FunctionEnd

Function CvNext
  SendMessage $HWNDPARENT 0x408 1 0 ; next page
FunctionEnd

Function CvChangeDir
  nsDialogs::SelectFolderDialog "Install ${PRODUCTNAME} in" "$INSTDIR"
  Pop $0
  ${If} $0 != error
    ; Like the standard folder page: always a "Convertino" folder inside
    ${GetFileName} $0 $1
    ${If} $1 != "${PRODUCTNAME}"
      StrCpy $0 "$0\${PRODUCTNAME}"
    ${EndIf}
    StrCpy $INSTDIR $0
    SendMessage $cvDirLabel ${WM_SETTEXT} 0 "STR:$INSTDIR"
  ${EndIf}
FunctionEnd

; ---------------------------------------------------------------- progress

Function CvInstFilesShow
  StrCpy $cvOnDialogs 0
  Call CvHideChrome
  FindWindow $cvPage "#32770" "" $HWNDPARENT
  SetCtlColors $cvPage ${CV_TEXT} ${CV_BG}
  ; NSIS keeps this page at its dialog-resource size: make it fill the window
  System::Call 'user32::SetWindowPos(p $cvPage, p 0, i 0, i 0, i $cvW, i $cvH, i 0x14)'

  ; Hide the details list and its button
  GetDlgItem $0 $cvPage 1016
  ShowWindow $0 ${SW_HIDE}
  GetDlgItem $0 $cvPage 1027
  ShowWindow $0 ${SW_HIDE}

  !insertmacro CvLogo $cvPage 84
  ${If} $UpdateMode = 1
    StrCpy $1 "Updating ${PRODUCTNAME}…"
  ${Else}
    StrCpy $1 "Installing ${PRODUCTNAME}…"
  ${EndIf}
  !insertmacro CvText $0 $cvPage "$1" 192 48 $cvFontTitle ${CV_TEXT} 0x200

  ; Thin flat bar: no theme, no border, blue on a dark track
  GetDlgItem $0 $cvPage 1004
  System::Call 'uxtheme::SetWindowTheme(p r0, w " ", w " ")'
  System::Call 'user32::GetWindowLongW(p r0, i -20) i .r1'
  IntOp $1 $1 & 0xFFFDFDFF ; no WS_EX_CLIENTEDGE / WS_EX_STATICEDGE
  System::Call 'user32::SetWindowLongW(p r0, i -20, i r1)'
  System::Call 'user32::GetWindowLongW(p r0, i -16) i .r1'
  IntOp $1 $1 & 0xFF7FFFFF ; no WS_BORDER
  System::Call 'user32::SetWindowLongW(p r0, i -16, i r1)'
  SendMessage $0 0x409 0 ${CV_ACCENT_REF}   ; PBM_SETBARCOLOR
  SendMessage $0 0x2001 0 ${CV_TRACK_REF}   ; PBM_SETBKCOLOR
  !insertmacro CvPx $1 280
  !insertmacro CvPx $2 6
  !insertmacro CvPx $3 260
  IntOp $4 $cvW - $1
  IntOp $4 $4 / 2
  System::Call 'user32::SetWindowPos(p r0, p 0, i r4, i r3, i r1, i r2, i 0x34)' ; NOZORDER|NOACTIVATE|FRAMECHANGED
  !insertmacro CvPx $5 3
  IntOp $1 $1 + 1
  IntOp $2 $2 + 1
  System::Call 'gdi32::CreateRoundRectRgn(i 0, i 0, i r1, i r2, i r5, i r5) p .r5'
  System::Call 'user32::SetWindowRgn(p r0, p r5, i 1)'

  ; What's happening (e.g. the WebView2 download), small and dim
  GetDlgItem $0 $cvPage 1006
  SendMessage $0 ${WM_SETFONT} $cvFontSmall 1
  SetCtlColors $0 ${CV_DIM} ${CV_BG}
  System::Call 'user32::GetWindowLongW(p r0, i -16) i .r1'
  IntOp $1 $1 & 0xFFFFFFE0 ; it was SS_LEFTNOWORDWRAP: clear the type first
  IntOp $1 $1 | 0x8001     ; SS_CENTER | SS_PATHELLIPSIS
  System::Call 'user32::SetWindowLongW(p r0, i -16, i r1)'
  !insertmacro CvPx $1 32
  !insertmacro CvPx $2 282
  !insertmacro CvPx $3 20
  IntOp $4 $cvW - $1
  IntOp $4 $4 - $1
  System::Call 'user32::SetWindowPos(p r0, p 0, i r1, i r2, i r4, i r3, i 0x34)'
FunctionEnd

; ---------------------------------------------------------------- done

Function CvFinish
  ${If} $PassiveMode = 1
    Abort
  ${EndIf}
  !insertmacro CvPage
  StrCpy $cvOpenApp 0

  !insertmacro CvLogo $cvPage 56
  !insertmacro CvText $0 $cvPage "${PRODUCTNAME} is ready" 156 48 $cvFontTitle ${CV_TEXT} 0x200
  !insertmacro CvText $0 $cvPage "Select files in File Explorer, then hold Alt and right-click them to open the wheel." 204 44 $cvFontBody ${CV_DIM} 0

  !insertmacro CvButton $0 $cvPage "Open ${PRODUCTNAME}" 64 276 184 44 ${CV_ACCENT} ${CV_WHITE}
  ${NSD_OnClick} $0 CvOpenAndClose
  !insertmacro CvButton $0 $cvPage "Close" 260 276 116 44 ${CV_SECONDARY} ${CV_TEXT}
  ${NSD_OnClick} $0 CvNext

  ; Desktop shortcut: a bare check box plus a clickable label, because a
  ; themed check box ignores the text colour on a dark background
  !insertmacro CvPx $1 132
  !insertmacro CvPx $2 352
  !insertmacro CvPx $3 16
  !insertmacro CvControl $0 $cvPage BUTTON "" 0x50010003 $1 $2 $3 $3
  StrCpy $cvShortcutBox $0
  SetCtlColors $0 ${CV_TEXT} ${CV_BG}
  !insertmacro CvPx $1 154
  !insertmacro CvPx $2 350
  !insertmacro CvPx $3 200
  !insertmacro CvPx $4 20
  !insertmacro CvControl $0 $cvPage STATIC "Add a desktop shortcut" 0x50000300 $1 $2 $3 $4
  SendMessage $0 ${WM_SETFONT} $cvFontSmall 1
  SetCtlColors $0 ${CV_DIM} ${CV_BG}
  ${NSD_OnClick} $0 CvToggleShortcut

  nsDialogs::Show
FunctionEnd

Function CvToggleShortcut
  SendMessage $cvShortcutBox ${BM_GETCHECK} 0 0 $0
  IntOp $0 1 - $0
  SendMessage $cvShortcutBox ${BM_SETCHECK} $0 0
FunctionEnd

Function CvOpenAndClose
  StrCpy $cvOpenApp 1
  Call CvNext
FunctionEnd

Function CvFinishLeave
  SendMessage $cvShortcutBox ${BM_GETCHECK} 0 0 $0
  ${If} $0 = 1
    Call CreateOrUpdateDesktopShortcut
  ${EndIf}
  ${If} $cvOpenApp = 1
    Call RunMainBinary
  ${EndIf}
FunctionEnd

; ---------------------------------------------------------------- the `convertino` command
; The command is a copy of convertino-cli.exe named convertino.exe in its own
; folder (the app's folder already has a convertino.exe: the windowed app),
; and that folder goes on the user's PATH. `convertino setup` does the PATH
; part (src-tauri/src/shell.rs).

!macro NSIS_HOOK_POSTINSTALL
  CreateDirectory "$INSTDIR\bin"
  Delete "$INSTDIR\bin\convertino.old.exe"
  ; An AI app may be running the old command (its MCP server): a running
  ; program can't be replaced, but it can be renamed out of the way.
  ${If} ${FileExists} "$INSTDIR\bin\convertino.exe"
    Rename "$INSTDIR\bin\convertino.exe" "$INSTDIR\bin\convertino.old.exe"
  ${EndIf}
  CopyFiles /SILENT "$INSTDIR\convertino-cli.exe" "$INSTDIR\bin\convertino.exe"
  nsExec::Exec '"$INSTDIR\bin\convertino.exe" setup add "$INSTDIR\bin"'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; Removing for good (not as part of an update): take the folder off PATH and
  ; disconnect AI apps, which would otherwise keep starting a missing command.
  ${If} $UpdateMode <> 1
    nsExec::Exec '"$INSTDIR\bin\convertino.exe" setup remove "$INSTDIR\bin"'
    Pop $0
  ${EndIf}
  Delete "$INSTDIR\bin\convertino.exe"
  Delete "$INSTDIR\bin\convertino.old.exe"
  RMDir "$INSTDIR\bin"
!macroend
