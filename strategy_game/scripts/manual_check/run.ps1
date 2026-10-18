# P31-004: 隔離した環境でゲームを起動する。
#
# 実行(prepare.ps1 の後):
#   powershell -ExecutionPolicy Bypass -File scripts\manual_check\run.ps1 normal   # 通常の New Game・保存/ロード
#   powershell -ExecutionPolicy Bypass -File scripts\manual_check\run.ps1 m1       # M1 の人工 Save
#   powershell -ExecutionPolicy Bypass -File scripts\manual_check\run.ps1 e1       # E1 の人工 Save
#   powershell -ExecutionPolicy Bypass -File scripts\manual_check\run.ps1 player_war  # プレイヤー自身の戦争・講和(P31-006)
#   powershell -ExecutionPolicy Bypass -File scripts\manual_check\run.ps1 victory     # 勝利直前の人工 Save(P31-006)
#   powershell -ExecutionPolicy Bypass -File scripts\manual_check\run.ps1 defeat      # 敗北直前の人工 Save(P31-006)
#
# 作業ディレクトリを target\manual_check\<環境名> にして、確認対象の実行ファイルを起動する。
# ゲームが読み書きする Save と assets は、その作業ディレクトリの saves\ と assets\ だけで、
# ユーザーの saves\savegame_v1.ron(リポジトリ直下の saves\)には触れない。
param(
    [Parameter(Mandatory = $true)][ValidateSet("normal", "m1", "e1", "player_war", "victory", "defeat")][string]$Environment
)
$ErrorActionPreference = "Stop"

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$root = Join-Path $repo "target\manual_check"
$dir = Join-Path $root $Environment
$exe = Join-Path $repo "target\release\strategy_game.exe"
$userSaves = Join-Path $repo "saves"

# 安全確認: 隔離ディレクトリであること、assets があること。
if (-not (Test-Path (Join-Path $dir "assets\data\buildings.ron"))) {
    throw "環境 '$Environment' が準備されていません。先に prepare.ps1 を実行してください: $dir"
}
if ((Resolve-Path $dir).Path -eq $repo -or (Resolve-Path $dir).Path -eq (Split-Path $userSaves -Parent)) {
    throw "作業ディレクトリがリポジトリ直下になっています。中止します。"
}
if (-not (Test-Path $exe)) { throw "実行ファイルがありません。先に prepare.ps1 を実行してください: $exe" }

# 実行ファイルの識別情報(prepare.ps1 が記録したものと照合する)。
$buildInfo = Join-Path $root "BUILD_INFO.txt"
if (Test-Path $buildInfo) {
    $recorded = (Select-String -Path $buildInfo -Pattern "^実行ファイルの SHA256: (.+)$").Matches[0].Groups[1].Value
    $actual = (Get-FileHash $exe -Algorithm SHA256).Hash
    if ($recorded -ne $actual) {
        throw "実行ファイルが prepare.ps1 の記録と異なります(再ビルドされた可能性)。prepare.ps1 を実行し直してください。"
    }
}

# bevy の dynamic_linking のため、実行ファイルが参照する DLL(target\release\deps の bevy_dylib-<hash>.dll と、
# rustc の標準ライブラリの DLL)を検索できるようにする(`cargo run` が PATH へ加えるものと同じ)。
$deps = Join-Path (Split-Path $exe -Parent) "deps"
$env:PATH = "$deps;$(Split-Path $exe -Parent);$env:PATH"
try {
    $libdir = (rustc --print target-libdir).Trim()
    if ($libdir) { $env:PATH = "$libdir;$env:PATH" }
} catch { }

$save = Join-Path $dir "saves\savegame_v1.ron"
Write-Host "環境: $Environment"
Write-Host "作業ディレクトリ: $dir"
Write-Host "実行ファイル: $exe"
Write-Host "assets の読み込み: $dir\assets\data\*.ron(作業ディレクトリ相対)"
Write-Host "Save の読み書き: $save"
if (Test-Path $buildInfo) { Get-Content $buildInfo | Select-Object -First 2 | ForEach-Object { Write-Host "  $_" } }
Write-Host "ウィンドウを閉じるまで、このコンソールで実行します。"

Push-Location $dir
try { & $exe } finally { Pop-Location }
