#!/bin/sh
# Build for Linux and macOS: collects the result into ./dist
#   Linux : dist/EasyTypstEditor  (+ register.sh, which adds it to the application menu)
#   macOS : dist/Easy Typst Editor.app   (UNIVERSAL=1 sh build.sh → Apple Silicon + Intel in one app)
# 빌드(Linux · macOS): 결과물을 dist 폴더에 모은다. Windows 는 build.ps1 을 쓴다.
set -eu
cd "$(dirname "$0")"
dist="$PWD/dist"
target_dir=$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
mkdir -p "$dist"

case "$(uname -s)" in
Darwin)
    if [ "${UNIVERSAL:-0}" = 1 ]; then
        rustup target add aarch64-apple-darwin x86_64-apple-darwin
        cargo build --release --target aarch64-apple-darwin
        cargo build --release --target x86_64-apple-darwin
        bin="$target_dir/EasyTypstEditor-universal"
        lipo -create -output "$bin" \
            "$target_dir/aarch64-apple-darwin/release/EasyTypstEditor" \
            "$target_dir/x86_64-apple-darwin/release/EasyTypstEditor"
    else
        cargo build --release
        bin="$target_dir/release/EasyTypstEditor"
    fi
    app="$dist/Easy Typst Editor.app"
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS"
    cp "$bin" "$app/Contents/MacOS/EasyTypstEditor"
    # CFBundleDocumentTypes: Finder 의 "다음으로 열기"에 .typ 가 나온다(기본 프로그램을 빼앗지는 않는다).
    # CFBundleLocalizations: 파일 고르기 창 같은 시스템 화면이 사용자의 언어로 나온다.
    cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Easy Typst Editor</string>
    <key>CFBundleDisplayName</key><string>Easy Typst Editor</string>
    <key>CFBundleIdentifier</key><string>io.github.qnai-ai.EasyTypstEditor</string>
    <key>CFBundleExecutable</key><string>EasyTypstEditor</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleAllowMixedLocalizations</key><true/>
    <key>CFBundleLocalizations</key>
    <array><string>en</string><string>ko</string><string>ja</string><string>zh_CN</string><string>es</string></array>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Typst document</string>
            <key>CFBundleTypeRole</key><string>Editor</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>CFBundleTypeExtensions</key><array><string>typ</string></array>
        </dict>
    </array>
</dict>
</plist>
EOF
    # 서명(ad-hoc): Apple Silicon 은 서명이 없는 앱을 실행하지 않는다
    codesign --force --sign - "$app"
    echo "Done: $app"
    ;;
*)
    cargo build --release
    cp "$target_dir/release/EasyTypstEditor" "$dist/"
    cp register.sh "$dist/"
    chmod +x "$dist/EasyTypstEditor" "$dist/register.sh"
    echo "Done: $dist/EasyTypstEditor"
    ;;
esac
