param(
    [switch] $Push
)

$ErrorActionPreference = 'Stop'

if ((git status --porcelain).Length -ne 0) {
    throw 'Commit all release changes before tagging.'
}

$Metadata = cargo metadata --locked --no-deps --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Could not read Cargo package metadata.' }
$Version = $Metadata.packages[0].version
$Tag = "v$Version"
$Head = git rev-parse HEAD
$Existing = git rev-parse --verify --quiet "refs/tags/$Tag"

if ($LASTEXITCODE -eq 0 -and $Existing -ne $Head) {
    throw "$Tag already exists at a different commit."
}

if ($Push) {
    git ls-remote --exit-code --tags origin "refs/tags/$Tag" | Out-Null
    if ($LASTEXITCODE -eq 0) {
        throw "The remote tag $Tag already exists."
    }
    if ($LASTEXITCODE -ne 2) {
        throw 'Could not check remote tags.'
    }

    # Publish the release commit before the tag can start the workflow.
    git push origin HEAD
    if ($LASTEXITCODE -ne 0) { throw 'Could not push the release commit.' }
}

if (-not $Existing) {
    git tag --annotate $Tag --message "Release $Tag"
    if ($LASTEXITCODE -ne 0) { throw "Could not create $Tag." }
}

if ($Push) {
    git push origin $Tag
    if ($LASTEXITCODE -ne 0) { throw "Could not push $Tag." }
    Write-Host "Published $Tag. The release workflow is now running."
} else {
    Write-Host "Created $Tag locally. Push it with: git push origin $Tag"
}
