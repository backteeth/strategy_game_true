# P32-027: CPU アフィニティを固定した追加測定(2倍の速度差が試行番号ではなく環境側の要因かを見る)。strategy_game\ で実行。
# 仮定(未検証): i7-12700F の論理プロセッサ 0-15 が P コア(HT 2本ずつ)、16-19 が E コア。マスク 0xFFFF / 0xF0000。
$ErrorActionPreference = "Stop"
$sysroot = (& rustc --print sysroot).Trim()
$env:PATH = "$PWD\target\release\deps;$sysroot\bin;" + $env:PATH
$exe = "$PWD\target\release\economy_parse_compare.exe"
$out = "verification_logs\phase-32\p32-027\raw"
foreach ($m in @(@("FFFF", 0xFFFF), @("F0000", 0xF0000), @("FFFFF", 0xFFFFF))) {
    $name = $m[0]; $mask = [int64]$m[1]
    $p = Start-Process -FilePath $exe -ArgumentList @("--section", "trials", "--states", "1000", "--countries", "3", "--trials", "3") `
        -PassThru -NoNewWindow -RedirectStandardOutput "$out\affinity_$name.txt" -RedirectStandardError "$out\affinity_${name}_err.txt"
    $p.ProcessorAffinity = [IntPtr]$mask
    $p.WaitForExit()
    "mask $name applied=$($mask) exit=$($p.ExitCode)"
}
