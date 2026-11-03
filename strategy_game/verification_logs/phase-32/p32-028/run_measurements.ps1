# P32-028 measurements, run sequentially (never in parallel with builds, tests or other measurements).
#   powershell -NoProfile -ExecutionPolicy Bypass -File verification_logs\phase-32\p32-028\run_measurements.ps1   (from strategy_game\)
# Prerequisite: build_both.sh created new_* and old_* release binaries in ~/p28_exes.
# CPU affinity: mask 0xFFFF (logical processors 0-15) pinned, and unpinned. The mask does not claim a core type.
$ErrorActionPreference = "Stop"
$sysroot = (& rustc --print sysroot).Trim()
$env:PATH = "$PWD\target\release\deps;$sysroot\bin;" + $env:PATH
$exes = Join-Path $HOME "p28_exes"
$raw = "verification_logs\phase-32\p32-028\raw"
New-Item -ItemType Directory -Force $raw | Out-Null

function Run-Exe([string]$name, [string]$exeName, [string]$argText, [int64]$mask) {
    $exe = Join-Path $exes $exeName
    $out = Join-Path $raw ($name + ".txt")
    $err = Join-Path $raw ($name + ".err.txt")
    $p = Start-Process -FilePath $exe -ArgumentList $argText -PassThru -NoNewWindow -RedirectStandardOutput $out -RedirectStandardError $err
    if ($mask -ne 0) { $p.ProcessorAffinity = [IntPtr]$mask }
    $p.WaitForExit()
    Write-Output ("done " + $name + " exe=" + $exeName + " mask=" + $mask)
}

$cmp = "new_economy_parse_compare.exe"
# 1. decode alone: new decode (fast path) vs legacy decode, same text, same trial
Run-Exe "decode_1000x3_pinned_a" $cmp "--section trials --states 1000 --countries 3 --trials 3" 65535
Run-Exe "decode_1000x3_unpinned" $cmp "--section trials --states 1000 --countries 3 --trials 3" 0
Run-Exe "decode_1000x100_pinned" $cmp "--section trials --states 1000 --countries 100 --trials 3" 65535
Run-Exe "decode_300x3_pinned"    $cmp "--section trials --states 300 --countries 3 --trials 3" 65535
Run-Exe "decode_100x3_pinned"    $cmp "--section trials --states 100 --countries 3 --trials 3" 65535
# 2. fallback cost on small invalid inputs (a 100-state save, broken)
Run-Exe "fallback_100x3_pinned"  $cmp "--section fallback --states 100 --countries 3 --trials 5" 65535

# 3. whole save/restore/inspect, old (551eaa2) vs new, order old,new,new,old; timed (phases) and untimed
$ser = "--series --cases 1000x3,1000x100 --modes on,off"
Run-Exe "e2e_pinned_old_a" "old_economy_save_timing.exe" $ser 65535
Run-Exe "e2e_pinned_new_a" "new_economy_save_timing.exe" $ser 65535
Run-Exe "e2e_pinned_new_b" "new_economy_save_timing.exe" $ser 65535
Run-Exe "e2e_pinned_old_b" "old_economy_save_timing.exe" $ser 65535
# unpinned (order old,new)
Run-Exe "e2e_unpinned_old" "old_economy_save_timing.exe" $ser 0
Run-Exe "e2e_unpinned_new" "new_economy_save_timing.exe" $ser 0
"all done"
