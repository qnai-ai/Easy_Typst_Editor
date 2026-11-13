# Build: collects EasyTypstEditor.exe (and WebView2Loader.dll when the GNU toolchain needs it) into .\dist
# 빌드: 실행 파일과 WebView2Loader.dll 을 dist 폴더에 모은다.
$ErrorActionPreference = 'Continue'
# GNU toolchain users: put a WinLibs MinGW from winget on PATH if present (MSVC users need nothing).
$mingw = Get-ChildItem "$env:LOCALAPPDATA\Microsoft\WinGet\Packages" -Directory -ErrorAction SilentlyContinue | Where-Object Name -like 'BrechtSanders.WinLibs*' | Select-Object -First 1
if ($mingw) { $env:Path = "$($mingw.FullName)\mingw64\bin;$env:Path" }
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
Set-Location $PSScriptRoot
cargo build --release
if ($LASTEXITCODE -ne 0) { exit 1 }
$meta = cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
$target = Join-Path $meta.target_directory 'release'
$dist = Join-Path $PSScriptRoot 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
$dll = Get-ChildItem "$target\build" -Recurse -Filter WebView2Loader.dll -ErrorAction SilentlyContinue | Where-Object FullName -like '*\x64\*' | Select-Object -First 1
if ($dll) {
    Copy-Item $dll.FullName $dist -Force
    Copy-Item $dll.FullName $target -Force
}
Get-Process EasyTypstEditor -ErrorAction SilentlyContinue | Where-Object Path -like "$dist*" | Stop-Process -Force
Copy-Item "$target\EasyTypstEditor.exe" $dist -Force
Write-Host "Done: $dist\EasyTypstEditor.exe"
