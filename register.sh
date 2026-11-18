#!/bin/sh
# Linux: adds Easy Typst Editor to the application menu and opens .typ files with it
# on double-click (current user only, no root needed).
#   register:  sh register.sh
#   undo:      sh register.sh -u     (goes back to the program that opened .typ before)
# Linux: 앱 메뉴에 넣고, .typ 파일을 더블클릭하면 이 편집기로 열리게 한다(현재 사용자만).
# Windows 는 register.ps1 을 쓴다. macOS 는 필요 없다(앱 안의 Info.plist 가 같은 일을 한다).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
exe="$here/EasyTypstEditor"
[ -x "$exe" ] || exe="$here/dist/EasyTypstEditor"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
desktop="$data/applications/easy-typst-editor.desktop"
mime="$data/mime/packages/easy-typst-editor.xml"
keep="$data/EasyTypstEditor/previous-typ-handler"
type="text/x-typst"

refresh() {
    command -v update-mime-database >/dev/null 2>&1 && update-mime-database "$data/mime" >/dev/null 2>&1 || true
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$data/applications" >/dev/null 2>&1 || true
}

if [ "${1:-}" = "-u" ] || [ "${1:-}" = "--undo" ]; then
    prev=""
    [ -f "$keep" ] && prev=$(cat "$keep")
    rm -f "$desktop" "$mime" "$keep"
    refresh
    if [ -n "$prev" ] && command -v xdg-mime >/dev/null 2>&1; then xdg-mime default "$prev" "$type" || true; fi
    echo "Unregistered. .typ opens with: ${prev:-(system default)}"
    exit 0
fi

if [ ! -x "$exe" ]; then echo "Executable not found: $exe  (run: sh build.sh)"; exit 1; fi
mkdir -p "$data/applications" "$data/mime/packages" "$data/EasyTypstEditor"

# 등록 전의 연결을 적어 둔다(되돌릴 때 쓴다)
if command -v xdg-mime >/dev/null 2>&1; then
    cur=$(xdg-mime query default "$type" 2>/dev/null || true)
    if [ -n "$cur" ] && [ "$cur" != "easy-typst-editor.desktop" ]; then printf '%s\n' "$cur" > "$keep"; fi
fi

cat > "$mime" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">
  <mime-type type="$type">
    <comment>Typst document</comment>
    <sub-class-of type="text/plain"/>
    <glob pattern="*.typ"/>
  </mime-type>
</mime-info>
EOF

cat > "$desktop" <<EOF
[Desktop Entry]
Type=Application
Name=Easy Typst Editor
Name[ko]=쉬운 지면 편집기
Comment=Edit Typst documents directly on the rendered page
Comment[ko]=Typst 교재를 지면에서 바로 고칩니다
Exec="$exe" %f
Terminal=false
Categories=Office;WordProcessor;Publishing;
MimeType=$type;
StartupWMClass=EasyTypstEditor
EOF

refresh
command -v xdg-mime >/dev/null 2>&1 && xdg-mime default easy-typst-editor.desktop "$type" || true
echo "Registered: .typ -> $exe"
