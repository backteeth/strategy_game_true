# P31-004: 手動確認用の隔離環境を準備する(ビルド・人工 Save の生成・環境の組み立て)。
#
# 実行(リポジトリの strategy_game ディレクトリ、またはどこからでも):
#   powershell -ExecutionPolicy Bypass -File scripts\manual_check\prepare.ps1
#
# 作るもの(すべて target\manual_check\ の下。target は git の除外対象):
#   normal\  通常の New Game・保存/ロード用(assets のコピーと、空の saves\)
#   m1\      M1(双方武装停滞の合意白紙講和)の人工 Save 入り(saves\savegame_v1.ron は人工の fixture)
#   e1\      E1(停滞した AI 戦争の白紙講和)の人工 Save 入り(同上)
#   BUILD_INFO.txt  確認対象コード・実行ファイルの識別情報
#
# ユーザーの saves\savegame_v1.ron(リポジトリ直下の saves\)は、読まない・書かない・移動しない。
# ゲームの保存先と data の読み込み先は、起動時の作業ディレクトリからの相対パス
# (saves/savegame_v1.ron、assets/data/*.ron)のため、各環境ディレクトリを作業ディレクトリにして起動する。
param(
    # 確認対象コード(P31-003 のコミット)。src・assets・Cargo.* がこのコミットと一致することを確認する。
    [string]$ExpectedCode = "1136c213bdeefe8e756cb2a34f34cc2cf7a40af7"
)
$ErrorActionPreference = "Stop"

$repo = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Set-Location $repo
$root = Join-Path $repo "target\manual_check"

function Run-Native([string]$what, [scriptblock]$cmd) {
    & $cmd
    if ($LASTEXITCODE -ne 0) { throw "$what に失敗しました(終了コード $LASTEXITCODE)" }
}

# 1. 確認対象コードの識別: src・assets・Cargo.toml・Cargo.lock が、確認対象のコミットと一致し、未コミットの変更がない。
$paths = @("src", "assets", "Cargo.toml", "Cargo.lock")
git cat-file -e "$ExpectedCode^{commit}" 2>$null
if ($LASTEXITCODE -ne 0) { throw "確認対象のコミット $ExpectedCode が見つかりません" }
$diff = git diff --name-only $ExpectedCode HEAD -- $paths
if ($diff) { throw "確認対象コードと HEAD の差分があります(実行を中止):`n$diff" }
$dirty = git status --porcelain -- $paths
if ($dirty) { throw "src / assets / Cargo.* に未コミットの変更があります(実行を中止):`n$dirty" }
$head = (git rev-parse HEAD).Trim()
Write-Host "確認対象コード: $ExpectedCode(HEAD: $head。src・assets・Cargo.* は一致)"

# 2. 実行ファイルのビルド(release)。
Run-Native "cargo build --release" { cargo build --release --bin strategy_game }
$exe = Join-Path $repo "target\release\strategy_game.exe"
if (-not (Test-Path $exe)) { throw "実行ファイルがありません: $exe" }

# 3. 人工 Save の生成(通常の全体テストとは別の補助処理。#[ignore] のテスト)。
Run-Native "M1 の人工 Save の生成" { cargo test --release --test p31_003_manual_fixture_test -- --ignored --nocapture }
Run-Native "E1 の人工 Save の生成" { cargo test --release --test p31_004_e1_manual_fixture_test -- --ignored --nocapture }
$m1Save = Join-Path $repo "target\p31_003_manual_fixture\savegame_v1.ron"
$e1Save = Join-Path $repo "target\p31_004_e1_fixture\savegame_v1.ron"
foreach ($s in @($m1Save, $e1Save)) { if (-not (Test-Path $s)) { throw "人工 Save が生成されていません: $s" } }

# 4. 環境の組み立て。normal の saves\ は、以前の確認で作った Save を消さない。人工 Save は毎回作り直す。
function Setup-Env([string]$name, [string]$fixture) {
    $dir = Join-Path $root $name
    New-Item -ItemType Directory -Force (Join-Path $dir "saves") | Out-Null
    if (Test-Path (Join-Path $dir "assets")) { Remove-Item -Recurse -Force (Join-Path $dir "assets") }
    Copy-Item -Recurse -Force (Join-Path $repo "assets") (Join-Path $dir "assets")
    if ($fixture) { Copy-Item -Force $fixture (Join-Path $dir "saves\savegame_v1.ron") }
}
Setup-Env "normal" $null
Setup-Env "m1" $m1Save
Setup-Env "e1" $e1Save

# 5. 識別情報。
$exeInfo = Get-Item $exe
$sha = (Get-FileHash $exe -Algorithm SHA256).Hash
$info = @(
    "確認対象コード: $ExpectedCode",
    "HEAD: $head",
    "実行ファイル: $exe",
    "実行ファイルの SHA256: $sha",
    "実行ファイルの更新日時: $($exeInfo.LastWriteTime)",
    "M1 の人工 Save の SHA256: $((Get-FileHash $m1Save -Algorithm SHA256).Hash)",
    "E1 の人工 Save の SHA256: $((Get-FileHash $e1Save -Algorithm SHA256).Hash)",
    "準備した日時: $(Get-Date)"
)
Set-Content -Path (Join-Path $root "BUILD_INFO.txt") -Value $info -Encoding UTF8
Write-Host ""
Write-Host "準備完了: $root"
$info | ForEach-Object { Write-Host "  $_" }
Write-Host ""
Write-Host "起動: powershell -ExecutionPolicy Bypass -File scripts\manual_check\run.ps1 normal   (m1 / e1 も同様)"
