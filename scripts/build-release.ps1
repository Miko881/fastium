param([ValidateSet('x86_64-pc-windows-msvc','aarch64-pc-windows-msvc')][string]$Target = 'x86_64-pc-windows-msvc')
$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
Push-Location $workspace
try {
    if (!(Test-Path -LiteralPath Cargo.lock)) {
        & cargo generate-lockfile
        if ($LASTEXITCODE -ne 0) { throw "Nie udalo sie rozwiazac zaleznosci." }
    }
    & cargo test --locked
    if ($LASTEXITCODE -ne 0) { throw "Testy nie przeszly." }
    & cargo build --release --locked --target $Target
    if ($LASTEXITCODE -ne 0) { throw "Kompilacja nie powiodla sie." }
    $exe = Join-Path $workspace "target\$Target\release\fastium.exe"
    $size = (Get-Item -LiteralPath $exe).Length
    if ($size -ge 30000000) { throw "EXE przekracza limit 30 MB: $size bajtow." }
    $package = Join-Path $workspace "dist\Fastium-$Target"
    New-Item -ItemType Directory -Force -Path $package | Out-Null
    Copy-Item -LiteralPath $exe -Destination $package
    Copy-Item -LiteralPath README.md,LICENSE,Cargo.lock -Destination $package
    & cargo tree --locked --target $Target | Out-File -LiteralPath (Join-Path $package 'dependencies.txt') -Encoding utf8
    if ($LASTEXITCODE -ne 0) { throw "Nie udalo sie zapisac listy zaleznosci." }
    $hash = Get-FileHash -LiteralPath (Join-Path $package 'fastium.exe') -Algorithm SHA256
    "$($hash.Hash)  fastium.exe" | Set-Content -LiteralPath (Join-Path $package 'SHA256SUMS.txt') -Encoding ascii
    $archive = "$package.zip"
    Compress-Archive -Path (Join-Path $package '*') -DestinationPath $archive -Force
    if ((Get-Item -LiteralPath $archive).Length -ge 30000000) { throw "Archiwum przekracza limit 30 MB." }
    Write-Host ('Fastium EXE: {0:N2} MB ({1} bajtow)' -f ($size / 1000000), $size)
    Write-Host "Paczka portable: $archive"

    $isccCandidates = @(
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
    )
    $iscc = $isccCandidates | Where-Object { Test-Path $_ -PathType Leaf } | Select-Object -First 1
    if (-not $iscc) {
        $cmd = Get-Command iscc.exe -ErrorAction SilentlyContinue
        if ($cmd) { $iscc = $cmd.Source }
    }
    if ($iscc -and (Test-Path (Join-Path $workspace 'installer.iss'))) {
        Write-Host "Kompilowanie instalatora Windows ($iscc)..."
        & $iscc (Join-Path $workspace 'installer.iss')
        if ($LASTEXITCODE -eq 0) {
            $installerPath = Join-Path $workspace 'dist\FastiumSetup.exe'
            if (Test-Path $installerPath) {
                $installerSize = (Get-Item $installerPath).Length
                Write-Host ('Instalator Windows: {0:N2} MB ({1} bajtow) -> {2}' -f ($installerSize / 1000000), $installerSize, $installerPath)
            }
        }
    }
    Write-Host "Zachowaj Cargo.lock w repozytorium, aby kolejne wydania uzywaly tych samych zaleznosci."
} finally { Pop-Location }
