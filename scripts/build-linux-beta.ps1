param([switch]$Check, [switch]$LoaderValidation)
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Set-Location -LiteralPath $workspace
function Invoke-BuildTool([string]$program, [string[]]$arguments) {
  & $program @arguments
  if ($LASTEXITCODE -ne 0) { throw "$program failed ($LASTEXITCODE)" }
}
$revision = (& git rev-parse HEAD).Trim()
if (& git status --porcelain --untracked-files=normal) { throw 'Commit task changes before building a candidate.' }
$version = (Get-Content -LiteralPath 'desktop/tauri.beta.conf.json' -Raw | ConvertFrom-Json).version
if ($version -notmatch '^\d+\.\d+\.\d+-beta\.\d+$') { throw 'Expected a beta version.' }
$outputKind = if ($LoaderValidation) { 'loader-validation' } else { 'beta' }
$destination = Join-Path $workspace "target/$outputKind/$version/linux"
Write-Output "Linux beta: $version; source: $revision; output: $destination"
if ($Check) { return }
if ([string]::IsNullOrWhiteSpace($env:PANITA_LAUNCHER_API_KEY)) {
  throw 'Set the public PANITA_LAUNCHER_API_KEY build variable before packaging.'
}
if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY)) {
  throw 'Set TAURI_SIGNING_PRIVATE_KEY (key text or path) and TAURI_SIGNING_PRIVATE_KEY_PASSWORD to create updater signatures.'
}
# The container cannot see host paths, so pass the key by value through the environment only.
if (Test-Path -LiteralPath $env:TAURI_SIGNING_PRIVATE_KEY -PathType Leaf) {
  $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content -LiteralPath $env:TAURI_SIGNING_PRIVATE_KEY -Raw).Trim()
}
$runId = [guid]::NewGuid().ToString('N')
$buildRoot = Join-Path $workspace "target/linux-build/$runId"
[IO.Directory]::CreateDirectory($buildRoot) | Out-Null
$bundle = Join-Path $buildRoot 'source.bundle'
Invoke-BuildTool 'git' @('bundle', 'create', $bundle, 'HEAD')
Invoke-BuildTool 'docker' @('build', '--platform', 'linux/amd64', '--tag', 'panita-client-build:ubuntu24.04', '--file', 'scripts/linux/Dockerfile', 'scripts/linux')
$imageId = (& docker image inspect --format '{{.Id}}' 'panita-client-build:ubuntu24.04').Trim()
if ($LASTEXITCODE -ne 0) { throw 'Cannot inspect build image.' }
$container = "panita-linux-beta-$runId"
$inputMount = "type=bind,source=$buildRoot,target=/input,readonly"
$scriptPath = Join-Path $workspace 'scripts/linux/build-candidate.sh'
$scriptMount = "type=bind,source=$scriptPath,target=/build-candidate.sh,readonly"
Invoke-BuildTool 'docker' @('create', '--platform', 'linux/amd64', '--name', $container,
  '--cpus', '2', '--memory', '4g', '--memory-swap', '6g', '--env', 'CARGO_BUILD_JOBS=2',
  '--env', 'PANITA_LAUNCHER_API_KEY',
  '--env', 'TAURI_SIGNING_PRIVATE_KEY', '--env', 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD',
  '--mount', $inputMount, '--mount', $scriptMount,
  '--mount', 'type=volume,source=panita-linux-cargo-registry,target=/home/panita-build/.cargo/registry',
  '--mount', 'type=volume,source=panita-linux-cargo-git,target=/home/panita-build/.cargo/git',
  '--mount', 'type=volume,source=panita-linux-build-target,target=/build-target',
  $imageId, 'bash', '/build-candidate.sh', $revision, $outputKind)
try {
  Invoke-BuildTool 'docker' @('start', '--attach', $container)
  $exitCode = (& docker inspect --format '{{.State.ExitCode}}' $container).Trim()
  if ($LASTEXITCODE -ne 0 -or $exitCode -ne '0') { throw "Linux build exited with status $exitCode. Container retained: $container" }
  if ((& git rev-parse HEAD).Trim() -ne $revision -or (& git status --porcelain --untracked-files=normal)) {
    throw 'Source changed during packaging; candidate is not delivered.'
  }
  [IO.Directory]::CreateDirectory($destination) | Out-Null
  Invoke-BuildTool 'docker' @('cp', "${container}:/work/panita-client/target/$outputKind/$version/linux/.", $destination)
  [ordered]@{ schema=1; type='linux_build_environment'; source_revision=$revision; image=$imageId;
    baseline='Ubuntu 24.04 x86_64'; installed_product_qualification='pending' } |
    ConvertTo-Json | Set-Content -LiteralPath (Join-Path $destination 'build-environment.json') -Encoding utf8
  Invoke-BuildTool 'docker' @('rm', $container)
  Write-Output "Linux candidate prepared: $destination"
} catch {
  Write-Warning "Build context retained for diagnostics: $buildRoot; container: $container"
  throw
}
