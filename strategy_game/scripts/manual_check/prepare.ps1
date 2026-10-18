# P31-004 / P31-006: 手動確認用の隔離環境を準備する(ビルド・人工 Save の生成・環境の組み立て)。
#
# 実行(リポジトリの strategy_game ディレクトリ、またはどこからでも):
#   powershell -ExecutionPolicy Bypass -File scripts\manual_check\prepare.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\manual_check\prepare.ps1 -Reset victory   # 指定した環境の Save を初期状態へ戻す
#
# 作るもの(すべて target\manual_check\ の下。target は git の除外対象):
#   normal\      通常の New Game・保存/ロード用(assets のコピーと、空の saves\)
#   m1\          M1(双方武装停滞の合意白紙講和)の人工 Save 入り
#   e1\          E1(停滞した AI 戦争の白紙講和)の人工 Save 入り
#   player_war\  プレイヤー自身の戦争・講和の確認用の人工 Save 入り(Dwarf、師団 3 個、戦争は未開始)
#   victory\     勝利直前の人工 Save 入り(他 4 国は消滅済み、Arcadia が最後の 1 州を占領された状態)
#   defeat\      敗北直前の人工 Save 入り(Dwarf が最後の 1 州を占領された状態)
#   fixtures\    各環境の初期状態の Save(`<環境名>_initial.ron`)。環境の Save を初期状態へ戻すときの元
#   BUILD_INFO.txt  確認対象コード・実行ファイルの識別情報
#
# **既存の環境の Save は上書きしない**: 環境の saves\savegame_v1.ron が既にあれば(確認中に保存した Save を含む)そのまま残す。
# 無い場合だけ、初期状態の Save をコピーする。初期状態へ戻したいときだけ -Reset <環境名> を使う
# (戻す前の Save は、その環境の saves\backup\ へ日時付きで退避する)。
#
# ユーザーの saves\savegame_v1.ron(リポジトリ直下の saves\)は、読まない・書かない・移動しない。
# ゲームの保存先と data の読み込み先は、起動時の作業ディレクトリからの相対パス
# (saves/savegame_v1.ron、assets/data/*.ron)のため、各環境ディレクトリを作業ディレクトリにして起動する。
param(
    # 確認対象コード(P31-003 のコミット)。src・assets・Cargo.* がこのコミットと一致することを確認する。
    [string]$ExpectedCode = "1136c213bdeefe8e756cb2a34f34cc2cf7a40af7",
    # Save を初期状態へ戻す環境(複数可)。
    [ValidateSet("m1", "e1", "player_war", "victory", "defeat")][string[]]$Reset = @()
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

# 3. 人工 Save の生成(通常の全体テストとは別の補助処理。#[ignore] のテスト。生成先は target\ の下で、環境の Save ではない)。
Run-Native "M1 の人工 Save の生成" { cargo test --release --test p31_003_manual_fixture_test -- --ignored --nocapture }
Run-Native "E1 の人工 Save の生成" { cargo test --release --test p31_004_e1_manual_fixture_test -- --ignored --nocapture }
Run-Native "player_war / victory / defeat の人工 Save の生成" { cargo test --release --test p31_006_manual_fixture_test -- --ignored --nocapture --test-threads=1 }
$fixtureSource = @{
    "m1"         = Join-Path $repo "target\p31_003_manual_fixture\savegame_v1.ron"
    "e1"         = Join-Path $repo "target\p31_004_e1_fixture\savegame_v1.ron"
    "player_war" = Join-Path $repo "target\p31_006_fixture\player_war\savegame_v1.ron"
    "victory"    = Join-Path $repo "target\p31_006_fixture\victory\savegame_v1.ron"
    "defeat"     = Join-Path $repo "target\p31_006_fixture\defeat\savegame_v1.ron"
}
foreach ($k in $fixtureSource.Keys) {
    if (-not (Test-Path $fixtureSource[$k])) { throw "人工 Save が生成されていません($k): $($fixtureSource[$k])" }
}

# 4. 環境の組み立て。環境の Save は上書きしない(無い場合だけ初期状態をコピーする)。-Reset 指定の環境は退避してから戻す。
$fixturesDir = Join-Path $root "fixtures"
New-Item -ItemType Directory -Force $fixturesDir | Out-Null
function Setup-Env([string]$name) {
    $dir = Join-Path $root $name
    $saveDir = Join-Path $dir "saves"
    New-Item -ItemType Directory -Force $saveDir | Out-Null
    if (Test-Path (Join-Path $dir "assets")) { Remove-Item -Recurse -Force (Join-Path $dir "assets") }
    Copy-Item -Recurse -Force (Join-Path $repo "assets") (Join-Path $dir "assets")
    if (-not $fixtureSource.ContainsKey($name)) { return "normal(Save なし。確認で作った Save はそのまま)" }
    $initial = Join-Path $fixturesDir "$($name)_initial.ron"
    Copy-Item -Force $fixtureSource[$name] $initial
    $save = Join-Path $saveDir "savegame_v1.ron"
    if (Test-Path $save) {
        if ($Reset -contains $name) {
            $backup = Join-Path $saveDir "backup"
            New-Item -ItemType Directory -Force $backup | Out-Null
            $stamp = Get-Date -Format "yyyyMMdd_HHmmss"
            Move-Item -Force $save (Join-Path $backup "savegame_v1_$stamp.ron")
            Copy-Item -Force $initial $save
            return "初期状態へ戻した(戻す前の Save は saves\backup\savegame_v1_$stamp.ron)"
        }
        return "既存の Save を残した"
    }
    Copy-Item -Force $initial $save
    return "初期状態の Save を作成した"
}
$setupResult = [ordered]@{}
foreach ($n in @("normal", "m1", "e1", "player_war", "victory", "defeat")) { $setupResult[$n] = Setup-Env $n }

# 5. 識別情報。
$exeInfo = Get-Item $exe
$sha = (Get-FileHash $exe -Algorithm SHA256).Hash
$info = @(
    "確認対象コード: $ExpectedCode",
    "HEAD: $head",
    "実行ファイル: $exe",
    "実行ファイルの SHA256: $sha",
    "実行ファイルの更新日時: $($exeInfo.LastWriteTime)"
)
foreach ($k in @("m1", "e1", "player_war", "victory", "defeat")) {
    $info += "$k の初期状態の人工 Save の SHA256: $((Get-FileHash (Join-Path $fixturesDir "$($k)_initial.ron") -Algorithm SHA256).Hash)"
}
$info += "準備した日時: $(Get-Date)"
Set-Content -Path (Join-Path $root "BUILD_INFO.txt") -Value $info -Encoding UTF8
Write-Host ""
Write-Host "準備完了: $root"
$info | ForEach-Object { Write-Host "  $_" }
Write-Host ""
foreach ($k in $setupResult.Keys) { Write-Host "  環境 $k : $($setupResult[$k])" }
Write-Host ""
Write-Host "起動: powershell -ExecutionPolicy Bypass -File scripts\manual_check\run.ps1 <normal|m1|e1|player_war|victory|defeat>"
