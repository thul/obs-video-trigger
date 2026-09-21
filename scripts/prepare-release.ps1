param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidatePattern('^v?\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$')]
    [string] $Version
)

$ErrorActionPreference = 'Stop'
$Version = $Version.TrimStart('v')
$Tag = "v$Version"

if ((git status --porcelain).Length -ne 0) {
    throw 'The working tree must be clean before preparing a release.'
}

git rev-parse --verify --quiet "refs/tags/$Tag" | Out-Null
if ($LASTEXITCODE -eq 0) {
    throw "The local tag $Tag already exists."
}

$Manifest = Join-Path $PSScriptRoot '..\Cargo.toml'
$Lines = Get-Content -LiteralPath $Manifest
$VersionLine = -1
for ($Index = 0; $Index -lt $Lines.Count; $Index++) {
    if ($Lines[$Index] -match '^version\s*=') {
        $VersionLine = $Index
        break
    }
}
if ($VersionLine -lt 0) {
    throw 'Could not find the package version in Cargo.toml.'
}
$Lines[$VersionLine] = "version = `"$Version`""
Set-Content -LiteralPath $Manifest -Value $Lines -Encoding utf8

cargo check
if ($LASTEXITCODE -ne 0) { throw 'cargo check failed.' }
cargo fmt --check
if ($LASTEXITCODE -ne 0) { throw 'cargo fmt failed.' }
cargo clippy --all-targets --locked -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'cargo clippy failed.' }
cargo test --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo test failed.' }

$PackageVersion = (cargo metadata --locked --no-deps --format-version 1 |
    ConvertFrom-Json).packages[0].version
if ($PackageVersion -ne $Version) {
    throw "Cargo resolved version $PackageVersion instead of $Version."
}

Write-Host "Prepared $Tag and updated Cargo.toml/Cargo.lock."
Write-Host 'Review the changes, then run:'
Write-Host '  git add Cargo.toml Cargo.lock'
Write-Host "  git commit -m 'Release $Tag'"
Write-Host '  .\scripts\tag-release.ps1 -Push'
