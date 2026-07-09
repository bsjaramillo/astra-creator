# Instalador de astra-creator para Windows.
# Baja el .exe desde GitHub Releases, lo instala en %LOCALAPPDATA%\astra-creator
# y lo agrega al PATH del usuario.
#
#   irm https://raw.githubusercontent.com/bsjaramillo/astra-creator/main/install.ps1 | iex
#
# Variables de entorno opcionales:
#   ASTRA_CREATOR_REPO     owner/repo (default: bsjaramillo/astra-creator)
#   ASTRA_CREATOR_VERSION  tag a instalar (default: latest)

$ErrorActionPreference = "Stop"

$repo    = if ($env:ASTRA_CREATOR_REPO)    { $env:ASTRA_CREATOR_REPO }    else { "bsjaramillo/astra-creator" }
$version = if ($env:ASTRA_CREATOR_VERSION) { $env:ASTRA_CREATOR_VERSION } else { "latest" }

$asset = "astra-creator-x86_64-pc-windows-msvc.exe"
if ($version -eq "latest") {
    $url = "https://github.com/$repo/releases/latest/download/$asset"
} else {
    $url = "https://github.com/$repo/releases/download/$version/$asset"
}

$dir = Join-Path $env:LOCALAPPDATA "astra-creator"
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$dest = Join-Path $dir "astra-creator.exe"

Write-Host "Descargando $asset ($version)..."
Invoke-WebRequest -Uri $url -OutFile $dest
Write-Host "Instalado en $dest"

# Agregar al PATH del usuario si falta.
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$dir*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$dir", "User")
    Write-Host "Agregado $dir al PATH del usuario. Reiniciá la terminal para que tome efecto."
}

Write-Host "Ejecutá: astra-creator"
