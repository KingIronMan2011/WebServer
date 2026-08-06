[CmdletBinding()]
param(
    [string]$BinaryPath = (Join-Path $PSScriptRoot "..\..\target\release\webserver.exe")
)

$ErrorActionPreference = "Stop"
$serviceName = "Webserver"
$installRoot = Join-Path $env:ProgramFiles "Webserver"
$configRoot = Join-Path $env:ProgramData "Webserver"
$contentRoot = "C:\inetpub\wwwroot\Webserver"
$certificateRoot = Join-Path $configRoot "certificates"
$acmeCertificateRoot = Join-Path $certificateRoot "acme"
$localCertificateRoot = Join-Path $certificateRoot "local"
$dataRoot = Join-Path $configRoot "data"
$sitesRoot = Join-Path $configRoot "sites"

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Run this installer from an elevated PowerShell session."
}
if (-not (Test-Path -LiteralPath $BinaryPath -PathType Leaf)) {
    throw "Executable not found: $BinaryPath"
}

New-Item -ItemType Directory -Force -Path $installRoot, $configRoot, $sitesRoot, $dataRoot, $certificateRoot, $acmeCertificateRoot, $localCertificateRoot, $contentRoot, (Join-Path $contentRoot "public") | Out-Null
Copy-Item -LiteralPath $BinaryPath -Destination (Join-Path $installRoot "webserver.exe") -Force

if (-not (Test-Path -LiteralPath (Join-Path $configRoot "webserver.toml"))) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "webserver.toml") -Destination (Join-Path $configRoot "webserver.toml")
}
if (-not (Test-Path -LiteralPath (Join-Path $configRoot "sites\localhost.conf"))) {
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot "sites\localhost.conf") -Destination (Join-Path $configRoot "sites\localhost.conf")
}
if (-not (Test-Path -LiteralPath (Join-Path $contentRoot "public\index.html"))) {
    Set-Content -LiteralPath (Join-Path $contentRoot "public\index.html") -Value "<!doctype html><title>Webserver</title><h1>It works!</h1>" -NoNewline
}

$localService = "NT AUTHORITY\LOCAL SERVICE"
# ProgramData commonly inherits read access for local Users. Remove that
# inheritance before storing session hashes, ACME account data, or private keys.
& icacls $configRoot /inheritance:r /grant:r "SYSTEM:(OI)(CI)F" "BUILTIN\Administrators:(OI)(CI)F" "${localService}:(OI)(CI)RX" /T /C | Out-Null
& icacls $sitesRoot /grant "${localService}:(OI)(CI)M" /T /C | Out-Null
& icacls $dataRoot /grant "${localService}:(OI)(CI)M" /T /C | Out-Null
& icacls $acmeCertificateRoot /grant "${localService}:(OI)(CI)M" /T /C | Out-Null
& icacls $localCertificateRoot /grant "${localService}:(OI)(CI)RX" /T /C | Out-Null
& icacls $contentRoot /grant "${localService}:(OI)(CI)RX" /T /C | Out-Null

$binary = '"' + (Join-Path $installRoot "webserver.exe") + '" run --config "' + (Join-Path $configRoot "webserver.toml") + '"'
$existingService = Get-Service -Name $serviceName -ErrorAction SilentlyContinue
if ($existingService) {
    if ($existingService.Status -ne "Stopped") {
        Stop-Service -Name $serviceName -Force
        $existingService.WaitForStatus("Stopped", [TimeSpan]::FromSeconds(30))
    }
    & sc.exe config $serviceName binPath= $binary start= auto obj= $localService | Out-Null
} else {
    & sc.exe create $serviceName binPath= $binary start= auto obj= $localService DisplayName= "Webserver" | Out-Null
}

& sc.exe start $serviceName | Out-Null
Get-Service -Name $serviceName
