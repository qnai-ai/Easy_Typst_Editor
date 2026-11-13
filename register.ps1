# .typ 파일을 더블클릭하면 쉬운 지면 편집기로 열리게 한다(현재 사용자만, 관리자 권한 불필요).
#   등록:  powershell -ExecutionPolicy Bypass -File register.ps1
#   해제:  powershell -ExecutionPolicy Bypass -File register.ps1 -Undo   (등록 전의 연결로 되돌린다)
param([switch]$Undo)
$exe = Join-Path $PSScriptRoot 'dist\EasyTypstEditor.exe'
$ext = 'HKCU:\Software\Classes\.typ'
$progId = 'EasyTypstEditor.typ'
$prog = "HKCU:\Software\Classes\$progId"
$keep = 'HKCU:\Software\EasyTypstEditor'

if ($Undo) {
    $prev = (Get-ItemProperty $keep -ErrorAction SilentlyContinue).PreviousTypProgId
    if ($prev) { Set-Item $ext -Value $prev } elseif (Test-Path $ext) { Set-Item $ext -Value '' }
    Remove-Item $prog -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host "해제했습니다. .typ 연결: $prev"
} else {
    if (-not (Test-Path $exe)) { Write-Host "실행 파일이 없습니다: $exe"; exit 1 }
    if (-not (Test-Path $ext)) { New-Item $ext -Force | Out-Null }
    $cur = (Get-Item $ext).GetValue('')
    if ($cur -and $cur -ne $progId) {
        New-Item $keep -Force | Out-Null
        Set-ItemProperty $keep -Name PreviousTypProgId -Value $cur
        # 예전 프로그램은 '연결 프로그램' 목록에 남긴다
        New-Item "$ext\OpenWithProgids" -Force | Out-Null
        New-ItemProperty "$ext\OpenWithProgids" -Name $cur -Value '' -PropertyType String -Force | Out-Null
    }
    New-Item "$prog\shell\open\command" -Force | Out-Null
    Set-Item $prog -Value 'Typst 교재 원고'
    Set-Item "$prog\shell\open" -Value '쉬운 지면 편집기로 열기'
    Set-Item "$prog\shell\open\command" -Value "`"$exe`" `"%1`""
    New-Item "$prog\DefaultIcon" -Force | Out-Null
    Set-Item "$prog\DefaultIcon" -Value "`"$exe`",0"
    Set-Item $ext -Value $progId
    Write-Host "등록했습니다: .typ → $exe"
}
# 탐색기에 바뀐 연결을 알린다
$sig = '[DllImport("shell32.dll")] public static extern void SHChangeNotify(int e, uint f, IntPtr a, IntPtr b);'
$shell = Add-Type -Namespace EteWin -Name Shell -MemberDefinition $sig -PassThru
$shell::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
